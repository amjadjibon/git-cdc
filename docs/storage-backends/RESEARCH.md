---
date: 2026-07-19
feature: storage-backends
status: Concluded
recommendation: One OpenDAL-backed store variant covers all seven services; document the rclone-gateway zero-code path too
---

# Research: storage-backends

## Question

1. How should git-cdc support Azure Blob, GCS, SFTP, FTP, Google Drive, Nextcloud (WebDAV), and OneDrive as chunk storage backends?
2. Do any of these need bespoke code, or does one abstraction cover them?

## Recommendation

**Add a single `OpendalStore` wrapping `opendal::Operator` (crate `opendal` 0.57.0), as a third `Backend` variant.** OpenDAL ships production backends for all seven services (`azblob`, `gcs`, `sftp`, `ftp`, `gdrive`, `webdav`, `onedrive`) behind one async API that maps 1:1 onto our contract (`has/put/get/remove/list`). One dependency, one ~150-line store, seven backends — versus seven bespoke implementations. Main risk: OpenDAL is pre-1.0 (API churn between minor versions) and each service is a cargo feature pulling its own dep tree — enable only the features we ship.

Also document the **zero-code path** in the README first: `rclone serve s3` fronts every one of these services with an S3 API, and the existing `S3Store` (`crates/core/src/store/s3.rs`) works against it today (GCS additionally has a native S3-interop XML endpoint). Users who just want Nextcloud/Drive today don't need to wait for the feature.

## Candidates

### OpenDAL single store — recommended
- **Fit**: `Operator::{exists, write, read, delete, lister}` maps directly onto the existing async `S3Store` shape; `Backend` enum in `crates/server/src/backend.rs` was explicitly left ready for "a third backend" (its own ponytail comment says add abstraction when one appears — it has). Config = scheme + key/value map, same flag/git-config plumbing as `S3Config`.
- **Cost**: one dependency (per-service feature flags; each pulls its service's client deps). No migration — additive backend.
- **Verified**: docs.rs opendal 0.57.0 (2026-06-01) — all seven services present with read/write/delete/stat/list capabilities (FIND-001..003).

### Do nothing / rclone gateway — partially adopted (document it)
- **Fit**: existing `S3Store` unchanged; `rclone serve s3 remote:` bridges all seven services. **Cost**: zero code; user runs a sidecar process. Rejected as the *only* answer because native support was requested and a sidecar is real ops burden — but it's the right README note and the fallback for any service OpenDAL handles poorly.

### Per-service SDKs (azure_storage_blobs, google-cloud-storage, suppaftp, reqwest_dav, Graph/Drive REST by hand) — rejected
- **Why rejected**: 6+ new dependencies and seven store implementations to maintain, for the same contract OpenDAL gives in one. Drive/OneDrive would mean hand-rolling OAuth refresh.

## Findings

- **FIND-001**: The full store contract is 5 ops — `has`, `put_encoded`, `get_encoded`, `remove`, `list` with optional last-modified (`crates/server/src/backend.rs`, `crates/core/src/store/mod.rs`). Envelope encode/verify is layered above the store, so any byte store works unchanged.
- **FIND-002**: opendal 0.57.0 ships `Azblob`, `Gcs`, `Sftp`, `Ftp` (incl. FTPS), `Gdrive`, `Webdav`, `Onedrive` services, each supporting stat/read/write/delete/list (docs.rs, 2026-06).
- **FIND-003**: OpenDAL SFTP: SSH key auth only (no passwords, by design) and **unix only** — Windows server builds can't enable it. FTP has no such limits but is plaintext unless FTPS.
- **FIND-004**: Gdrive/OneDrive auth: pass `refresh_token` + `client_id` + `client_secret` and OpenDAL auto-refreshes access tokens; access-token-only mode dies after ~1h. Google Drive permits duplicate names per parent — OpenDAL resolves to most-recently-modified, so hash-keyed flat objects are safe but not transactional.
- **FIND-005**: Nextcloud is plain WebDAV (`https://host/remote.php/dav/files/<user>/` + basic auth or app password) — the `webdav` service covers it; no Nextcloud-specific code needed.
- **FIND-006**: Server already has async plumbing for remote stores (`Backend::S3` arm) — a `Backend::Opendal` arm is mechanical.

## Assumptions

- **ASSUMPTION-001**: OpenDAL's lister returns `last_modified` for azblob/gcs/webdav entries without a per-entry `stat` (GC grace period wants it; contract tolerates `None`, and per-entry stat is an acceptable fallback for slow backends). Verify with a 10-line spike against MinIO-style local webdav/azurite during planning.
- **ASSUMPTION-002**: Drive/OneDrive throughput and API quotas are tolerable for chunk workloads (many small objects). Likely poor — treat them as "works, not recommended" tiers in docs. Verify only if a user actually cares.

## Open Questions

- Should serverless CLI mode (currently `S3Store` direct) also grow OpenDAL support, or server-only first? Recommend server-only first; CLI can follow the same store.
