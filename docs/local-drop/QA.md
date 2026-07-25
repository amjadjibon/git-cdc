---
date: 2026-07-25
feature: local-drop
mode: lite
suite: cargo test --workspace
result: pass
---

# QA: local-drop

## Coverage added

| Test | File | What it pins |
|------|------|--------------|
| `drop_reclaims_worktree_and_chunks` | `crates/cli/tests/serverless.rs` | The whole feature: both halves of local disk reclaimed, `git status` clean, `pull` restores byte-identical content |
| `drop_skips_files_with_uncommitted_changes` | `crates/cli/tests/serverless.rs` | SEC-003 — the data-loss guard. Edits survive verbatim, exit non-zero, no chunks removed |
| `drop_chunks_without_a_remote_touches_nothing` | `crates/cli/tests/serverless.rs` | SEC-002 — the abort happens before the worktree is touched, not after |
| `drop_keeps_chunks_missing_from_the_remote` | `crates/cli/tests/serverless.rs` | SEC-001 — the one chunk the remote lacks is the one that stays |
| `drop_dry_run_changes_nothing` | `crates/cli/tests/serverless.rs` | REQ-004 |
| `drop_requires_an_explicit_selection` | `crates/cli/tests/serverless.rs` | REQ-002 — bare `drop` refuses; unknown path errors |
| `drop_resolves_paths_from_a_subdirectory` | `crates/cli/tests/serverless.rs` | CON-004 |
| `drop_all_covers_every_tracked_file` | `crates/cli/tests/serverless.rs` | REQ-002 `--all` across two files |
| `drop_chunks_against_the_http_server` | `crates/cli/tests/full.rs` | `remote_present`'s HTTP branch — a distinct code path from the serverless listings |

`cdc_cmd` was added to `crates/cli/tests/utils.rs` (non-asserting git-cdc
runner, mirroring the existing `git_cmd`) so the refusal cases can assert a
non-zero exit.

## Gap analysis

- **Closed during QA**: `remote_present`'s HTTP branch was untested — every
  drop test used the fs-backed OpenDAL remote, and the batch-API reading
  ("no download action means absent") is separate logic. Added
  `drop_chunks_against_the_http_server`, which also covers the case the
  serverless tests cannot reach cheaply: a remote that holds *nothing*, where
  every chunk must survive.
- **Left open — ssh branch of `remote_present`**: `crates/cli/tests/ssh.rs`
  has a harness, but the ssh arm is three lines structurally identical to the
  opendal arm (one `list()`, one intersection), both of which are covered. Not
  worth the fixture cost.
- **Left open — shared-chunk drop**: dropping file A can remove a chunk file B
  also uses while B stays hydrated. This is intended (RESEARCH FIND-006: the
  remote can serve it again, `pull` refetches) and is structurally the same
  path as `--all`, which is covered.
- **Left open — very large files**: `cmd_drop` reads each file whole to hash
  it against `m.oid`. `cmd_pull` already buffers whole files the same way, so
  this adds no new ceiling.

## Result

```
cargo test --workspace   → all suites pass (12 CLI + 36 core + server)
cargo clippy --all-targets -- -D warnings → clean
```

Manually exercised the release binary end to end in a scratch repo (track →
commit → push → `drop --chunks` → `pull`), confirming the `git status`
behaviour outside the test harness, where timing differs.

## Pre-existing issue found, not fixed

`git cdc pull` leaves materialized files reading as modified in `git status`:
git's stat cache recorded the manifest's size, and nothing re-checks the
content. Verified that `git update-index --refresh`, `--really-refresh`, and
`--again` are all stat-only and do **not** settle it; only `git add` does,
which would re-chunk every file just pulled. Pre-existing (a fresh clone's
`pull` hits the same path — not introduced by `drop`), so it is left with a
`ponytail:` note in `cmd_pull` rather than fixed here. See the deviation note
in PLAN.md §2.
