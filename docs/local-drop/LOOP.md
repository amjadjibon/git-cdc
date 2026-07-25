---
feature: local-drop
task: create a command that will delete the local file but keep track of the cdc hash, to reduce local storage usage
branch: local-drop
mode: lite
started: 2026-07-25
max_iterations: 2
max_phases: 1
max_agents: 2
current_iteration: 3
status: complete
last_review_base: '81602bb'
---

# Dev Loop: local-drop

## Iterations

| Iter | Phase | Verdict | Crit | High | Med | Low | Mode | Action |
|------|-------|---------|------|------|-----|-----|------|--------|
| 1    | 1     | Request Changes | 0 | 0 | 3 | 3 | lite | §3.C.1 direct fix |
| 2    | 1     | Approve | 0 | 0 | 0 | 0 | lite | Clean exit — awaiting approval |
| 3    | 1     | —       | —    | —    | —   | —   | lite | Post-approval follow-ups (see below) |

## Stacked PRs

| Phase | Branch | PR URL | Base | Status |
|-------|--------|--------|------|--------|
| 1     | local-drop | https://github.com/amjadjibon/git-cdc/pull/10 | main | open |

## Active Worktrees

| Worktree path | Branch | Purpose | Status |
|---------------|--------|---------|--------|

## Log

### Iteration 1
- [x] dev-implement-plan
- [x] dev-qa
- [x] dev-code-review — Request Changes (3 Medium, 3 Low)
- [x] decide — §3.C.1 direct fix on the branch

### Iteration 2
- [x] fix — all 6 findings, each Medium with a regression test verified to
      fail against the old code
- [x] dev-code-review — Approve
- [x] decide — Clean Exit, awaiting user approval to push and open a PR

### Iteration 3 — post-review follow-ups requested by the user
Each landed with tests verified non-vacuous by disabling the feature under test.
- [x] smoke check `scripts/test-git-cdc-drop.sh` (`5dd2031`) — running it caught
      its own flaky chunk-count assertion, fixed by sizing the fixture so
      >= 2 chunks is arithmetic rather than probability
- [x] `settle_index` after `pull` (`b9de348`) — the INFO-001 wart, fixed after
      measurement disproved the cost argument that had left it open. Uses
      `update-index --cacheinfo` + `--refresh` (stat-only) rather than
      `git add`, which was verified to silently stage a re-chunked manifest
- [x] chunk-config divergence note in `pull` (`b884981`)
- [x] `clean` inherits a tracked file's committed chunk parameters (`9f84add`),
      making divergence self-healing; reworded the `b884981` note, which the
      new behaviour had made inaccurate
- [x] decide — approved by the user, pushing and opening the PR

Bootstrap notes: RESEARCH.md and PLAN.md predate this loop (run via
/dev-research and /dev-create-plan). dev-review-plan returned **Blocked**
(BLOCK-001: overwriting uncommitted edits is unrecoverable data loss); the plan
was revised to v1.1 resolving BLOCK-001, REVISE-001/002/003 and SUGGEST-001
before the loop started.
