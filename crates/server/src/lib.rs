use std::sync::Arc;
use std::time::{Duration, SystemTime};

use axum::body::Bytes;
use axum::extract::{Path, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use git_cdc_core::manifest::parse_hash;
use git_cdc_core::protocol::*;
use git_cdc_core::store::{ChunkStore, DiskStore};

pub struct AppState {
    pub store: DiskStore,
    pub token: String,
    pub grace: Duration,
}

pub fn app(state: AppState) -> Router {
    let state = Arc::new(state);
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/objects/batch", post(batch))
        .route("/chunks/{oid}", put(put_chunk).get(get_chunk))
        .route("/gc", post(gc))
        .layer(middleware::from_fn_with_state(state.clone(), auth))
        .with_state(state)
}

async fn auth(State(state): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let expected = format!("Bearer {}", state.token);
    let ok = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        == Some(expected.as_str());
    if ok {
        next.run(req).await
    } else {
        (StatusCode::UNAUTHORIZED, "missing or invalid bearer token").into_response()
    }
}

async fn batch(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchRequest>,
) -> Response {
    if req.hash_algo != HASH_ALGO {
        return (
            StatusCode::BAD_REQUEST,
            format!("unsupported hash_algo {:?}, server speaks {HASH_ALGO:?}", req.hash_algo),
        )
            .into_response();
    }
    if !req.transfers.is_empty() && !req.transfers.iter().any(|t| t == TRANSFER_BASIC) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("no supported transfer; server speaks {TRANSFER_BASIC:?}"),
        )
            .into_response();
    }

    let objects = req
        .objects
        .iter()
        .map(|obj| {
            let mut result = ObjectResult {
                oid: obj.oid.clone(),
                size: obj.size,
                actions: None,
                error: None,
            };
            let Ok(hash) = parse_hash(&obj.oid) else {
                result.error = Some(ObjectError {
                    code: 422,
                    message: "invalid oid".into(),
                });
                return result;
            };
            let present = state.store.has(&hash);
            match req.operation {
                // Present chunks get no actions — client skips them (the dedup win).
                Operation::Upload if !present => {
                    result.actions = Some(Actions {
                        upload: Some(Action {
                            href: format!("/chunks/{}", obj.oid),
                        }),
                        download: None,
                    });
                }
                Operation::Download if present => {
                    result.actions = Some(Actions {
                        upload: None,
                        download: Some(Action {
                            href: format!("/chunks/{}", obj.oid),
                        }),
                    });
                }
                Operation::Download => {
                    result.error = Some(ObjectError {
                        code: 404,
                        message: "chunk not found".into(),
                    });
                }
                Operation::Upload => {}
            }
            result
        })
        .collect();

    Json(BatchResponse {
        transfer: TRANSFER_BASIC.into(),
        objects,
    })
    .into_response()
}

async fn put_chunk(
    State(state): State<Arc<AppState>>,
    Path(oid): Path<String>,
    body: Bytes,
) -> Response {
    let Ok(hash) = parse_hash(&oid) else {
        return (StatusCode::UNPROCESSABLE_ENTITY, "invalid oid").into_response();
    };
    if blake3::hash(&body) != hash {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            "body does not hash to the given oid",
        )
            .into_response();
    }
    match state.store.put(&hash, &body) {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn get_chunk(State(state): State<Arc<AppState>>, Path(oid): Path<String>) -> Response {
    let Ok(hash) = parse_hash(&oid) else {
        return (StatusCode::UNPROCESSABLE_ENTITY, "invalid oid").into_response();
    };
    if !state.store.has(&hash) {
        return (StatusCode::NOT_FOUND, "chunk not found").into_response();
    }
    match state.store.get(&hash) {
        Ok(data) => data.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Client-driven mark-and-sweep (PLAN 5.3): client sends the complete live
/// set; unreferenced chunks older than the grace period are deleted. The
/// grace period protects racing in-flight uploads.
async fn gc(State(state): State<Arc<AppState>>, Json(req): Json<GcRequest>) -> Response {
    let mut live = std::collections::HashSet::new();
    for oid in &req.live_oids {
        match parse_hash(oid) {
            Ok(h) => {
                live.insert(h);
            }
            Err(e) => {
                return (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()).into_response();
            }
        }
    }

    let all = match state.store.list() {
        Ok(all) => all,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    let mut resp = GcResponse {
        deleted: Vec::new(),
        kept_live: 0,
        kept_grace: 0,
    };
    let now = SystemTime::now();
    for hash in all {
        if live.contains(&hash) {
            resp.kept_live += 1;
            continue;
        }
        // ponytail: mtime-based grace assumes store host clock sanity —
        // fine for MVP, revisit if stores ever span hosts.
        let age = std::fs::metadata(state.store.path_for(&hash))
            .and_then(|m| m.modified())
            .ok()
            .and_then(|mtime| now.duration_since(mtime).ok());
        match age {
            Some(age) if age >= state.grace => {
                if !req.dry_run {
                    if let Err(e) = state.store.remove(&hash) {
                        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
                            .into_response();
                    }
                }
                resp.deleted.push(format!("blake3:{}", hash.to_hex()));
            }
            _ => resp.kept_grace += 1,
        }
    }
    Json(resp).into_response()
}
