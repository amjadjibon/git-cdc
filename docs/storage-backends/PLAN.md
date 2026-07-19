---
goal: OpenDAL chunk store — Azure Blob, GCS, SFTP, FTP, Google Drive, WebDAV, OneDrive
version: 1.0
date_created: 2026-07-19
last_updated: 2026-07-19
owner: amjadjibon
status: 'Planned'
tags: [feature]
---

# OpenDAL Storage Backends

![Status: Planned](https://img.shields.io/badge/status-Planned-blue)

Adds a third server backend, `opendal`, wrapping `opendal::Operator` so one store
implementation serves Azure Blob, GCS, SFTP, FTP(S), Google Drive, WebDAV
(Nextcloud), and OneDrive. Follows docs/storage-backends/RESEARCH.md: one
dependency, one ~150-line store mirroring `S3Store`, wired as a new `Backend`
enum variant.

## 1. Requirements & Constraints

- **REQ-001**: `git-cdc-server --backend opendal --opendal-scheme <scheme> --opendal-option key=value ...` serves chunks from any of the seven services.
- **REQ-002**: `OpendalStore` enforces the same guards as `S3Store`: `put` verifies `blake3(data) == hash`; `put_encoded` decodes the envelope before admitting (upload-poisoning guard).
- **REQ-003**: `list()` returns `(hash, Option<SystemTime>)` from entry `last_modified` so GC grace periods keep working; `None` is acceptable where a service omits it.
- **REQ-004**: Foreign objects under the prefix (non-hex keys) are skipped by `list()`, same as `S3Store`.
- **SEC-001**: Credentials arrive via `--opendal-option` values or service env vars only; never logged. FTP without TLS and access-token-only Drive/OneDrive are documented as discouraged, not blocked.
- **CON-001**: opendal is pre-1.0 — pin `0.57` and enable only the seven service features plus `services-fs` (tests). No other new dependencies.
- **CON-002**: `services-sftp` is unix-only upstream; the project already targets unix (Containerfile, dev on darwin) — document, don't cfg-gate.
- **CON-003**: Serverless CLI mode stays S3-only this iteration (RESEARCH.md open question resolved: server-only first).

## 2. Implementation Steps

> After each phase: `git add -u` and commit. No `Co-authored-by:`. Tick `[x]` as each task completes.

### Phase 1: OpendalStore + server wiring + docs

**Goal**: The whole feature — store, backend variant, CLI flags, tests, README.

- [ ] TASK-001: Add `opendal = { version = "0.57", default-features = false, features = ["services-azblob", "services-gcs", "services-sftp", "services-ftp", "services-gdrive", "services-webdav", "services-onedrive", "services-fs"] }` to `crates/core/Cargo.toml`.
- [ ] TASK-002: Create `crates/core/src/store/opendal.rs` with `OpendalConfig { scheme: String, options: Vec<(String, String)>, prefix: String }` and `OpendalStore` mirroring `S3Store`'s API (`connect`, `has`, `put`, `put_encoded`, `get`, `get_encoded`, `remove`, `list`), built via `opendal::Operator::via_iter(scheme, options)`. Keys are `{prefix}{hex}`; `list` maps entry `last_modified()` to `SystemTime` and skips non-hash keys. Export from `crates/core/src/store/mod.rs`.
- [ ] TASK-003: Add `Backend::Opendal(OpendalStore)` arm to `crates/server/src/backend.rs` for all five methods.
- [ ] TASK-004: Wire `--backend opendal` in `crates/server/src/main.rs`: `--opendal-scheme` (env `GIT_CDC_OPENDAL_SCHEME`, required for the backend), repeatable `--opendal-option KEY=VALUE`, `--opendal-prefix` (default `chunks/`); runtime bail when scheme is missing, matching the disk/s3 checks. Extend the existing arg-validation unit tests.
- [ ] TASK-005: Add `crates/server/tests/opendal_backend.rs` running `OpendalStore` against the `fs` scheme in a tempdir: put/has/get round trip, hash-mismatch rejection, `put_encoded` envelope verification, `remove`, `list` returns the hash with a mtime and skips a planted foreign key.
- [ ] TASK-006: README: document the `opendal` backend with example invocations for azblob, gcs, webdav/Nextcloud, sftp; note SFTP unix/key-only and Drive/OneDrive OAuth-refresh setup; add the `rclone serve s3` zero-code alternative.

**Completion criteria**: `cargo test --workspace` passes including the new `opendal_backend` integration test; `cargo run -p git-cdc-server -- --backend opendal --token t` fails with a clear "scheme required" error and succeeds with `--opendal-scheme fs --opendal-option root=/tmp/x`.

**git commit**: `git add -u && git commit -m "feat: opendal storage backend (azblob, gcs, sftp, ftp, gdrive, webdav, onedrive)"`

**Agent Prompt**:
```
You are a sub-agent implementing Phase 1 of storage-backends.

Context: git-cdc stores content-addressed chunks (blake3 hex keys, zstd envelope
encoding) behind a Backend enum (disk, s3). Add a third backend via Apache
OpenDAL covering azblob/gcs/sftp/ftp/gdrive/webdav/onedrive.
Branch: storage-backends  |  Base: main

Tasks: TASK-001..TASK-006 as listed in docs/storage-backends/PLAN.md §2.

Key files:
- crates/core/src/store/s3.rs — mirror this API exactly in a new opendal.rs
- crates/core/src/store/mod.rs — export the new module
- crates/core/src/store/envelope.rs — encode/decode used by put/put_encoded
- crates/server/src/backend.rs — add the enum arm
- crates/server/src/main.rs — flags + runtime validation + arg unit tests
- crates/server/tests/s3_backend.rs — test style to follow (fs scheme, no fixture needed)

Completion criteria: cargo test --workspace passes including the new
opendal_backend integration test; --backend opendal without a scheme bails
with a clear error.

When done: git add -u (explicit paths for new files) && git commit -m "feat:
opendal storage backend (azblob, gcs, sftp, ftp, gdrive, webdav, onedrive)"
— no Co-authored-by. Reply with a one-paragraph summary and commit SHA.
Do NOT push, open PRs, or modify PLAN.md.
```

## 3. Testing

- [ ] TEST-001: `crates/server/tests/opendal_backend.rs` — fs-scheme round trip, poisoning guards, list mtime + foreign-key skip (TASK-005).
- [ ] TEST-002: `crates/server/src/main.rs` unit tests — `--backend opendal` requires `--opendal-scheme`; option parsing splits `KEY=VALUE`.

## 4. Risks & Assumptions

- **RISK-001**: opendal 0.5x API churn — mitigation: pinned minor version; the store surface touched is tiny (via_iter, read, write, delete, exists, lister).
- **RISK-002**: Seven service features bloat compile time — mitigation: `default-features = false`; drop features later if build time hurts.
- **ASSUMPTION-001** (from RESEARCH.md): lister entries carry `last_modified` for azblob/gcs/webdav; verified for `fs` by TEST-001, cloud services tolerated as `None` by the GC contract.
- **ASSUMPTION-002** (from RESEARCH.md): Drive/OneDrive quotas make them "works, not recommended" — documented as such, unverified.
