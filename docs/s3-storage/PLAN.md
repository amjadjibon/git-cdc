---
status: Implemented
version: 1.1
date: 2026-07-17
feature: s3-storage
design: docs/git-cdc-mvp/DESIGN.md (§8, §13.5 — deferred from MVP)
---

# Plan: S3 Chunk Storage (server backend + serverless mode)

Two consumers of one S3 store implementation (user decision: "both"):

1. **Server backend** — `git-cdc-server --backend s3` stores chunks in a
   bucket instead of local disk. Central auth/policy point stays.
2. **Serverless mode** — the CLI talks straight to the bucket
   (`git config cdc.s3.bucket …`), no server process at all; IAM credentials
   replace the bearer token, like restic/DVC.

## Assumptions

- `aws-sdk-s3` with the standard credential chain (env vars, profiles, IMDS);
  MinIO/R2 via endpoint override + force-path-style.
- `S3Store` lives in `git-cdc-core` (one impl, two consumers). Core gains
  the AWS SDK + a small tokio runtime for the CLI's blocking wrapper —
  acceptable; the alternative is a copy in each crate.
- The sync `ChunkStore` trait stays as-is for the local store. The server
  gets a `Backend` **enum** (disk | s3), the CLI a `Remote` **enum**
  (http | s3) — two variants, one call site each; no trait objects.
- S3 keys are flat chunk hex under an optional prefix; directory sharding
  is a filesystem concern S3 doesn't have.
- Serverless upload negotiation uses one paginated `ListObjectsV2` into a
  set (1 request/1000 chunks) rather than per-chunk `HeadObject`.
- Serverless mode has no server-side upload verification — acceptable: a
  client with bucket write access can write anything regardless; the read
  path (chunk re-hash on get + whole-file oid check) still catches
  corruption before it ever reaches a worktree.
- GC needs per-chunk age: `list()` returns `(hash, modified)`; disk uses
  file mtime, S3 uses `LastModified`. In serverless mode the CLI's
  `--grace-secs` applies to the bucket sweep (there is no server to own it).

## Phase 1: Shared S3Store + server Backend enum

Depends on: nothing.

- [x] **1.1** `git-cdc-core::s3`: `S3Store` — `head_object` (has),
  `put_object` (put, after hash verify), `get_object` (get, re-verify on
  read), paginated `list_objects_v2` (list with `LastModified`),
  `delete_object` (remove); plus a shared client builder (endpoint override,
  force-path-style, region fallback). Deps: `aws-sdk-s3`, `aws-config`,
  `tokio` (rt).
  *Done when*: `cargo build -p git-cdc-core` passes.
- [x] **1.2** Server `backend` module: `enum Backend { Disk(DiskStore),
  S3(S3Store) }` with async `has/put/get/remove/list`; GC's mtime logic
  moves behind `list()`. Rewire `AppState`/handlers.
  *Done when*: existing integration suite passes unchanged on the disk path.
- [x] **1.3** Server flags: `--backend disk|s3` (default disk; `--root`
  required for disk, `--s3-bucket` required for s3 — enforced at startup,
  not first request), `--s3-prefix`, `--s3-endpoint`,
  `--s3-force-path-style`.
  *Done when*: `--backend s3` without `--s3-bucket` errors at startup.

## Phase 2: Serverless CLI mode

Depends on: Phase 1.

- [x] **2.1** CLI `Remote` enum: `Http` (existing batch client) | `S3`
  (`S3Store` + owned tokio runtime, `block_on` per op). Selection: if
  `cdc.s3.bucket` is set → S3 (with `cdc.s3.prefix`, `cdc.s3.endpoint`,
  `cdc.s3.force-path-style`); else `cdc.url` → HTTP; neither → error
  naming both options.
  *Done when*: `push`/`pull`/`gc` compile against `Remote` and the HTTP
  path behaves exactly as before.
- [x] **2.2** S3 paths: push = bucket `list()` → set-diff → upload missing;
  pull = `get` missing chunks; gc = `list()` + age-filtered `remove` of
  non-live chunks, honoring `--dry-run`/`--grace-secs`.
  *Done when*: exercised by the Phase 3 gated test / MinIO smoke.

## Phase 3: Tests

Depends on: Phase 2.

- [x] **3.1** Existing suites keep passing on disk/HTTP paths (regression
  gate for both refactors).
  *Done when*: `cargo test --workspace` green.
- [x] **3.2** Env-gated S3 test (`GIT_CDC_TEST_S3_ENDPOINT` + AWS env
  creds): server-backend integration (batch/upload/download/gc against
  the bucket) and a serverless CLI e2e (track→commit→push→clone→pull→gc,
  no server). Skips with a notice when the env is absent; run against
  MinIO in smoke testing if docker/minio is available.
  *Done when*: `cargo test --workspace` passes without the env; both S3
  suites pass against MinIO when available.

## Phase 4: Docs

Depends on: Phase 3.

- [x] **4.1** README: serverless quick-start (bucket config, cred chain),
  server `--backend s3` flags, MinIO example; adjust "out of scope" list.
  *Done when*: README reflects reality.

## Verification

```sh
cargo build --workspace
cargo test --workspace                      # disk path, always
GIT_CDC_TEST_S3_ENDPOINT=... cargo test -p git-cdc-server  # S3 path, when available
```

## Risks

- `aws-sdk-s3` is a heavy dependency tree — now in core too (serverless mode
  needs it in the CLI); accepted in Assumptions when scope became "both".
- S3 `LastModified` clock vs. server clock for GC grace — same accepted
  MVP caveat as disk mtime.
- No S3 endpoint in the default test environment — mitigated by the
  env-gated test + disk-path regression suite; S3 path additionally
  verified via MinIO smoke test when possible.
