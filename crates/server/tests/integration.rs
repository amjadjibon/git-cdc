use std::time::Duration;

use git_cdc_core::protocol::*;
use git_cdc_core::store::DiskStore;
use git_cdc_server::{app, AppState, Backend};

async fn spawn_server(grace: Duration) -> (String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState {
        backend: Backend::Disk(DiskStore::new(dir.path().join("objects"))),
        token: "test-token".into(),
        grace,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app(state)).await.unwrap();
    });
    (base, dir)
}

fn client() -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("authorization", "Bearer test-token".parse().unwrap());
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap()
}

fn oid(data: &[u8]) -> String {
    format!("blake3:{}", blake3::hash(data).to_hex())
}

async fn batch(
    client: &reqwest::Client,
    base: &str,
    operation: Operation,
    objects: Vec<ObjectSpec>,
) -> BatchResponse {
    client
        .post(format!("{base}/objects/batch"))
        .json(&BatchRequest {
            operation,
            transfers: vec![TRANSFER_BASIC.into()],
            git_ref: None,
            objects,
            hash_algo: HASH_ALGO.into(),
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

#[tokio::test]
async fn auth_is_enforced() {
    let (base, _dir) = spawn_server(Duration::from_secs(3600)).await;
    let anon = reqwest::Client::new();
    let r = anon.get(format!("{base}/healthz")).send().await.unwrap();
    assert_eq!(r.status(), 401);
    let r = anon
        .get(format!("{base}/healthz"))
        .bearer_auth("wrong-token")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    let r = client().get(format!("{base}/healthz")).send().await.unwrap();
    assert_eq!(r.status(), 200);
}

#[tokio::test]
async fn batch_upload_download_round_trip() {
    let (base, _dir) = spawn_server(Duration::from_secs(3600)).await;
    let c = client();
    let data = b"chunk one content".to_vec();
    let spec = ObjectSpec {
        oid: oid(&data),
        size: data.len() as u64,
    };

    // Upload negotiation: server is empty, so it must offer an upload action.
    let resp = batch(&c, &base, Operation::Upload, vec![spec.clone()]).await;
    assert_eq!(resp.transfer, "basic");
    let action = resp.objects[0].actions.as_ref().unwrap().upload.as_ref().unwrap();

    let r = c
        .put(format!("{base}{}", action.href))
        .body(data.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);

    // Re-negotiate: present chunk gets no actions (client skips it).
    let resp = batch(&c, &base, Operation::Upload, vec![spec.clone()]).await;
    assert!(resp.objects[0].actions.is_none());
    assert!(resp.objects[0].error.is_none());

    // Download negotiation + fetch.
    let resp = batch(&c, &base, Operation::Download, vec![spec.clone()]).await;
    let href = &resp.objects[0].actions.as_ref().unwrap().download.as_ref().unwrap().href;
    let got = c.get(format!("{base}{href}")).send().await.unwrap();
    assert_eq!(got.status(), 200);
    assert_eq!(got.bytes().await.unwrap().as_ref(), &data[..]);

    // Download of a missing chunk: per-object 404 error.
    let missing = ObjectSpec {
        oid: oid(b"never uploaded"),
        size: 1,
    };
    let resp = batch(&c, &base, Operation::Download, vec![missing]).await;
    assert_eq!(resp.objects[0].error.as_ref().unwrap().code, 404);
}

#[tokio::test]
async fn upload_with_wrong_oid_is_rejected() {
    let (base, _dir) = spawn_server(Duration::from_secs(3600)).await;
    let c = client();
    let r = c
        .put(format!("{base}/chunks/{}", oid(b"claimed content")))
        .body(b"actual different content".to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);

    let r = c
        .get(format!("{base}/chunks/{}", oid(b"nope")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn wrong_hash_algo_is_rejected() {
    let (base, _dir) = spawn_server(Duration::from_secs(3600)).await;
    let r = client()
        .post(format!("{base}/objects/batch"))
        .json(&BatchRequest {
            operation: Operation::Upload,
            transfers: vec![],
            git_ref: None,
            objects: vec![],
            hash_algo: "sha256".into(),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn gc_deletes_orphans_past_grace_only() {
    // Zero grace so orphans are immediately eligible.
    let (base, _dir) = spawn_server(Duration::ZERO).await;
    let c = client();

    let live = b"live chunk".to_vec();
    let orphan = b"orphan chunk".to_vec();
    for data in [&live, &orphan] {
        let r = c
            .put(format!("{base}/chunks/{}", oid(data)))
            .body(data.clone())
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
    }

    // Dry run deletes nothing.
    let resp: GcResponse = c
        .post(format!("{base}/gc"))
        .json(&GcRequest {
            live_oids: vec![oid(&live)],
            dry_run: true,
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp.deleted, vec![oid(&orphan)]);
    let still = c.get(format!("{base}/chunks/{}", oid(&orphan))).send().await.unwrap();
    assert_eq!(still.status(), 200);

    // Real run deletes the orphan, keeps the live chunk.
    let resp: GcResponse = c
        .post(format!("{base}/gc"))
        .json(&GcRequest {
            live_oids: vec![oid(&live)],
            dry_run: false,
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp.deleted, vec![oid(&orphan)]);
    assert_eq!(resp.kept_live, 1);
    let gone = c.get(format!("{base}/chunks/{}", oid(&orphan))).send().await.unwrap();
    assert_eq!(gone.status(), 404);
    let kept = c.get(format!("{base}/chunks/{}", oid(&live))).send().await.unwrap();
    assert_eq!(kept.status(), 200);
}
