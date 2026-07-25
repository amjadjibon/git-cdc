---
date: 2026-07-25
branch: local-drop
reviewer: Claude
verdict: Approve
---

# Code Review: local-drop

## Verdict

**Approve** (iteration 2) — iteration 1 returned Request Changes with 3 Medium
and 3 Low findings; all six were fixed in `81602bb` and the follow-up commit,
each Medium with a regression test verified to fail against the old code.
Findings are kept below with their resolutions for the record.

## Summary

Reviewed `main...HEAD`: the new `cmd_drop`/`remote_present`/`resolve_paths` in
`crates/cli/src/sync.rs`, the repo-root fix in `index_manifests`
(`crates/cli/src/git.rs`), the subcommand wiring, 9 new e2e tests, and docs.
The dangerous paths are handled the way the plan requires — the remote is
resolved before any file is touched, the blake3-vs-`m.oid` check gates every
overwrite, and the index blob is written verbatim. The findings below are
about which chunks get deleted and how the result is reported, not about the
gates themselves.

## Findings

### [MED-001] Chunks are deleted for files that were skipped, not dropped *(Medium)* — **fixed**

**File**: `crates/cli/src/sync.rs:466` (candidate set) and `:539` (removal)
**Category**: Correctness

**Issue**: `candidates` is built from every **selected** file, but the loop
that follows may skip a file — because it has uncommitted edits (`:521`), is
already manifest text, or is absent from the worktree. Its chunks stay in
`candidates` and are deleted anyway at `:539`.

Concretely, `git cdc drop --chunks --all` in a repo where `b.bin` has
uncommitted edits: `b.bin` is correctly left alone in the worktree, and then
its committed chunks are removed from the local store regardless. Not data
loss — the remote-presence gate still holds, so `git cdc pull` refetches — but
it contradicts what the command reports ("1 files skipped") and means a later
`git checkout -- b.bin` silently degrades to manifest passthrough instead of
restoring content from a store that had it a moment ago.

**Fix**: Keep resolving the remote up front (SEC-002 needs that), but build the
deletion set from the files actually dropped. Collect `dropped_paths` in the
loop, then intersect: delete `h` only when `removable.contains(h)` **and** some
dropped file references it. Under `--dry-run` the same filter applies to the
reported count.

### [MED-002] Non-ASCII filenames never match — `git ls-files` C-quotes them *(Medium)* — **fixed**

**File**: `crates/cli/src/sync.rs:99` (`resolve_paths`), `crates/cli/src/git.rs:139` (`index_manifests`)
**Category**: Correctness

**Issue**: `git ls-files` quotes paths outside ASCII by default
(`core.quotePath` defaults to true). Verified on git 2.39.5: a file named
`café.bin` comes back as the 17-character literal `"caf\303\251.bin"`, quotes
and backslashes included.

So `resolve_paths` compares that literal against `index_manifests`' paths —
which are quoted the same way, so they *do* match each other, but the string
then flows into `root.join(path)` (`sync.rs:511`) and `git show :{path}`
(`sync.rs:530`), where the quoting is not stripped. The file is silently
invisible to `drop`, and to `pull`'s materialization loop for the same reason.
`index_manifests` predates this branch, but the diff modifies it and `drop` is
the first command to take paths from the user, so it is in scope.

**Fix**: Pass `-c core.quotePath=false` to the `ls-files` and `check-attr`
invocations in both functions — verified to emit `café.bin` unquoted. A `-z`
NUL-delimited read is the more thorough option if paths with embedded newlines
ever matter; `core.quotePath=false` covers the realistic cases for one flag.
Add a test with a non-ASCII filename.

### [MED-003] "bytes freed" reports only the worktree half *(Medium)* — **fixed**

**File**: `crates/cli/src/sync.rs:551`
**Category**: Correctness

**Issue**: `freed` accumulates `m.size` only. With `--chunks` — the flag whose
entire purpose is the *other* half of the disk usage — the removed chunk bytes
are never added, so the command under-reports what it reclaimed by roughly
half. REQ-005 asks for bytes freed; a user comparing the number against `du`
will not be able to reconcile it.

**Fix**: Accumulate the removed chunks' bytes separately (`candidates` already
carries each chunk's length) and report the two figures, e.g. `… 3145728 bytes
of worktree + 2984021 bytes of chunks freed`. Chunk length is the uncompressed
size while the store holds enveloped bytes, so either label it as such or read
the on-disk size before removing.

### [LOW-001] The "everything was modified" bail also fires when files were legitimately skipped *(Low)* — **fixed**

**File**: `crates/cli/src/sync.rs:536`
**Category**: Correctness

**Issue**: `if dropped == 0 && modified > 0` exits non-zero whenever nothing
was dropped and at least one file had uncommitted edits. With `--all` on a
repo that is already fully dropped plus one modified file, every other file
was skipped for the harmless reason ("already manifest text") and the command
still fails. The plan's wording was "exit non-zero if every selected file was
skipped for an oid mismatch".

**Fix**: Gate on `modified == selected.len()`, or on `dropped == 0 && skipped
== modified`.

### [LOW-002] A rewrite that fails to re-add leaves the file looking modified *(Low)* — **fixed**

**File**: `crates/cli/src/sync.rs:534`
**Category**: Correctness

**Issue**: `fs::write` then `git_out(… "add" …)` — if the `git add` fails, the
`?` propagates after the worktree file has already been replaced, leaving
exactly the dirty-index state CON-002 exists to prevent, with no hint that a
`git add <path>` would clear it.

**Fix**: Wrap the `git add` failure with context naming the path and the
remedy. Not worth a rollback — the content is recoverable by `pull`.

### [LOW-003] Dry-run test asserts on an output string *(Low)* — **fixed**

**File**: `crates/cli/tests/serverless.rs:441`
**Category**: Test quality

**Issue**: `assert!(log.contains("would drop 1 files"))` couples the test to
the exact summary wording, which MED-003's fix will change. The behavioural
assertions either side (file unchanged, store unchanged) are what matter.

**Fix**: Assert on `"would drop"` alone, or drop the string check.

### [INFO-001] `pull`'s stat-cache wart is documented, not fixed

**File**: `crates/cli/src/sync.rs:331`
**Category**: Correctness

`cmd_pull` leaves materialized files reading as modified in `git status`. The
`ponytail:` note records what was verified (all three `update-index` variants
are stat-only; only `git add` settles it, at the cost of re-chunking every
pulled file) and why it was left. Pre-existing — a fresh clone's `pull` hits
the same path — so out of scope here, but it is the mirror image of the
problem `drop` had to solve and deserves its own follow-up.

## Resolutions (iteration 2)

| Finding | Fix | Regression test |
|---------|-----|-----------------|
| MED-001 | `reclaimable` set: only chunks of files that actually reached manifest state (dropped this run **or** already dropped) are eligible. A `pinned` set additionally protects chunks a skipped file still needs, since dedup can share one chunk between a dropped and a skipped file. | `drop_keeps_chunks_of_skipped_files`, `drop_keeps_chunks_shared_with_a_skipped_file`, `drop_chunks_reaches_an_already_dropped_file` |
| MED-002 | `core.quotePath=false` on `ls-files`/`check-attr` in both `resolve_paths` and `index_manifests`, via a shared `git::QUOTE_PATH`. | `drop_handles_non_ascii_filenames` — verified to fail with `quotePath=true` (`"caf\303\251.bin" is not a git-cdc file`) |
| MED-003 | Worktree bytes and chunk bytes reported separately. | covered by existing dry-run assertions |
| LOW-001 | Bail condition is now `modified == selected.len()`. | — |
| LOW-002 | `with_context` naming the path and the `git add -- <path>` remedy. | — |
| LOW-003 | Assertion relaxed to `"would drop"`. | — |

Both Medium fixes were confirmed non-vacuous by reverting each fix in place and
watching its test fail, then restoring it.

## What's Good

- The abort ordering is real, not just claimed: `remote_present` is called at
  `:474` before the write loop at `:504`, and
  `drop_chunks_without_a_remote_touches_nothing` asserts the worktree file is
  byte-identical afterwards — which is what distinguishes a correct abort from
  one that fails after doing half the work.
- `drop_chunks_against_the_http_server` covers the case the serverless tests
  cannot reach cheaply: a remote holding *nothing*, where the gate must keep
  every chunk. It exercises the gate's "deny" path rather than only its
  "allow" path.
- The `index_manifests` fix is a root-cause fix — running every git call from
  the repo root also repairs `cmd_pull`'s latent `root.join(cwd_relative_path)`
  bug, rather than working around it inside `drop`.

## Pre-Merge Checklist

- [x] All Critical and High findings resolved (none found)
- [x] No secrets in committed files; `.gitignore` covers new artifact types
- [x] Tests cover changed behaviour + at least one unhappy path
- [x] All async calls awaited or errors handled; resources closed in all paths
- [ ] N/A: auth/user data
- [ ] N/A: uploads

## Machine-Readable Verdict

```yaml
verdict: Approve
critical: 0
high: 0
medium: 0
low: 0
info: 1
blocking_ids: []
```
