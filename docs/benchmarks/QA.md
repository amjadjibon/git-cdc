---
date: 2026-07-19
feature: benchmarks
---

# QA Report: benchmarks

- All 12 benchmarks run to completion (`cargo bench -p git-cdc-core`);
  numbers recorded in RESULTS.md from a full local run.
- Benches are compile-checked in CI via clippy `--all-targets` (already in
  the ci workflow — no workflow change needed); CI does not *run* them:
  shared-runner timing is noise, and criterion baselines belong to a
  stable machine.
- Sanity checks embedded in the numbers themselves: small-params chunking
  ≈ default (confirms gear-hash-bound), compressible envelope encode
  faster than incompressible (confirms the early-exit heuristic), zstd
  decode faster than raw decode at equal verified bytes (confirms hashing
  dominates).

## Remaining Gaps

- No automated regression gate on the numbers (criterion's `--baseline`
  workflow is manual by design here); RESULTS.md is the reference point.
