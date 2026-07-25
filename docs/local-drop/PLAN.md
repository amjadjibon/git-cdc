---
goal: Add `git cdc drop` to reclaim local disk from hydrated tracked files
version: 1.1
date_created: 2026-07-25
last_updated: 2026-07-25
owner: amjadjibon
status: 'Planned'
tags: [feature]
---

# Add `git cdc drop`

![Status: Planned](https://img.shields.io/badge/status-Planned-blue)

A tracked file costs local disk twice: the hydrated worktree copy plus its
chunks in `.git/cdc/objects`. `git cdc drop` reclaims both — it rewrites the
worktree file back to its committed manifest (the state a fresh clone is
already in) and, with `--chunks`, deletes local chunks the remote provably
holds. Nothing new is stored to "keep the hash": the manifest is already in
git, and `git cdc pull` is already the rehydrate path.

Follows the recommendation in [RESEARCH.md](RESEARCH.md). Revised to v1.1 per
[PLAN-REVIEW.md](PLAN-REVIEW.md) — BLOCK-001 (SEC-003), REVISE-001 (REQ-003),
REVISE-002 (SEC-002 ordering), REVISE-003 (CON-004), SUGGEST-001 (REQ-004).

## 1. Requirements & Constraints

- **REQ-001**: `git cdc drop <path>...` overwrites each named tracked file with
  the exact bytes of its index blob (its manifest), leaving `git status` clean.
- **REQ-002**: `git cdc drop --all` does the same for every tracked file in the
  index. With neither `--all` nor paths, the command errors — a whole-repo
  destructive default is a footgun (RESEARCH open question, resolved this way).
- **REQ-003**: `--chunks` additionally removes the selected files' chunks from
  the local `DiskStore`. It always accompanies the worktree drop; there is no
  chunks-only mode. Dropping chunks while leaving a file hydrated reclaims the
  smaller half and the next `git add` re-chunks the file anyway, so the second
  mode is not worth a third flag (PLAN-REVIEW REVISE-001).
- **REQ-004**: `--dry-run` reports what would be dropped and frees nothing,
  matching the existing `git cdc gc --dry-run` convention. Under `--dry-run`,
  `--chunks` without a configured remote reports what it would free and notes
  that real removal needs a remote, exiting zero rather than erroring
  (PLAN-REVIEW SUGGEST-001).
- **REQ-005**: Output reports files dropped, chunks removed, and bytes freed.
- **SEC-001**: `--chunks` deletes a chunk only after the configured remote
  confirms it holds that chunk. A chunk the remote lacks is kept, with a
  warning naming it. Remote presence is a sufficient gate — a chunk the remote
  holds is refetchable by `git cdc pull` regardless of which other files or
  commits share it (RESEARCH FIND-006).
- **SEC-002**: `--chunks` with no remote configured aborts before *any* side
  effect, worktree rewrites included — so the remote must be resolved before
  the first file is touched (PLAN-REVIEW REVISE-002). `remote()` returning
  `Err` is a hard error here, unlike in `cmd_gc`, which merely skips the remote
  sweep.
- **SEC-003**: A file is dropped only when its worktree content still matches
  the manifest — blake3-hash the file and compare against `m.oid` before
  overwriting. A mismatch means uncommitted edits that exist in no store, local
  or remote, and that overwriting would destroy unrecoverably (PLAN-REVIEW
  BLOCK-001). Mismatching files are skipped with a warning naming them; if
  every selected file is skipped, exit non-zero. The whole-file oid comparison
  is exact and, unlike `git diff-files`, does not depend on git's stat cache,
  which RESEARCH FIND-002 showed is unreliable for exactly this size drop. No
  `--force` escape hatch until a use case appears.
- **CON-001**: The worktree rewrite must write the **index blob bytes
  verbatim** (`git show :<path>`), not a re-encoded `Manifest`, so the blob OID
  cannot drift from what is committed.
- **CON-002**: Each dropped path must be re-added with `git add -- <path>`
  afterwards. Git's stat cache distrusts the size drop and reports the file
  modified otherwise, and `git update-index --refresh` does not clear it
  (RESEARCH FIND-002/003). The re-add stages nothing — the clean filter passes
  manifest text through unchanged, yielding the same blob OID.
- **CON-003**: No new dependencies, no new on-disk state, no manifest format
  change.
- **CON-004**: Path arguments must be normalized to repo-relative before
  matching against `index_manifests()` or feeding `git show :<path>`, so the
  command works from a subdirectory. This is the first path-taking command in
  the CLI, so no existing helper covers it (PLAN-REVIEW REVISE-003).

## 2. Implementation Steps

> After each phase: `git add -u` and commit. No `Co-authored-by:`. Tick `[x]` as each task completes.

### Phase 1: `git cdc drop` command, docs, and e2e coverage

**Goal**: Ship the whole feature — subcommand, implementation, docs, tests —
in one branch. The change is one command over machinery that already exists
(`index_manifests`, smudge passthrough, `DiskStore::remove`, the per-remote
listings in `cmd_push`), so splitting it into phases would cost more than it
buys. Task order encodes SEC-002: everything that can fail is resolved before
anything is written.

- [x] TASK-001: Add `Cmd::Drop { paths: Vec<String>, all: bool, chunks: bool, dry_run: bool }`
      to the `Subcommand` enum in `crates/cli/src/main.rs` (after `Gc`, before
      `Diff`), with doc comments matching the style of the neighbouring
      variants, and dispatch it to `sync::cmd_drop` in `main()`.
- [x] TASK-002: Implement `pub fn cmd_drop(paths: &[String], all: bool, chunks: bool, dry_run: bool) -> Result<()>`
      in `crates/cli/src/sync.rs`. Select tracked files from `index_manifests()`:
      `--all` takes every entry; otherwise normalize each argument to
      repo-relative (canonicalize against `repo_root()` at `git.rs:25`, or
      delegate to `git ls-files --full-name -- <arg>`) per CON-004, match it
      against the tracked paths, and `bail!` on an argument that names no
      tracked file. `bail!` with a usage message when neither `--all` nor any
      path is given.
- [x] TASK-003: Add a `remote_present(chunks: &HashSet<blake3::Hash>) -> Result<HashSet<blake3::Hash>>`
      helper in `sync.rs` that returns which of `chunks` the configured remote
      holds: `Remote::Opendal` and `Remote::Ssh` via one `list()` intersected
      with `chunks` (same shape as `cmd_push` at `sync.rs:122` and
      `sync.rs:135`); `Remote::Http` via `client.batch(Operation::Download, …)`,
      counting an object as present when it has no `error` and carries a
      download action. Propagate the `remote()` error rather than swallowing it.
- [x] TASK-004: In `cmd_drop`, when `--chunks` is set, resolve the remote and
      compute `remote_present` **before touching any file** (SEC-002) — with
      the `--dry-run` exception from REQ-004, where a missing remote degrades
      to a note instead of an error. Collect the candidate set as the union of
      the selected manifests' chunk hashes that `store.has()`.
- [x] TASK-005: In `cmd_drop`, for each selected file: skip it when its
      worktree content is already manifest text (`is_manifest`, as `cmd_pull`
      checks at `sync.rs:228`); otherwise blake3-hash the file and compare
      against `m.oid`, skipping with a warning on mismatch (SEC-003). For the
      survivors, write the raw index blob over the file via `git show :<path>`
      — reuse the `git::git_out`/`Command` pattern already used in
      `index_manifests` — then run `git add -- <path>` to refresh the stat
      cache (CON-002). Accumulate `m.size` as bytes freed. Exit non-zero if
      every selected file was skipped for an oid mismatch.
- [x] TASK-006: In `cmd_drop`, when `--chunks` is set, delete the chunks
      confirmed by TASK-004 with `DiskStore::remove`, and `eprintln!` a warning
      per chunk the remote lacks (naming it with `oid_str`) — those stay. Skip
      all deletions under `--dry-run`.
- [x] TASK-007: Print a summary line in the `git-cdc: …` style used by
      `cmd_push`/`cmd_gc`, reporting files dropped, files skipped, chunks
      removed, chunks kept for lack of a remote copy, and bytes freed; prefix
      with "would" under `--dry-run`.
- [x] TASK-008: Improve the `cmd_push` error at `crates/cli/src/sync.rs:86` (and
      the matching `with_context` messages for the opendal and ssh remotes) to
      mention `git cdc drop --chunks` as a cause of a missing local chunk
      alongside the existing `git cdc pull` advice.
- [x] TASK-009: Document the command — add a `git cdc drop` row to the table in
      `docs/book/src/commands.md`, a short section explaining the two halves of
      local disk usage, that `git cdc pull` rehydrates, and that files with
      uncommitted edits are skipped; and a "Reclaiming local disk" note in
      `README.md` near the existing gc material. Cross-link
      `docs/book/src/gc.md` so the difference is explicit: `gc` sweeps
      *unreferenced* chunks, `drop` deletes *referenced* ones on purpose.
- [x] TASK-010: Add e2e test `drop_reclaims_worktree_and_chunks` to
      `crates/cli/tests/serverless.rs` (TEST-001), following the existing
      `serverless_push_clone_pull_gc` fs-scheme remote setup.
- [x] TASK-011: Add the selection and safety-rule coverage of TEST-002 through
      TEST-008, using `utils::git_cmd`-style non-asserting invocations for the
      cases that must exit non-zero.

**Completion criteria**: `cargo test -p git-cdc-cli` and `cargo clippy --all-targets -- -D warnings`
pass. In a repo with a configured remote and a pushed tracked file:
`git cdc drop --chunks <path>` shrinks the worktree file to its manifest,
leaves `git status --porcelain` empty, empties the file's chunks from
`.git/cdc/objects`, and a subsequent `git cdc pull` restores byte-identical
content with `git status --porcelain` still empty. Editing that file without
committing and re-running `git cdc drop` leaves the edit intact.

**Deviation (implementation)**: the post-`pull` half of that criterion is not
achievable without an out-of-scope change and was relaxed. `pull` grows the
file far past the size git recorded for the manifest, and git reports the
difference as an unstaged modification until something re-runs the clean
filter. Verified during implementation that neither `git update-index
--refresh` nor `--really-refresh` nor `--again` settles it — all are stat-only
— and only `git add` does, which would re-chunk every file just pulled. This
is pre-existing (a fresh clone's `pull` hits the same path, it is not new to
`drop`) and is left as a follow-up with a `ponytail:` note in `cmd_pull`. The
test asserts instead that `pull` stages nothing (index blob == HEAD blob). The
post-`drop` half of the criterion holds and is asserted.

**git commit**: `git add -u && git commit -m "feat: add git cdc drop to reclaim local disk"`

**Agent Prompt**:
```
You are a sub-agent implementing Phase 1 of local-drop in the git-cdc repo
(Rust workspace: crates/cli, crates/core, crates/server).

Context: a tracked file costs local disk twice — the hydrated worktree copy
plus its chunks in .git/cdc/objects. Add `git cdc drop` to reclaim both. It
rewrites the worktree file back to its committed manifest (the same state a
fresh clone is in, which the smudge filter and `git cdc pull` already handle)
and, with --chunks, deletes local chunks the remote provably holds. Read
docs/local-drop/RESEARCH.md first — its findings are load-bearing.

Branch: local-drop  |  Base: local-drop (already checked out)

Two rules that are easy to get wrong and silently corrupt `git status` or lose
user data. Do not deviate from either:
  1. Never overwrite a worktree file whose blake3 hash does not equal the
     manifest's oid. That mismatch means uncommitted edits, which exist in NO
     store (the clean filter runs at `git add`, not on edit) and cannot be
     recovered. Skip with a warning instead.
  2. Write the index blob bytes VERBATIM (`git show :<path>`), never a
     re-encoded Manifest, and always `git add -- <path>` afterwards. Git
     distrusts the file-size drop and reports the file modified otherwise, and
     `git update-index --refresh` does NOT clear it.

Tasks:
- TASK-001: Add `Cmd::Drop { paths: Vec<String>, all: bool, chunks: bool,
  dry_run: bool }` to the Subcommand enum in crates/cli/src/main.rs (after Gc,
  before Diff), doc-commented like its neighbours, dispatched to
  sync::cmd_drop in main().
- TASK-002: Implement `pub fn cmd_drop(paths: &[String], all: bool, chunks:
  bool, dry_run: bool) -> Result<()>` in crates/cli/src/sync.rs. Select tracked
  files from index_manifests(): --all takes all; otherwise normalize each
  argument to repo-relative (canonicalize against repo_root() at git.rs:25, or
  delegate to `git ls-files --full-name -- <arg>`) so the command works from a
  subdirectory, match it against tracked paths, and bail! on an argument naming
  no tracked file. bail! with a usage message when neither --all nor any path
  is given.
- TASK-003: Add `remote_present(chunks: &HashSet<blake3::Hash>) ->
  Result<HashSet<blake3::Hash>>` in sync.rs returning which chunks the
  configured remote holds: Remote::Opendal and Remote::Ssh via one list()
  intersected with chunks (same shape as cmd_push at sync.rs:122 and
  sync.rs:135); Remote::Http via client.batch(Operation::Download, …), counting
  an object present when it has no `error` and carries a download action.
  Propagate the remote() error — unlike cmd_gc, drop must NOT proceed without a
  remote.
- TASK-004: When --chunks is set, resolve the remote and compute remote_present
  BEFORE touching any file, so a missing remote aborts with zero side effects.
  Exception: under --dry-run a missing remote degrades to a printed note and a
  zero exit. Candidate set = union of the selected manifests' chunk hashes that
  store.has().
- TASK-005: For each selected file: skip it if its worktree content is already
  manifest text (is_manifest, as cmd_pull checks at sync.rs:228); otherwise
  blake3-hash the file and compare against m.oid, skipping with a warning on
  mismatch (rule 1 above). For survivors, write the raw index blob over the
  file via `git show :<path>`, then `git add -- <path>` (rule 2 above).
  Accumulate m.size as bytes freed. Exit non-zero if every selected file was
  skipped for an oid mismatch.
- TASK-006: When --chunks is set, delete the chunks confirmed in TASK-004 with
  DiskStore::remove, and eprintln! a warning per chunk the remote lacks (name
  it with oid_str) — those stay. Skip all deletions under --dry-run.
- TASK-007: Print a summary in the `git-cdc: …` style of cmd_push/cmd_gc:
  files dropped, files skipped, chunks removed, chunks kept for lack of a
  remote copy, bytes freed; prefix with "would" under --dry-run.
- TASK-008: Extend the cmd_push error at crates/cli/src/sync.rs:86 and the
  matching with_context messages for the opendal and ssh remotes to mention
  `git cdc drop --chunks` as a cause of a missing local chunk, alongside the
  existing `git cdc pull` advice.
- TASK-009: Document it — a `git cdc drop` row in the table in
  docs/book/src/commands.md plus a short section on the two halves of local
  disk usage, `git cdc pull` as the rehydrate path, and the fact that files
  with uncommitted edits are skipped; a "Reclaiming local disk" note in
  README.md near the gc material; and a cross-link in docs/book/src/gc.md
  making the distinction explicit (gc sweeps *unreferenced* chunks, drop
  deletes *referenced* ones on purpose).
- TASK-010: Add e2e test `drop_reclaims_worktree_and_chunks` to
  crates/cli/tests/serverless.rs, following the existing
  serverless_push_clone_pull_gc setup (fs-scheme OpendalStore remote via
  cdc.store.scheme/option/prefix config). Track and commit a file, push, then
  `git cdc drop --chunks <path>` and assert: the worktree file is manifest text
  (starts with b"version git-cdc/spec/v1\n"), `git status --porcelain` is
  empty, the local store holds no chunks for it, and a following `git cdc pull`
  restores byte-identical content with `git status --porcelain` still empty.
- TASK-011: Add safety and selection coverage in crates/cli/tests/serverless.rs:
  (a) `git cdc drop` with neither --all nor paths exits non-zero; (b) `git cdc
  drop <untracked-path>` exits non-zero; (c) --dry-run leaves the worktree file
  and the local store untouched; (d) a chunk the remote lacks is kept, not
  deleted (delete one chunk from the fs-backed remote first); (e) --chunks with
  no remote configured exits non-zero AND leaves both the worktree file and the
  store untouched; (f) a file with uncommitted edits is skipped and its content
  survives verbatim; (g) `drop` invoked from a subdirectory with a
  cwd-relative path works.

Key files:
- crates/cli/src/main.rs — subcommand enum + dispatch (see Cmd::Gc at :52)
- crates/cli/src/sync.rs — cmd_drop, remote_present; existing cmd_pull (:157),
  cmd_gc (:251), and the per-remote listings in cmd_push (:120-148)
- crates/cli/src/git.rs — index_manifests (:132), local_store (:36),
  oid_str (:62), git_out (:13), repo_root (:25)
- crates/core/src/store/disk.rs — DiskStore::remove (:57), has, list
- crates/core/src/manifest.rs — Manifest.oid, Manifest.size, is_manifest
- crates/cli/tests/serverless.rs — e2e harness with an fs-backed remote
- crates/cli/tests/utils.rs — git/cdc/git_cmd helpers (git_cmd for non-zero exits)
- docs/book/src/commands.md, docs/book/src/gc.md, README.md — docs

Completion criteria: `cargo test -p git-cdc-cli` and `cargo clippy
--all-targets -- -D warnings` pass. In a repo with a configured remote and a
pushed tracked file: `git cdc drop --chunks <path>` shrinks the worktree file
to its manifest, leaves `git status --porcelain` empty, empties the file's
chunks from .git/cdc/objects, and a subsequent `git cdc pull` restores
byte-identical content with `git status --porcelain` still empty. Editing that
file without committing and re-running `git cdc drop` leaves the edit intact.

When done: git add -u && git commit -m "feat: add git cdc drop to reclaim local disk" — no Co-authored-by.
Reply with a one-paragraph summary and commit SHA.
Do NOT push, open PRs, or modify PLAN.md.
```

## 3. Testing

- [x] TEST-001: `drop_reclaims_worktree_and_chunks` in
      `crates/cli/tests/serverless.rs` — track, commit, push, `drop --chunks`,
      assert manifest-text worktree + clean `git status --porcelain` + no local
      chunks, then `pull` and assert byte-identical content and clean status.
      This is the one test that proves the whole feature.
- [x] TEST-002: `git cdc drop` with neither `--all` nor paths exits non-zero
      (use `utils::git_cmd`-style non-asserting invocation).
- [x] TEST-003: `git cdc drop <untracked-path>` exits non-zero rather than
      silently doing nothing.
- [x] TEST-004: `--dry-run` leaves both the worktree file and the local chunk
      store untouched.
- [x] TEST-005: A chunk absent from the remote is kept, not deleted — delete
      one chunk from the fs-backed remote store before running
      `drop --chunks`, then assert it still exists locally.
- [x] TEST-006: `--chunks` with no remote configured exits non-zero and leaves
      **both** the local store and the worktree file untouched (SEC-002 — the
      worktree assertion is what proves the abort happens before side effects).
- [x] TEST-007: A file with uncommitted edits is skipped and its content
      survives byte-for-byte, with a non-zero exit when it was the only
      selected file (SEC-003). This is the data-loss guard.
- [x] TEST-008: `git cdc drop <cwd-relative-path>` run from a subdirectory
      resolves the file (CON-004).

## 4. Risks & Assumptions

- **RISK-001**: `--chunks` on a chunk that exists nowhere else is
  unrecoverable data loss — mitigation: SEC-001's remote-presence gate, with
  TEST-005 and TEST-006 pinning it.
- **RISK-002**: Overwriting a file with uncommitted edits destroys work that
  exists in no store — mitigation: SEC-003's oid check, pinned by TEST-007.
- **RISK-003**: Re-encoding the manifest instead of writing the index blob
  verbatim would change the blob OID and stage a spurious modification —
  mitigation: CON-001 mandates `git show :<path>` bytes; TEST-001's clean
  `git status` assertion catches a regression.
- **RISK-004**: A user drops a file, then edits the manifest text by accident,
  and `git cdc clean` passes it through as a valid-looking manifest. This is
  pre-existing behaviour for fresh clones, not new to `drop` — out of scope.
- **ASSUMPTION-001**: Requiring explicit paths or `--all` is the right default
  (RESEARCH open question). Resolved toward safety; a bare `git cdc drop` that
  nukes every hydrated file in the repo is too easy to run by mistake.
- **ASSUMPTION-002**: Inherited from RESEARCH ASSUMPTION-001 — the `git add`
  stat-cache refresh was verified on git 2.39.5 and is assumed to hold on every
  git the project supports (≥ 2.11, implied by `filter-process`).
- **ASSUMPTION-003**: `git cdc drop` needs no `git cdc undrop` — `git cdc pull`
  already rehydrates from exactly this state and is documented as such.
- **ASSUMPTION-004**: Phase 1 carries 11 tasks, above the ~5 the planning
  guidance suggests. Accepted for `lite` mode, which mandates a single phase
  (PLAN-REVIEW SUGGEST-002); the natural split if this is re-run at `full` is
  command + safety gate, then docs + tests.
