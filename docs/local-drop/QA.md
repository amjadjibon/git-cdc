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

## Pre-existing issue found and fixed

`git cdc pull` left materialized files reading as modified in `git status`:
git's stat cache recorded the manifest's size, and nothing re-checked the
content. Pre-existing — a fresh clone's `pull` hits the same path — but it made
the documented `drop` → `pull` round trip look broken, so it is fixed here.

The first review left it open on a cost argument that measurement disproved:
`git add` on a 34 MB file costs 0.106s against pull's own 0.262s (~40%, not the
assumed doubling — the chunks already exist, so the store writes short-circuit).

`git add` is still the wrong mechanism, for a different reason: it re-chunks
with the *current* `cdc.chunk.*` config, and with `cdc.chunk.avg` changed since
the commit it silently staged a different manifest (blob `d5601509` against
HEAD's `f0fc41d4`) — verified.

`settle_index` instead rewrites each entry with `git update-index --cacheinfo`
(same mode and blob, which zeroes the recorded size — git's own signal that the
content must be compared for real) and then `--refresh`. Stat-only, so it can
never stage anything:

| Scenario | Result |
|----------|--------|
| chunk params unchanged | status clean, index blob unchanged |
| chunk params changed | stays honestly ` M` unstaged, index blob unchanged |

Cost 0.118s on the 34 MB file (+45% of pull). A genuinely divergent file keeps
a zeroed size, so later `git status` runs re-check it — unavoidable, since its
content really does differ from the index.

| Test | Pins |
|------|------|
| `drop_reclaims_worktree_and_chunks` | post-`pull` `git status` is now clean |
| `pull_never_stages_a_rechunked_manifest` | changed chunk params never stage a re-chunked manifest — fails if swapped for `git add` |
| `pull_preserves_the_file_mode` | an executable file stays 100755 — fails if the mode is hardcoded |
