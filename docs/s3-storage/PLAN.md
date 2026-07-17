---
status: Planned
version: 1.0
date: 2026-07-17
feature: s3-storage
design: docs/git-cdc-mvp/DESIGN.md (§8, §13.5 — deferred from MVP)
---

# Plan: S3 Chunk Storage Backend

Give `git-cdc-server` an S3-compatible backend (AWS S3, MinIO, R2) selectable
at startup, alongside the existing disk backend. Server-side only — the CLI
keeps its local disk store and speaks the same batch API regardless of what
the server stores chunks in.

## Assumptions

- The server still proxies chunk bytes (basic transfer). Pre-signed URLs /
  CDN offload stay v2 (DESIGN §13.5 → transfer adapters, §15.3).
- `aws-sdk-s3` with the standard credential chain (env vars, profiles, IMDS);
  MinIO/R2 via `--s3-endpoint` + force-path-style.
- The sync `ChunkStore` trait in `git-cdc-core` stays as-is for the CLI's
  local store. The server gets a small async `Backend` **enum** (disk | s3) —
  two variants with one call site each; a trait object + `async_trait` would
  be abstraction for a single use case.
- S3 object keys are flat chunk hex (with optional `--s3-prefix`); directory
  sharding is a filesystem concern S3 doesn't have.
- GC needs per-chunk age: `Backend::list()` returns `(hash, modified)`;
  disk uses file mtime (as today), S3 uses `LastModified`.

## Phase 1: Async Backend enum + S3Store

Crate `crates/server`. Depends on: nothing.

- [ ] **1.1** Add deps: `aws-sdk-s3`, `aws-config`. New `backend` module:
  `enum Backend { Disk(DiskStore), S3(S3Store) }` with
  `async fn has/put/get/remove` and `async fn list() -> Vec<(Hash, Option<SystemTime>)>`.
  Disk variant delegates to the existing sync store; `put` keeps the
  verify-hash-before-admit guard in both variants.
  *Done when*: `cargo build -p git-cdc-server` passes with handlers still on disk.
- [ ] **1.2** `S3Store`: `head_object` (has), `put_object` (put, after hash
  verify), `get_object` (get, re-verify hash on read like DiskStore),
  `list_objects_v2` paginated (list, parsing hex keys, `LastModified` → age),
  `delete_object` (remove). Config: bucket, optional key prefix, optional
  endpoint URL + force-path-style for MinIO.
  *Done when*: compiles; exercised by Phase 3 tests.
- [ ] **1.3** Rewire `AppState`/handlers and GC onto `Backend` (GC's mtime
  logic moves behind `list()`); CLI flags: `--backend disk|s3` (default
  disk), `--s3-bucket`, `--s3-prefix`, `--s3-endpoint`,
  `--s3-force-path-style`, region/creds from the AWS default chain.
  *Done when*: existing integration suite passes unchanged on the disk path.

## Phase 2: Tests

Depends on: Phase 1.

- [ ] **2.1** Existing server integration tests keep running against the disk
  backend (regression gate for the refactor).
  *Done when*: `cargo test --workspace` green.
- [ ] **2.2** S3 integration test, env-gated (no S3 in the default test env):
  `GIT_CDC_TEST_S3_ENDPOINT` + standard AWS env creds → runs the full
  batch/upload/download/gc suite against a real endpoint (MinIO); otherwise
  the test skips with a notice.
  *Done when*: `cargo test -p git-cdc-server` passes without the env; passes
  against MinIO when available (verified in smoke testing if docker/minio
  exists locally).

## Phase 3: Docs

Depends on: Phase 2.

- [ ] **3.1** README: S3 quick-start (server flags, MinIO example, cred
  chain note); adjust "out of scope" list.
  *Done when*: README reflects reality.

## Verification

```sh
cargo build --workspace
cargo test --workspace                      # disk path, always
GIT_CDC_TEST_S3_ENDPOINT=... cargo test -p git-cdc-server  # S3 path, when available
```

## Risks

- `aws-sdk-s3` is a heavy dependency tree — server-crate only; the CLI/core
  crate stays lean.
- S3 `LastModified` clock vs. server clock for GC grace — same accepted
  MVP caveat as disk mtime.
- No S3 endpoint in the default test environment — mitigated by the
  env-gated test + disk-path regression suite; S3 path additionally
  verified via MinIO smoke test when possible.
