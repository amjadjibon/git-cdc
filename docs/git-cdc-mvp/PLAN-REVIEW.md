---
date: 2026-07-17
plan: docs/git-cdc-mvp/PLAN.md
plan_version: 1.0
reviewer: Claude
verdict: Needs Revision
resolution: All findings applied in PLAN.md v1.1 (2026-07-17)
---

# Plan Review: git-cdc-mvp

## Verdict

**Needs Revision** — structurally sound and properly scoped, but the fresh-clone flow contradicts the plan's own hard-error invariant and must be decided before Phase 4.

Previous review (of the unphased design doc, since renamed to DESIGN.md): both blocks and all four revises resolved in this version.

## Findings

### [REVISE-001] Fresh-clone checkout conflicts with "missing chunk = hard error"

**Phase**: 4 (task 4.3), interacts with 5.2
**Issue**: On a fresh clone the local store (`.git/cdc/objects`) is empty, so `git checkout` invokes `smudge`, which per 4.3 hard-errors on every missing chunk — the clone itself fails before `git cdc pull` can ever run. 5.2's `pull` assumes a completed checkout exists ("for current checkout's manifests"), so the two tasks deadlock each other. git-lfs solves this deliberately (pointer passthrough when content is absent, or on-demand download in smudge); the plan never picks a behavior.
**Fix**: Decide in 4.3: either (a) smudge writes the manifest text through to the worktree when chunks are missing and prints a "run `git cdc pull`" hint (do not set `filter.cdc.required`), with `pull` reading manifests from the index rather than the worktree — the lazy, LFS-proven option — or (b) smudge fetches missing chunks from `cdc.url` on demand. Add the fresh-clone case to the 5.4 e2e test either way.

### [SUGGEST-001] 5.2's manifest-enumeration sketch won't find historical manifests

**Phase**: 5 (task 5.2)
**Issue**: "`git rev-parse --all` + `git cat-file` on tracked paths" enumerates refs, not blobs; finding every manifest across history (which both `push` and `gc` need for correctness) requires walking objects. An implementer will discover this mid-task.
**Fix**: Specify `git rev-list --all --objects` piped to `git cat-file --batch`, identifying manifests by their fixed first line (`version https://git-cdc.dev/spec/v1`) rather than by path/attribute matching.

### [SUGGEST-002] No pre-push ordering guard

**Phase**: 4 (task 4.2) / 5
**Issue**: Nothing stops `git push` before `git cdc push`, publishing manifests whose chunks aren't on the server — the classic LFS footgun. git-lfs installs a pre-push hook during `install` for exactly this.
**Fix**: Either add a pre-push hook (running `git cdc push`) to `install` in 4.2, or explicitly accept manual ordering for MVP with a one-line note in the plan and a warning in `git cdc status`-adjacent output. Fine to defer the hook to v2 if the acceptance is written down.

## What's Good

- The MVP boundary and "decisions baked in from review" sections are explicit and binding — every previously deferred item (§13, §15.3–15.4, §16, refcounts, S3/Postgres) is named, not hand-waved.
- The `smudge(clean(x)) == x` invariant is tested at three distinct levels (unit 4.3, filter e2e 4.4, network e2e 5.4), and 5.4 asserts the actual dedup win (second push uploads only changed chunks), not just byte equality.
- Phase dependencies are correctly ordered and honest — Phase 4 depends only on Phase 1, which is right, and the `core`-vs-`git-cdc-core` Cargo naming trap is documented before someone hits it.

## Machine-Readable Verdict

```yaml
verdict: Needs Revision
block: 0
revise: 1
suggest: 2
blocking_ids: []
```
