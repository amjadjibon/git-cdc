---
date: 2026-07-17
feature: branching-behavior
status: Concluded
recommendation: Nothing to build — branching already works through git's own mechanics; chunks dedup across branches by construction.
---

# Research: how git-cdc handles branching

## Question

1. What happens to chunks/manifests when work spans multiple branches?
2. Does switching branches restore correct content, including when chunks
   aren't local?
3. Do `push` and `gc` account for all branches?

Interpreted from "how does hand branching?" — assumed to mean git branching
behavior (ASSUMPTION-001).

## Recommendation

**Do nothing** — branching needs no git-cdc-specific machinery. A branch is
just refs pointing at commits whose tracked blobs are manifests; the chunk
store is branch-agnostic (content-addressed), so identical content on two
branches is stored once, and all sync/GC paths already walk every ref.

## Findings

All verified by spike (scratch repo, `main` + `feature` with a 1 KiB edit to
an 8 MiB file; spike dir deleted):

- **FIND-001 — chunks dedup across branches automatically.** The two branch
  versions produced 4 manifest chunks each but only **5 distinct chunks** in
  the store (3 shared, 1 unique per branch). Nothing branch-aware exists in
  the store — CAS keying by BLAKE3 does it (`crates/core/src/store.rs`).
- **FIND-002 — branch switch = ordinary smudge.** `git checkout <branch>`
  rewrites the worktree from that branch's manifest; with chunks local, both
  directions restored byte-identical content (distinct checksums verified).
- **FIND-003 — switching without local chunks degrades safely.** With the
  local store wiped, `git checkout main` succeeds, leaves manifest text in
  the worktree, and prints ``run `git cdc pull` to fetch file content``
  (`cmd_smudge` passthrough, `crates/core/src/bin/git-cdc.rs`). `pull` then
  fetches only that branch's missing chunks — this is exactly the
  fresh-clone path already covered by `e2e_full`.
- **FIND-004 — push and gc are all-branch by construction.** `all_manifests`
  enumerates `git rev-list --all --objects` (every ref: branches, tags,
  remote-tracking), so `push` uploads chunks reachable from any branch and
  the GC live set includes every branch — deleting a branch is what makes
  its unique chunks collectable (verified in `e2e_serverless`, where GC only
  swept v2 chunks after the ref stopped referencing them).
- **FIND-005 — the server/bucket has no branch concept at all.** The batch
  protocol carries an optional `ref` field (accepted, logged) reserved for
  per-branch ACLs — declared v2 in the MVP plan; it has no effect today.

## Assumptions

- **ASSUMPTION-001**: "hand branching" means git branching behavior — if you
  meant something else (e.g. chunk-boundary "branching" in FastCDC, or
  branch-scoped access control), say so and I'll research that instead.

## Open Questions

None for current behavior. If per-branch ACLs ("only CI pushes chunks
reachable from main") become a requirement, that's the existing `ref` field
plus server policy — a small planned feature, not a redesign.
