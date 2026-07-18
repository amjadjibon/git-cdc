---
date: 2026-07-18
feature: configurable-chunking
coverage_before: n/a (feature branch)
coverage_after: all new paths covered
---

# QA Report: configurable-chunking

## Tests Added

- `chunker::params_validation` — defaults + fastcdc bound edges accepted;
  out-of-range rejected naming the config key; misordered rejected.
- `chunker::custom_params_change_chunking` — smaller bounds → more chunks,
  all ≤ configured max; oid unchanged (chunking-independent).
- e2e `chunk_sizes_configurable_via_gitconfig` — repo-local `64k/256k/1m`
  config (git suffix parsing), manifest headers echo configured values,
  ≥4 chunks for 4 MiB, byte-identical restore.
- e2e `invalid_chunk_config_fails_the_add` — `cdc.chunk.min 63` surfaces the
  key in the error during `git add`.
- server `chunks_above_old_default_max_are_accepted` — 12 MiB upload → 200,
  17 MiB (over ceiling) → 413.

## Also fixed during QA

E2E suites now set `GIT_CONFIG_GLOBAL=/dev/null` / `GIT_CONFIG_SYSTEM=/dev/null`
on every git/git-cdc subprocess — the suites previously inherited the
developer's real global gitconfig (a global `cdc.s3.bucket` made e2e_full's
push hit a live MinIO).

## Remaining Gaps

- `chunk_params()` unset-vs-invalid distinction is covered indirectly by the
  two e2e tests; no direct unit test (requires a git repo fixture — e2e is
  the natural home).
