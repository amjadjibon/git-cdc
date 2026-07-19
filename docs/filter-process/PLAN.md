---
status: Ready
version: 1.0
date: 2026-07-19
feature: filter-process
research: docs/filter-process/RESEARCH.md
---

# Plan: git filter-process protocol (+ constant-time token compare)

One long-running filter per git operation instead of a process per file.
Design per RESEARCH.md; `delay` capability deferred.

## Assumptions

- Protocol details per RESEARCH.md; the e2e suite against real git (2.39
  locally, 2.54 in CI) is the conformance gate.
- `install` sets `filter.cdc.process = git-cdc filter-process` and keeps
  `clean`/`smudge` as documented fallback for git < 2.11.
- Constant-time auth compare via double-BLAKE3 (no new dependency).

## Phase 1: pkt-line + filter refactor

- [x] **1.1** `core::pktline`: `read_packet`/`write_packet`/flush,
  `PktReader: Read` (content packets until flush), `PktWriter: Write`
  (≤65516-byte packets). Unit tests: round trip, empty payload, max-size
  payload, oversized write splits.
- [x] **1.2** Extract `clean_stream(reader, writer, store, params)` and
  `smudge_stream(reader, writer, store)` from the one-shot commands; the
  commands become thin stdin/stdout wrappers. Behavior unchanged.
  *Done when*: existing e2e_filter suite passes untouched.

## Phase 2: filter-process command

- [x] **2.1** Hidden `git-cdc filter-process` subcommand: handshake,
  capability negotiation (clean+smudge), per-file loop calling the shared
  streams, per-file `status=error` on failure (process survives), store +
  chunk params opened once.
- [x] **2.2** `install` writes `filter.cdc.process` (keeping clean/smudge).
  *Done when*: a repo configured only with `process` round-trips
  add/checkout byte-identically.

## Phase 3: server constant-time compare

- [x] **3.1** Auth middleware compares `blake3(presented) == blake3(expected)`
  (constant-time Hash eq). Existing auth tests pass.

## Phase 4: tests + docs

- [x] **4.1** e2e (process-configured scratch repo, no clean/smudge keys):
  add/checkout round trip; multi-file checkout through ONE process; file
  larger than one packet (> 64 KiB) and larger than a chunk; empty-store
  passthrough + pull materialization; corrupt-store error keeps other
  files checking out.
- [x] **4.2** README + book: install now uses the long-running filter;
  remove filter-process from out-of-scope lists (README, book development
  chapter, PONYTAIL-DEBT related-scope note).

## Verification

```sh
cargo test --workspace && cargo clippy --workspace --all-targets && cargo fmt --all --check
```
