---
date: 2026-07-22
feature: remove-code-duplication
coverage_before: 84.30%
coverage_after: 84.40%
---

# QA Report: remove-code-duplication

Scope: the new/changed code on this branch — `sync.rs`'s extracted
`pending_uploads`/`sweep_decision`, `remote.rs`'s new `opendal_options()`
parser and generic `remote()` path, and the rewritten `e2e_serverless.rs`.
Existing end-to-end suites (`gc_grace_survives_skewed_store_clock`,
`gc_deletes_orphans_past_grace_only`, `gc_accepts_large_live_sets`, the
`e2e_full`/`e2e_ssh` gc/push flows) already exercise most of the
push/gc semantics through the server and ssh paths — this pass targeted the
gaps those don't reach.

## Coverage

| File | Before | After |
| ---- | ------ | ----- |
| `crates/cli/src/sync.rs` | 87.76% regions | 88.15% regions |
| `crates/cli/src/remote.rs` | 45.64% regions | 46.15% regions |
| Workspace total | 84.30% regions | 84.40% regions |

`remote.rs`'s remaining uncovered regions (lines 16–89, `cmd_stdio`'s ssh
stdio-protocol handlers) are pre-existing and out of this branch's
scope — `e2e_ssh.rs` invokes `git-cdc stdio` as a real subprocess, which
`cargo-llvm-cov` does instrument, but that suite is limited by design to
the happy path plus a couple of transport-level checks; expanding it is a
separate task, not a gap this refactor introduced.

## Gaps Found and Closed

- **`sweep_decision`'s `KeepGrace` branch was never hit.** Every existing
  gc test (`e2e_full`, `e2e_ssh`, `e2e_serverless`'s original test) runs
  `--grace-secs 0`, so a chunk that's unreferenced but still within its
  grace window was untested for the CLI's own local/remote/ssh sweeps
  (the server's equivalent decision *is* covered, by
  `gc_grace_survives_skewed_store_clock`). Added
  `serverless_gc_keeps_chunks_within_grace_period` in
  `crates/cli/tests/e2e_serverless.rs`: pushes v1 + v2, drops v2 via
  `git reset --hard`, runs `gc --grace-secs 3600` and asserts nothing is
  swept, then `gc --grace-secs 0` and asserts the sweep happens — isolates
  the grace window as the variable, not gc itself.
- **`opendal_options()`'s malformed-`KEY=VALUE` error path was never hit.**
  All existing tests set well-formed options. Added
  `serverless_malformed_opendal_option_errors_clearly`: sets
  `cdc.opendal.option` to a value with no `=`, runs `push`, and asserts the
  failure names both the bad entry and the expected `KEY=VALUE` shape.

## Tests Added

- `serverless_gc_keeps_chunks_within_grace_period` (`crates/cli/tests/e2e_serverless.rs`) — `sweep_decision`'s `KeepGrace` outcome, on both a real (non-dry-run) gc and the subsequent zero-grace sweep.
- `serverless_malformed_opendal_option_errors_clearly` (`crates/cli/tests/e2e_serverless.rs`) — `opendal_options()`'s parse-failure path surfaces a clear, actionable error.

## Remaining Gaps

- `crates/cli/src/remote.rs`: `opendal_options()` with zero
  `cdc.opendal.option` entries set (the `unwrap_or_default()` → empty-line
  iterator → `Ok(vec![])` path) is untested directly — it's a one-line
  fallback on stdlib behavior (`str::lines()` on `""` yields nothing), not
  custom logic, and every scheme actually usable in this workspace (`fs`,
  `s3`, `azblob`, ...) needs at least one option (`root`, `bucket`, ...) to
  connect at all, so there's no realistic scheme to exercise a genuinely
  empty options list end-to-end without a synthetic no-op OpenDAL service.
  Low risk; not worth a dedicated test.
- `crates/cli/src/remote.rs` lines 16–89 (`cmd_stdio` internals) and the
  `cdc.ssh.remote`-without-`cdc.ssh.command` config path (lines 235–242):
  pre-existing coverage gaps unrelated to this refactor — out of scope
  here.

## Manual Test Cases

None — every path touched by this branch is covered by automated e2e/integration tests; no webhook/OAuth/browser flow involved.
