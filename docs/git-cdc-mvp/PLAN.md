---
status: Implemented
version: 1.1
date: 2026-07-17
feature: git-cdc-mvp
design: docs/git-cdc-mvp/DESIGN.md
review: docs/git-cdc-mvp/PLAN-REVIEW.md
---

# Plan: git-cdc MVP

Phased implementation plan derived from [DESIGN.md](DESIGN.md), scoped per
[PLAN-REVIEW.md](PLAN-REVIEW.md).

## MVP Boundary

**In scope**: FastCDC chunker, manifest (DESIGN §15.1 format, single normative
spec), local-disk CAS, axum batch server (basic transfer only, static bearer
token), `git-cdc` CLI with clean/smudge filter, client-driven GC, end-to-end
round-trip test.

**Out of scope (v2+)**: S3/Postgres backends, refcounting, filter-process
protocol, transfer adapters, SSH transport, CDN pre-signed URLs, delta patch
precomputation, file tagging/selective sync, build labels, bloom-filter
reservoir, mmap/rayon performance work, per-chunk zstd compression.
DESIGN §13, §15.3–15.4, §16 are all deferred.

**Decisions baked in from review**:
- `ChunkStore` trait is `has`/`put`/`get` only — no refcount methods
  (REVISE-003). GC is client-driven mark-and-sweep (REVISE-001).
- Manifest format is DESIGN §15.1; whole-file `oid` computed in the same
  pass as chunking via a running `blake3::Hasher` (REVISE-004).
- Server verifies the BLAKE3 hash of every uploaded chunk before admitting
  it to the CAS; auth is a static bearer token (SUGGEST-001).
- Local disk storage only (SUGGEST-002).

## Crate Layout

Two crates, not six — MVP has one consumer per boundary:

```
crates/
├── core/             # package git-cdc-core — lib: chunker, manifest, store, batch client
│   └── src/bin/git-cdc.rs    # CLI: install, track, clean, smudge, pull, push, gc, diff
└── server/           # package git-cdc-server — axum bin: batch API + chunk PUT/GET
```

(Directory names `core`/`server`; package names keep the `git-cdc-` prefix
since Cargo rejects `core` as a package name — it collides with Rust's
built-in `core` crate. The CLI binary stays `git-cdc` for the `git cdc`
extension mechanism.)

Chunking params (DESIGN §4): min 512 KiB / avg 2 MiB / max 8 MiB, constants
in `git-cdc` for now; config plumbing is v2.

---

## Phase 1: Chunker + Manifest

Crate `crates/core` (lib only). Depends on: nothing.

- [x] **1.1** Workspace member `crates/core` (package `git-cdc-core`) with deps: `fastcdc`,
  `blake3`, `anyhow`, `thiserror`. Types: `Chunk { hash, offset, length }`.
  *Done when*: `cargo build -p git-cdc-core` passes.
- [x] **1.2** `chunker::chunk_stream(reader) -> anyhow::Result<(Vec<Chunk>, FileOid, u64)>`
  using `fastcdc::v2020::StreamCDC` so multi-GB files never load fully into
  memory; feed a running whole-file `blake3::Hasher` in the same pass.
  *Done when*: chunking a generated 100 MiB file yields chunks whose sizes
  respect min/max bounds and whose concatenated lengths equal file size.
- [x] **1.3** `manifest` module: encode/parse the §15.1 format —
  `version` line first, remaining header keys sorted, `{key} {value}\n`
  LF-only UTF-8, keys `[a-z0-9.-]`, unknown header keys preserved on
  rewrite; chunk lines follow the header.
  *Done when*: `parse(encode(m)) == m` and encoding is byte-stable.
- [x] **1.4** Tests: round-trip property test (random content →
  chunk → manifest → parse → reassemble == original bytes); edge cases:
  empty file, file < min chunk size, file exactly at max chunk size,
  manifest with unknown keys survives parse→encode; reject non-manifest
  input (passthrough detection for `smudge` later).
  *Done when*: `cargo test -p git-cdc-core` passes.

## Phase 2: Disk Chunk Store

Depends on: Phase 1 (hash type).

- [x] **2.1** `store::ChunkStore` trait — `has`, `put`, `get` only.
  `put` verifies `blake3(data) == hash` and errors on mismatch (this is
  the server's upload-poisoning guard too).
  *Done when*: trait compiles with `DiskStore` as sole impl.
- [x] **2.2** `DiskStore`: sharded layout `<root>/<hex[0..2]>/<hex[2..4]>/<hex>`,
  write via temp file + atomic rename so a killed process never leaves a
  half-written chunk that `has()` reports present.
  *Done when*: put/get/has round-trip; `put` of corrupted data errors;
  concurrent double-`put` of the same chunk is safe.
- [x] **2.3** Tests for 2.1–2.2 including the corruption-rejection case.
  *Done when*: `cargo test -p git-cdc-core store` passes.

## Phase 3: Batch Server

Crate `crates/server` (package `git-cdc-server`). Depends on: Phase 2.

- [x] **3.1** Workspace member `crates/server` with `axum`, `tokio`,
  `serde`/`serde_json`, depending on `git-cdc-core` for store + types. Config via
  env/flags: `--root <dir>`, `--token <bearer>`, `--listen <addr>`.
  *Done when*: server starts and serves a health endpoint.
- [x] **3.2** `POST /objects/batch`: LFS-shaped request with `operation`,
  `objects`, `hash_algo` (`"blake3"`, reject others), optional `ref`
  (accepted and logged; ACLs are v2). Response contains actions only for
  chunks the store is missing (upload) or has (download), `basic` transfer
  only.
  *Done when*: batch request against a seeded store omits present chunks.
- [x] **3.3** `PUT /chunks/{oid}` and `GET /chunks/{oid}` — basic transfer
  endpoints; PUT re-hashes body before store admit (via 2.1's `put`).
  *Done when*: uploading a chunk with a wrong oid returns 422; GET of a
  missing chunk returns 404.
- [x] **3.4** Static bearer-token auth middleware on all routes.
  *Done when*: requests without/with-wrong token get 401.
- [x] **3.5** Integration test: spin server on an ephemeral port, drive
  batch → upload → batch → download with `reqwest`.
  *Done when*: `cargo test -p git-cdc-server` passes.

## Phase 4: CLI + Filter Driver

Depends on: Phase 1 (Phase 3 not needed — clean/smudge work against the
local store; network sync is Phase 5).

- [x] **4.1** `src/bin/git-cdc.rs` with `clap`: subcommands `install`,
  `track`, `clean`, `smudge`, `pull`, `push`, `gc`, `diff` (pull/push/gc
  wired in Phase 5). Repo discovery via `git rev-parse --show-toplevel`
  (DESIGN §14.3).
  *Done when*: `git cdc --help` works when the binary is on `$PATH`.
- [x] **4.2** `install [--global]`: set `filter.cdc.clean` / `filter.cdc.smudge`
  via `git config` (do **not** set `filter.cdc.required` — see 4.3's
  passthrough behavior); repo-local install also writes a pre-push hook
  running `git cdc push` so manifests can't be published before their
  chunks (the classic LFS footgun). `track <pattern>...`: append
  `<pattern> filter=cdc -text` lines to `.gitattributes` (idempotent).
  *Done when*: running both in a scratch repo produces working config +
  hook + attributes; re-running changes nothing; an existing pre-push hook
  is not clobbered (append/chain or warn).
- [x] **4.3** `clean`: stdin → chunk (1.2) → write chunks to the local store
  (`.git/cdc/objects` via 2.2) → manifest to stdout. `smudge`: stdin
  manifest → read chunks from local store → original bytes to stdout;
  non-manifest input passes through unchanged (same safety behavior as
  git-lfs). Missing chunks: smudge writes the **manifest text through to
  the worktree** and prints a "run `git cdc pull`" hint on stderr — the
  git-lfs-proven answer to the fresh-clone case, where the local store is
  empty and a hard error would make `git clone` itself fail before `pull`
  could ever run. Corrupt/truncated chunk content (hash mismatch on read)
  is still a hard error naming the oid — never emit wrong bytes.
  *Done when*: `smudge(clean(x)) == x` unit test passes for the Phase 1
  edge-case corpus, and smudge-with-empty-store yields the manifest text
  plus a nonzero-signal hint (not an error).
  <!-- ponytail: single-shot clean/smudge; add filter-process protocol (DESIGN §10) when checkout latency on many-file repos measurably hurts -->
- [x] **4.4** End-to-end filter test (shell or `assert_cmd`): scratch git
  repo, `git cdc install && git cdc track '*.bin'`, add a binary, commit,
  delete worktree file, `git checkout -- .`, assert byte-identical restore
  and that the committed blob is a manifest.
  *Done when*: test passes via `cargo test -p git-cdc-core --test e2e_filter`.

## Phase 5: Sync + GC + Full E2E

Depends on: Phases 3, 4.

- [x] **5.1** Batch client in `git-cdc-core` lib (`reqwest`, one shared `Client`):
  negotiate against `POST /objects/batch`, then upload/download via basic
  transfer. Server URL + token from `git config` keys `cdc.url` / `cdc.token`.
  *Done when*: client round-trips chunks against a test server.
- [x] **5.2** `push`: enumerate every manifest blob across history via
  `git rev-list --all --objects` piped to `git cat-file --batch`,
  identifying manifests by their fixed first line
  (`version git-cdc/spec/v1`) — not by path/attribute matching,
  which misses renamed or historical files; batch-negotiate, upload only
  missing chunks. `pull`: read manifests from the **index** (`git ls-files`
  + attribute check), not the worktree — after a fresh clone the worktree
  holds passed-through manifest text (4.3); fetch chunks absent from the
  local store, then re-smudge (`git checkout -- <paths>` or direct write)
  to materialize real content.
  *Done when*: push→wipe local store→pull restores bytes, including from a
  worktree in the passed-through-manifest state.
- [x] **5.3** `gc [--dry-run]` — client-driven mark-and-sweep per review
  REVISE-001: enumerate all reachable manifests locally, send the live-hash
  set to the server (`POST /gc`, auth'd), server deletes unreferenced chunks
  older than a grace period (default 24h, from file mtime); same sweep runs
  locally against `.git/cdc/objects`.
  *Done when*: seeded orphan chunks are removed, live + fresh chunks survive,
  `--dry-run` deletes nothing.
- [x] **5.4** Full e2e test: scratch repo + live server; commit v1 of a
  20 MiB file, edit 1 KiB in the middle, commit v2, `git cdc push` — assert
  the second push uploads only the few changed chunks (the dedup win).
  Then a **real fresh clone**: `git clone` succeeds (smudge passthrough,
  no hard error), worktree holds manifest text, `git cdc pull` materializes
  both versions byte-identically. Also assert the pre-push hook (4.2) blocks
  a `git push` whose chunks haven't been uploaded.
  *Done when*: `cargo test --workspace` passes end to end.

---

## Verification

Every phase gates on its listed `cargo test` command; the whole plan gates on:

```sh
cargo build --workspace
cargo test --workspace
```

The load-bearing invariant, tested at three levels (unit 4.3, filter e2e 4.4,
network e2e 5.4): **`smudge(clean(bytes)) == bytes`, and a missing chunk is a
hard error, never silent corruption.**

## Risks

- `fastcdc` crate's streaming API surface may differ from the sketch —
  verify `StreamCDC` in 1.2 before building on it; fallback is windowed
  reads with overlap (DESIGN §4).
- Git filter behavior differs subtly across versions (empty-file handling,
  `-text` interaction) — the 4.4 e2e test is the guard.
- GC grace period via mtime assumes store host clock sanity — acceptable
  for MVP, noted in `gc --help`.
