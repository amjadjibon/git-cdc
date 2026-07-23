---
date: 2026-07-22
plan: docs/remove-code-duplication/PLAN.md
plan_version: 1.0
reviewer: Claude
verdict: Ready
---

# Plan Review: remove-code-duplication

## Verdict
**Ready** — tasks are concrete (file:line-level), ordered safest-to-riskiest, completion criteria are runnable commands, and TASK-005 explicitly extracts only pure decision logic rather than forcing one abstraction across the async-S3/sync-SSH split (CON-002).

## Findings

### [SUGGEST-001] Shared test-support modules will trip `dead_code` under `-D warnings`
**Phase**: 1 (TASK-003, TASK-004)
**Issue**: every test binary in `tests/` compiles `mod support;` independently. A binary that uses only `git()` but not `base_setup_repo()` (e.g. `e2e_filter`) makes the unused function a `dead_code` warning in that binary — and the done-when requires `clippy -D warnings` clean, so this fails the gate on first compile.
**Fix**: annotate the shared support items (or the module) with `#[allow(dead_code)]` — standard practice for shared integration-test helpers — or `pub use` everything and accept per-binary allows. Same applies to `crates/server/tests/support.rs` if `s3_backend.rs` doesn't use `client()`.

### [SUGGEST-002] Verify `sweep_decision` reuse for the local sweep doesn't change the mtime-error path
**Phase**: 1 (TASK-005)
**Issue**: the local sweep gets mtime via `fs::metadata(...).and_then(modified).ok()` (an `Option`), and remote sweeps get `Option<SystemTime>` from `list()`. Shapes line up, but note the semantics: today a missing/unreadable mtime means *keep* (not old enough) in all three sweeps. The extracted function must preserve "no mtime → keep" or GC behavior silently changes.
**Fix**: keep the `is_some_and(|age| age >= grace)` shape inside `sweep_decision` so `None` mtime maps to `KeepGrace`, and rely on the existing `gc_grace_survives_skewed_store_clock` test as the guard.

## What's Good
- Findings are pre-verified with exact file:line ranges — no speculative "scan for duplication" task an agent could wander on.
- CON-002 preempts the classic refactor trap here (unifying async and sync I/O behind one trait for two call sites).
- Completion criteria include a grep-based structural check (`fn test_data` appears exactly once), not just "tests pass".

## Machine-Readable Verdict

```yaml
verdict: Ready
block: 0
revise: 0
suggest: 2
blocking_ids: []
```
