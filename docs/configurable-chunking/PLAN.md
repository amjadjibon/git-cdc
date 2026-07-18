---
status: Ready
version: 1.0
date: 2026-07-18
feature: configurable-chunking
research: docs/configurable-chunking/RESEARCH.md
---

# Plan: configurable chunking sizes via gitconfig

`cdc.chunk.min` / `cdc.chunk.avg` / `cdc.chunk.max` (bytes; git's `k`/`m`/`g`
suffixes work) control FastCDC bounds at clean time. Unset keys keep the
current defaults (512 KiB / 2 MiB / 8 MiB). Manifests are self-describing, so
existing data is unaffected (RESEARCH FIND-004).

## Assumptions

- Validation lives in git-cdc because fastcdc only `debug_assert`s its bounds
  (FIND-002): each value within fastcdc's hard range, and `min ≤ avg ≤ max`.
- Config is read once per `clean` invocation via `git config --type=int`
  (FIND-003) — no caching concerns, filters are one-shot processes.
- The server raises its body limit to the 16 MiB protocol ceiling + slack
  instead of gaining a flag (FIND-005).
- Cross-client manifest instability is documented, not prevented (README).

## Phase 1: core — ChunkParams

- [x] **1.1** `chunker::ChunkParams { min: u32, avg: u32, max: u32 }` with
  `Default` = current constants; `validate()` checking fastcdc hard bounds +
  ordering, error names the bad key/value. `chunk_stream` gains a
  `params: ChunkParams` argument (callers updated).
  *Done when*: `cargo test -p git-cdc-core` passes with defaults.
- [x] **1.2** `Manifest::new` takes `ChunkParams` and records it in the
  header fields.
  *Done when*: manifest round-trip tests pass.

## Phase 2: CLI — read gitconfig

- [x] **2.1** `cmd_clean` reads `cdc.chunk.{min,avg,max}` with
  `git config --type=int` (each optional, defaulting per-key), validates via
  `ChunkParams::validate`, and chunks with the result. Invalid config is a
  hard error (never silently chunk with wrong params).
  *Done when*: e2e with repo-local `cdc.chunk.*` config produces chunk sizes
  within the configured bounds and manifest headers reflect them.

## Phase 3: server — protocol ceiling

- [x] **3.1** Body limit becomes fastcdc's 16 MiB `MAXIMUM_MAX` + 4096
  (protocol ceiling), decoupled from the client default.
  *Done when*: existing integration tests pass; a >8 MiB chunk upload is
  accepted.

## Phase 4: tests + docs

- [x] **4.1** Unit: validate() accepts defaults/bounds, rejects out-of-range
  and misordered values. E2E: repo with `cdc.chunk.min=64k, avg=256k,
  max=1m` commits a 4 MiB file → more chunks than default config would give,
  all ≤ 1 MiB, manifest headers show the configured values.
- [x] **4.2** README: config table entry + caveat (same config for all
  clients of a repo); spec doc note that header values echo the writer's
  config.

## Verification

```sh
cargo test --workspace
```
