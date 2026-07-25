---
date: 2026-07-25
plan: docs/local-drop/PLAN.md
plan_version: 1.0
reviewer: Claude
verdict: Blocked
---

# Plan Review: local-drop

## Verdict

**Blocked** — TASK-003 overwrites the worktree file with no check that its
content still matches the manifest, so `git cdc drop` on a file with
uncommitted edits destroys them unrecoverably. Three further gaps (a
requirement no flag can express, an abort that fires too late, and unnormalized
path arguments) will bite during implementation.

## Findings

### [BLOCK-001] `drop` overwrites uncommitted local modifications

**Phase**: 1 (TASK-003, REQ-001, SEC-001)

**Issue**: TASK-003 selects "each selected file whose worktree content is not
already manifest text" and writes the index blob over it. A file the user has
edited but not committed is exactly that — not manifest text — so `git cdc drop
<path>` silently replaces their work with the manifest. There is no recovery:
the content was never chunked (the clean filter runs at `git add`, not on
edit), so it is in no store, local or remote, and `git cdc pull` will
cheerfully "restore" the committed version over the top. `--all` turns this
into a whole-repo hazard. SEC-001 gates chunk deletion on remote presence but
says nothing about the worktree half, and no RISK-* covers it.

Note this is the inverse of `cmd_pull`'s behaviour at `sync.rs:228-231`, which
deliberately skips files that are not manifest text with the comment "already
real content (or locally modified — leave it)". `drop` targets precisely the
files `pull` refuses to touch, so it cannot inherit that safety.

**Fix**: Before overwriting, verify the worktree content is what the manifest
describes: blake3-hash the file and compare against `m.oid`, which the manifest
already records. Mismatch → skip the file with a warning naming it, and exit
non-zero if every selected file was skipped. Hashing costs one read of a file
being deleted anyway, and unlike `git diff`/`git diff-files` it does not depend
on git's stat cache, which RESEARCH FIND-002 showed is unreliable for exactly
this size-drop scenario. Add `--force` only if a use case appears; do not add
it speculatively. Add a REQ or SEC line and a matching test.

### [REVISE-001] REQ-003's standalone `--chunks` mode is unimplementable as specified

**Phase**: 1 (REQ-003 vs TASK-003, ASSUMPTION-002)

**Issue**: REQ-003 says `--chunks` "is independent of the worktree drop:
`--chunks` alone reclaims the redundant store copy while leaving the file
hydrated." But TASK-003 drops the worktree copy for every selected file
unconditionally, with no flag guarding it. There is no argument combination
that produces the behaviour REQ-003 promises. ASSUMPTION-002 claims the mode
"falls out of the flag design for free" — it does not; it needs a third flag
and a second documented mode.

**Fix**: Cut REQ-003's standalone mode and ASSUMPTION-002. The reclaim it
offers is the smaller half (RESEARCH FIND-001: 3,000,000 B worktree vs 2.9 MiB
store, and a still-hydrated file will re-chunk on the next `git add` anyway),
and one command with one meaning is worth more than the saving. If it is kept
instead, add an explicit `--keep-worktree` flag, a task making TASK-003
conditional on it, and a test.

### [REVISE-002] SEC-002's abort fires after the worktree has already been rewritten

**Phase**: 1 (SEC-002, TASK-003/004/005 ordering)

**Issue**: SEC-002 requires `--chunks` with no remote to "abort before deleting
anything", and TEST-006 asserts a non-zero exit with the local store untouched.
But the task order is TASK-003 (rewrite worktrees) → TASK-004/005 (resolve
remote, delete chunks). By the time `remote()` returns `Err`, every selected
file has already been dehydrated and re-added. The store is untouched, so
TEST-006 passes while the command still did half its work and reported failure
— the worst outcome to debug.

**Fix**: Reorder — resolve the remote and compute `remote_present` first, then
rewrite worktrees, then delete chunks. Make this explicit in the task text
rather than leaving it to the implementer's ordering. Extend TEST-006 to assert
the worktree file is unchanged too, not just the store.

### [REVISE-003] Path arguments are not normalized to repo-relative

**Phase**: 1 (TASK-002, TASK-003)

**Issue**: TASK-002 matches arguments "against the tracked paths" from
`index_manifests()`, which returns repo-relative paths (`git ls-files`), and
TASK-003 feeds them to `git show :<path>`, which also takes repo-relative
paths. A user running `git cdc drop asset.bin` from a subdirectory passes a
cwd-relative path that matches nothing and gets "names no tracked file" for a
file that is plainly tracked. Every other command in the CLI is cwd-agnostic
(`pull`, `push`, and `gc` take no paths), so this is the first place the
distinction appears and there is no existing helper for it.

**Fix**: Add a task to normalize each argument to repo-relative before
matching — canonicalize against `repo_root()` (`git.rs:25`), or delegate to
`git ls-files --full-name -- <arg>`. Add a test invoking `drop` from a
subdirectory.

### [SUGGEST-001] `--dry-run` should not hard-require a remote

**Phase**: 1 (SEC-002, REQ-004)

**Issue**: SEC-002 aborts `--chunks` whenever no remote is configured. Under
`--dry-run` nothing is deleted, so the abort blocks the one invocation a user
would reach for to find out what a drop *would* free before configuring a
remote.

**Fix**: Under `--dry-run`, report what would be dropped and state that chunk
removal needs a remote, exiting zero. Keep the hard error for real runs.

### [SUGGEST-002] Phase 1 carries 10 tasks

**Phase**: 1

**Issue**: Ten tasks in one phase is double the ~5 the structure guidance
suggests, and the phase mixes three domains (command implementation, docs,
tests). `lite` mode mandates a single phase, so this is inherent to the mode
rather than a planning error, but the phase is large enough that a partial
failure mid-phase leaves an ambiguous state.

**Fix**: Either accept it (defensible for `lite`) or re-run at `full` with the
natural split: command + safety gate, then docs + tests. Not worth re-planning
on its own.

## What's Good

- The load-bearing spike results carry into the plan as constraints rather than
  prose: CON-001 (write index blob bytes verbatim) and CON-002 (`git add` to
  refresh the stat cache) are exactly the two things that silently break
  `git status`, and RISK-002 pins the first with a test.
- SEC-001's remote-presence gate is the right safety model for the chunk half,
  and RESEARCH FIND-006's argument for why it is *sufficient* — no per-file
  dedup analysis needed — is recorded rather than re-derived.
- The three RESEARCH open questions are all resolved in the plan with stated
  reasoning instead of being carried forward as unknowns.
- No `undrop` command: ASSUMPTION-004 correctly identifies that `git cdc pull`
  already is one.

## Machine-Readable Verdict

```yaml
verdict: Blocked
block: 1
revise: 3
suggest: 2
blocking_ids: [BLOCK-001]
```
