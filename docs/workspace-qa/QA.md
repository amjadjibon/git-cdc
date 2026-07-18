---
date: 2026-07-18
feature: workspace-qa
coverage_before: 74.8%
coverage_after: 88.6%
---

# QA Report: workspace test improvement

Region coverage via `cargo llvm-cov --workspace` (line coverage: 77.4% → 92.0%).

**Update (same day):** the S3 suites now self-host an in-process s3s-fs
server instead of being env-gated, so `cargo test --workspace` covers the S3
paths by default. `GIT_CDC_TEST_S3_ENDPOINT` still overrides with a real S3.

## Coverage

| File | Before | After |
| ---- | ------ | ----- |
| core/src/bin/git-cdc.rs | 73.5% | 74.6% |
| core/src/manifest.rs | 94.0% | 95.1% |
| core/src/protocol.rs | untested | 100% |
| core/src/store.rs | 93.0% | 94.2% |
| server/src/main.rs | 0% | 51.3% |
| core/src/store/s3.rs | 0% | 89.3% |
| **TOTAL (regions)** | **74.8%** | **88.6%** |

## Bug found and fixed

`Manifest::parse` accepted CRLF chunk/header lines — `str::lines()` silently
strips the `\r`, violating the LF-only rule in `docs/spec/manifest.md`. Fixed
with an explicit CR rejection in `parse` (crates/core/src/manifest.rs).

## Tests Added

Unit:

- `manifest::rejects_carriage_returns_anywhere` — CRLF bodies and a single mixed CRLF line are invalid (regression for the fix above)
- `manifest::rejects_header_key_after_chunk_lines` — section ordering is enforced
- `manifest::rejects_uppercase_and_underscore_keys` — key charset `[a-z0-9.-]`
- `protocol::batch_request_wire_format_is_lfs_shaped` — locks `ref` rename, lowercase operations, field names
- `protocol::optional_fields_are_omitted_not_null` — absent ref/transfers/actions/error are omitted from JSON
- `protocol::missing_optional_fields_deserialize` — defaults on the read side (`dry_run` false)
- `store::list_skips_foreign_and_temp_files` — crashed-put `.tmp-*` leftovers and stray files are not reported as chunks
- `server main::s3_backend_requires_bucket` / `disk_backend_requires_root` — startup flag pairing (PLAN 1.3 done-when); documents that a defaulted backend skips clap's `required_if_eq` and relies on the runtime bail

E2E:

- `smudge_never_emits_corrupt_data` — every chunk tampered on disk; checkout must fail or fall back to manifest text, never materialize wrong bytes (the flagship safety promise, previously untested)
- `track_without_patterns_errors` — usage error on bare `git cdc track`
- `install_leaves_foreign_pre_push_hook_alone` — pre-existing hook is warned about, never clobbered
- `push_with_missing_local_chunk_says_how_to_recover` — server wants chunks the local store lost; error names `git cdc pull`
- `sync_without_remote_config_names_both_options` — error mentions both `cdc.url` and `cdc.s3.bucket`

## Remaining Gaps

- `core/src/client.rs` error branches (~27% of regions) — non-2xx `bail!` arms;
  server-side rejections are asserted in `server/tests/integration.rs`, the
  client-side message formatting is not worth a mock HTTP server.
- `git-cdc.rs` remaining ~13% — stdin lock paths and error arms not
  reachable in-process.
- `server/src/main.rs` runtime half — the `main()` body (socket bind, backend
  construction); exercised by manual smoke tests only.

## Manual Test Cases

- [ ] Run the S3 suites against a real store once per release:
  `GIT_CDC_TEST_S3_ENDPOINT=… + AWS creds, cargo test --workspace`
- [ ] `git-cdc-server --backend s3` end-to-end against MinIO/RustFS (startup, batch, gc)
