---
date: 2026-07-25
feature: local-drop
status: Concluded
recommendation: Add `git cdc drop [--chunks] [<path>...]` — rewrite the worktree file back to its index manifest, `git add` it to refresh the stat cache, and optionally delete local chunks that the remote provably has. `git cdc pull` is already the rehydrate path.
---

# Research: local-drop

## Question

1. What must a "delete the local file but keep the CDC hash" command actually
   delete to reduce local disk — the worktree copy, the local chunk store copy,
   or both?
2. Where is the hash kept, and does the repo need new state to track it?
3. What is the safety gate that stops the command from being data loss?

## Recommendation

**`git cdc drop [--chunks] [<path>...]`, a thin command over machinery that
already exists.** A tracked file costs disk twice: the hydrated worktree copy
plus the chunks in `.git/cdc/objects`. Dropping the worktree copy means writing
the file's *index manifest* over it — the exact state a fresh clone is in
(`crates/cli/src/filter.rs:52`), already round-tripped by `git cdc pull`
(`crates/cli/src/sync.rs:226`). No new on-disk state is needed: the hash is the
manifest, and the manifest is committed to git. `--chunks` additionally sweeps
the file's chunks from the local store, gated on the remote confirming it has
every one of them.

Cost: ~40 lines in `sync.rs` plus a subcommand. Main risk: `--chunks` on chunks
that exist nowhere else is unrecoverable data loss — hence the remote-presence
gate, which reuses the same listing `cmd_push` already does.

## Candidates

### A. `git cdc drop [--chunks] [<path>...]` — recommended

- **Fit**: Reuses `index_manifests()` (`git.rs:132`), the smudge passthrough
  state, `DiskStore::remove` (`store/disk.rs:57`), and the per-remote listing
  in `cmd_push` (`sync.rs:122,135`). Rehydrate is the existing `git cdc pull` —
  no new inverse command.
- **Cost**: One subcommand + one function. No new file format, no new config,
  no dependency.
- **Verified**: Spike below (FIND-001…004).

### B. Extend `git cdc gc` with a `--drop-worktree` flag — rejected

- **Why rejected**: `gc` sweeps *unreferenced* chunks; drop deletes
  *referenced* ones on purpose. Same flag, opposite invariant — mixing them
  makes the grace-period logic in `sweep_decision` (`sync.rs:38`) mean two
  different things.

### C. Do nothing — document `git show :<path> > <path>` — rejected

- **Why rejected**: Close, but incomplete on both ends. It leaves the file
  showing as modified in `git status` (FIND-002) and it does not reclaim the
  chunk-store half of the disk usage, which is the larger and less obvious
  half. Worth mentioning: it *is* the whole worktree half of the feature.

### D. `cdc.autoMaterialize=false` — smudge never hydrates — rejected as the answer

- **Why rejected**: Solves "never hydrate" (sparse checkout), not "reclaim
  space from files I already hydrated". Complementary future work, not a
  substitute. Not researched further.

## Findings

- **FIND-001**: A tracked file occupies disk twice. Spike with a 3,000,000-byte
  file: worktree 3,000,000 B + `.git/cdc/objects` 2.9 MiB. Rewriting the
  worktree file to its manifest took it to **254 B**; dropping local chunks too
  reclaims essentially all of it.
- **FIND-002 (load-bearing)**: After overwriting the worktree file with its
  manifest, **`git status` reports ` M <path>` and `git update-index --refresh`
  does not clear it** ("needs update"), even though the blob is byte-identical
  (`git hash-object -w --path=asset.bin asset.bin` == `git rev-parse
  :asset.bin` == `c8f67d3b…`). Git's stat-cache check sees the size drop
  (3,000,000 → 254) and refuses to trust it. Verified on git 2.39.5.
- **FIND-003**: **`git add <path>` clears it.** It re-runs the clean filter,
  which passes manifest text through unchanged (`filter.rs:26-31`), gets the
  same blob OID, and rewrites the index stat entry (`size: 254`). Nothing gets
  staged. So `drop` must `git add` each dropped path — cheap, since the filter
  only reads 254 bytes. Verified in the same spike.
- **FIND-004**: `git cdc pull` rehydrates from this exact state — it skips
  files whose worktree content is not manifest text and materializes the rest
  (`sync.rs:228-246`), re-fetching missing chunks first. Spike: `git cdc pull`
  restored the 3,000,000-byte file and left `git status` clean.
- **FIND-005**: All three remote kinds can answer "do you have this chunk"
  cheaply — `Remote::Opendal` and `Remote::Ssh` via one `list()`
  (`sync.rs:122,135`), `Remote::Http` via `client.batch(Operation::Download,…)`
  (`sync.rs:185`), which already reports per-object errors for chunks the
  server lacks. `--chunks` should reuse the listing shape of `cmd_push`.
- **FIND-006**: Remote presence is a *sufficient* safety gate on its own — no
  per-file chunk-liveness analysis is needed. Deduplication means a chunk may
  be shared with other files and other commits, but any chunk the remote holds
  is refetchable by `git cdc pull`, so "remote has it" covers every sharing
  case. `full.rs:265-280` already proves a chunk deleted locally is recoverable
  from the remote in a fresh clone.
- **FIND-007**: Plain `rm <path>` is not equivalent and must not be what the
  command does — git then reports the file deleted, and `git checkout --
  <path>` re-smudges the full content from local chunks, reclaiming nothing.
  The file must *contain the manifest*, not be absent.

## Assumptions

- **ASSUMPTION-001**: FIND-002/003 were verified on git 2.39.5 (Apple Git-154).
  The `git add` refresh is standard filter behaviour and should hold across
  versions — verify by re-running the spike on the oldest supported git
  (`filter-process` already implies ≥ 2.11) if CI ever pins an older git.
- **ASSUMPTION-002**: `--chunks` with no remote configured should refuse
  outright rather than delete. Not verified against user expectation — it is a
  design call for `dev-design`/`dev-create-plan`.

## Open Questions

- Default path set with no `<path>` arguments: every tracked file, or error out
  and require `--all`? (Recommend requiring explicit paths or `--all`;
  whole-repo destructive default is a footgun.)
- Should `--chunks` without a worktree drop be allowed (reclaims the store copy
  while keeping the file hydrated)? It is legitimate and free to support, but
  it is a second mode to document.
- Interaction with the pre-push hook: a repo full of dropped files still pushes
  fine (manifests are in git), but `cmd_push` hard-errors if the remote wants a
  chunk the local store no longer has (`sync.rs:86`). With the FIND-006 gate
  that cannot happen, but the error message should mention `git cdc drop`.
