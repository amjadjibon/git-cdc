---
date: 2026-07-17
feature: git-cdc-mvp
branch: git-cdc-mvp
verdict: Pass
---

# QA Report: git-cdc-mvp

26 tests across 4 suites, all passing (`cargo test --workspace`).

## Coverage by invariant

| Invariant | Tested by |
|---|---|
| `smudge(clean(x)) == x` | unit round-trip, `e2e_filter` (real git add/checkout), `e2e_full` (network) |
| Missing chunk → passthrough, never wrong bytes | `smudge_with_empty_store_passes_manifest_through`, corrupt-chunk hard-error path in store tests |
| Chunk bounds (min/avg/max), contiguous offsets | `bounds_and_sizes_respected`, `file_exactly_max_chunk_size_round_trips` |
| Edge cases: empty file, < min size, exactly max size | chunker + manifest unit tests |
| Manifest strictness: sorted keys, LF-only, unknown-key preservation, byte stability | manifest unit tests |
| Dedup: 1-byte edit changes ≤2 chunks; second push uploads only changed chunks | `small_edit_changes_few_chunks`, `e2e_full` |
| Upload poisoning: server re-hashes before admit | store unit + server 422 integration test |
| Auth: 401 without/with-wrong token on all routes | `auth_is_enforced` |
| Fresh clone succeeds, `pull` materializes | `e2e_full` |
| Pre-push hook blocks publish with un-uploaded chunks | `e2e_full` (unreachable server case) |
| GC: dry-run deletes nothing, grace period, live set survives | server integration + `e2e_full` |
| `diff` changelist output | `diff_reports_changed_chunks` |

## Gaps accepted for MVP (not blocking)

- No concurrent-upload stress test (uploads are sequential by design; noted `ponytail:` in `cmd_push`).
- `install --global` path untested (would mutate the developer's real gitconfig from a test — deliberately skipped).
- Windows: pre-push hook and permissions are unix-only (`#[cfg(unix)]`); untested on Windows.
