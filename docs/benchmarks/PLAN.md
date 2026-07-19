---
status: Ready
version: 1.0
date: 2026-07-19
feature: benchmarks
---

# Plan: criterion benchmarks for the hot paths

One bench target (`crates/core/benches/core.rs`) covering the paths where
throughput actually matters; numbers recorded in docs so regressions have
a baseline to argue against.

## Assumptions

- criterion 0.8, `harness = false`, dev-dependency of core only. Clippy's
  `--all-targets` in CI compile-checks benches; CI does not *run* them
  (shared runners make timing noise, not signal).
- Deterministic xorshift data (same generator as the test suites) so runs
  are comparable.
- Groups: chunker throughput (default + small params, ~32 MiB),
  envelope encode/decode (compressible + incompressible, 2 MiB),
  manifest encode/parse (1000-chunk manifest), pkt-line writer/reader
  round trip (8 MiB), disk store put/get (2 MiB chunk, tempdir).

## Phase 1: bench target

- [x] **1.1** Add criterion dev-dep + `[[bench]]`; write the five groups
  with `Throughput::Bytes` where it's a bytes-per-second story.
  *Done when*: `cargo bench -p git-cdc-core` runs all groups and reports.

## Phase 2: record + document

- [x] **2.1** Run on this machine, record results in
  `docs/benchmarks/RESULTS.md` (hardware noted); book Development chapter
  gains a Benchmarks section (`cargo bench -p git-cdc-core`).
  *Done when*: results file exists with real numbers; book builds.

## Verification

```sh
cargo bench -p git-cdc-core
cargo clippy --workspace --all-targets && cargo fmt --all --check
```
