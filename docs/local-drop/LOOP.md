---
feature: local-drop
task: create a command that will delete the local file but keep track of the cdc hash, to reduce local storage usage
branch: local-drop
mode: lite
started: 2026-07-25
max_iterations: 2
max_phases: 1
max_agents: 2
current_iteration: 1
status: running
last_review_base: ''
---

# Dev Loop: local-drop

## Iterations

| Iter | Phase | Verdict | Crit | High | Med | Low | Mode | Action |
|------|-------|---------|------|------|-----|-----|------|--------|
| 1    | 1     | —       | —    | —    | —   | —   | lite | —      |

## Stacked PRs

| Phase | Branch | PR URL | Base | Status |
|-------|--------|--------|------|--------|
| 1     | local-drop | — | main | pending |

## Active Worktrees

| Worktree path | Branch | Purpose | Status |
|---------------|--------|---------|--------|

## Log

### Iteration 1
- [ ] dev-implement-plan
- [ ] dev-qa
- [ ] dev-code-review
- [ ] decide

Bootstrap notes: RESEARCH.md and PLAN.md predate this loop (run via
/dev-research and /dev-create-plan). dev-review-plan returned **Blocked**
(BLOCK-001: overwriting uncommitted edits is unrecoverable data loss); the plan
was revised to v1.1 resolving BLOCK-001, REVISE-001/002/003 and SUGGEST-001
before the loop started.
