---
goal: Remove genuine code duplication across the git-cdc workspace
version: 1.0
date_created: 2026-07-22
last_updated: 2026-07-22
owner: amjadjibon
status: 'Planned'
tags: [refactor]
---

# Remove Code Duplication

![Status: Planned](https://img.shields.io/badge/status-Planned-blue)

A senior-architect pass over the git-cdc workspace (crates: `cli`, `core`, `server`) to
eliminate confirmed copy-pasted code: a byte-identical test fixture, a 6-times-duplicated
PRNG helper, a ~120-line e2e test harness repeated across 4 files, near-identical
S3/SSH branches in `sync.rs`, and a small duplicated auth-client builder in server tests.
Pure refactor — behavior must not change.

## 1. Requirements & Constraints

- **REQ-001**: Every duplicate definition is deleted, not left alongside a new shared one.
- **REQ-002**: `cargo test --workspace`, `cargo fmt --all --check`, and
  `cargo clippy --workspace --all-targets -- -D warnings` all pass after every task.
- **CON-001**: No new crates and no new external dependencies — all fixes use `#[path]`
  module includes, a `pub` export, or a small extracted function/module within existing
  crates.
- **CON-002**: `sync.rs`'s S3 branch is async (`rt.block_on`) and its SSH branch is sync
  (no runtime) — do not force them behind one trait/closure abstraction spanning both;
  extract the shared *pure decision logic* (what to keep/delete/upload) and let each
  branch keep its own I/O calls.

## 2. Implementation Steps

> After each task: `git add -u` and commit. No `Co-authored-by:`. Tick `[x]` as each task completes.

### Phase 1: Remove duplication

**Goal**: delete all five confirmed duplications, safest/most mechanical first, riskiest
(`sync.rs`) last.

> **Mid-flight scope change**: partway through TASK-001 the user separately directed
> removing the dedicated S3 backend/remote entirely (server's `--backend s3`, CLI's
> `Remote::S3` + `cdc.s3.*` config) in favor of the already-generic OpenDAL path, since
> S3 is just one OpenDAL scheme among many already supported. That superseded the
> shared `test-support/s3_fixture.rs` this task had just created — both fixture copies,
> `crates/server/tests/s3_backend.rs`, and `OpendalConfig::s3` are now deleted rather
> than deduplicated, and `e2e_serverless.rs` was rewritten to exercise the generic
> `cdc.opendal.*` remote against the `fs` scheme instead of a fake S3 server. TASK-005's
> `Remote::S3` branches became `Remote::Opendal` accordingly.

- [x] TASK-001: Deduplicate the S3 test fixture. Move
  `crates/cli/tests/s3_fixture/mod.rs` to a new workspace-level file
  `test-support/s3_fixture.rs` (plain file, not a crate — no `Cargo.toml`). Delete
  `crates/server/tests/s3_fixture/mod.rs`. In `crates/cli/tests/e2e_serverless.rs` and
  `crates/server/tests/s3_backend.rs`, replace `mod s3_fixture;` with
  `#[path = "../../../test-support/s3_fixture.rs"] mod s3_fixture;` (adjust the relative
  `../` count to actually resolve from each crate's `tests/` directory — verify with
  `cargo test -p git-cdc-cli --test e2e_serverless` and
  `cargo test -p git-cdc-server --test s3_backend`). Remove the now-empty
  `crates/cli/tests/s3_fixture/` and `crates/server/tests/s3_fixture/` directories.

- [x] TASK-002: Deduplicate the `test_data` PRNG helper. In
  `crates/core/src/chunker.rs`, add an unconditionally-compiled `pub mod test_util { pub
  fn test_data(len: usize, seed: u64) -> Vec<u8> { ... } }` (same 11-line xorshift body
  currently at chunker.rs:111-121, `#[cfg(test)] pub(crate) mod tests`) — not
  `cfg(test)`-gated, since consumers in other crates' integration-test binaries don't
  share this crate's `cfg(test)` and an 11-line pure byte generator costs nothing to
  ship in the normal build. Update `crates/core/src/chunker.rs`'s own test module (and
  any use in `manifest.rs`/`envelope.rs` tests) to call `test_util::test_data` instead of
  the old `pub(crate)` copy, then delete that copy. Update
  `crates/core/benches/core.rs:15-25`, `crates/cli/tests/e2e_filter.rs:70-80`,
  `e2e_full.rs:103-113`, `e2e_serverless.rs:78-88`, `e2e_ssh.rs:79-89` to delete their
  local `test_data` and call `git_cdc_core::chunker::test_util::test_data` (confirm the
  exact module path compiles — `chunker` must be a public module of `git_cdc_core` for
  this path to resolve from other crates; check `crates/core/src/lib.rs` and adjust the
  path/visibility if `chunker` is private, e.g. re-export via
  `pub use chunker::test_util` from `lib.rs`).

- [x] TASK-003: Deduplicate the e2e test harness. Create
  `crates/cli/tests/support/mod.rs` with shared `git()`, `cdc()`, and
  `base_setup_repo()` extracted from the near-identical definitions in
  `crates/cli/tests/e2e_full.rs:39-101`, `e2e_serverless.rs:16-76`, `e2e_ssh.rs:14-77`,
  and the lighter variant in `e2e_filter.rs:10-68` — read all four first to size the
  shared surface accurately (e.g. `git()`'s optional PATH-prepend param, `cdc()`'s
  stderr-on-success return, `base_setup_repo()` installing the filter + wiring
  `filter.cdc.{clean,smudge,process}` to `$BIN` with no remote config). Each of the four
  files adds `mod support;` (or `#[path = "support/mod.rs"] mod support;`) and keeps only
  its own 2-3 lines of remote-specific config (http token / s3 config / ssh command) on
  top of `support::base_setup_repo()`. Delete the four duplicated definitions.

- [x] TASK-004: Deduplicate the server test auth-client builder. Create
  `crates/server/tests/support.rs` (or extend one if TASK-003's pattern suggests a
  shared name) exporting a `client()` function building the bearer-auth
  `reqwest::Client` currently duplicated at `crates/server/tests/integration.rs:23-30`
  and inline at `opendal_backend.rs:96-101`. Both files add `mod support;` (or
  `#[path]`) and call `support::client()`, deleting their own copies.

- [x] TASK-005: Deduplicate `sync.rs`'s S3/SSH branches. Read
  `crates/cli/src/sync.rs` in full first. In `cmd_push` (S3 branch: lines ~76-92, SSH
  branch: ~93-112), extract a pure function, e.g. `fn pending_uploads(chunks:
  &HashMap<blake3::Hash, u64>, present: &HashSet<blake3::Hash>) -> Vec<blake3::Hash>`
  (the "what's missing from `present`" diff, currently duplicated inline), and call it
  from both branches — each branch still does its own `s3.list().await?` /
  `ssh.list()?` and its own upload loop (async vs sync I/O stays separate; only the pure
  diff logic is shared). In `cmd_gc` (S3 branch: ~261-288, SSH branch: ~289-313),
  extract a pure function, e.g. `enum SweepAction { KeepLive, KeepGrace, Delete }` plus
  `fn sweep_decision(live: &HashSet<blake3::Hash>, hash: &blake3::Hash, modified:
  Option<SystemTime>, grace: Duration, now: SystemTime) -> SweepAction` (the
  live-check + grace-age check currently duplicated inline in both branches, and again
  in `cmd_gc`'s local-store sweep at lines ~220-241 — reuse it there too if the local
  sweep's shape lines up once written), and call it from all three sweeps, keeping each
  branch's own `.remove()`/counting call. Do not touch the `Remote::Http` branches
  (server-owned GC/batch protocol — genuinely different shape, not duplication).

**Completion criteria**: `cargo test --workspace` passes, `cargo fmt --all --check` is
clean, `cargo clippy --workspace --all-targets -- -D warnings` is clean, and `grep -rn
"fn test_data" crates/` returns exactly one definition (in
`crates/core/src/chunker.rs`'s `test_util` module); the two `s3_fixture/mod.rs` files
and the four duplicated e2e harnesses no longer exist as separate copies.

**git commit**: `git add -u && git commit -m "refactor: remove duplicated test fixtures, harness, and sync.rs branches"`

**Agent Prompt**:
```
You are a sub-agent implementing Phase 1 (the whole feature — lite mode) of
remove-code-duplication.

Context: git-cdc is a small Rust workspace (crates/cli, crates/core, crates/server).
This phase removes five confirmed code duplications without changing any behavior.

Branch: remove-code-duplication  |  Base: main

Tasks:
- TASK-001: dedupe crates/cli/tests/s3_fixture/mod.rs vs crates/server/tests/s3_fixture/mod.rs
  (byte-identical, 60 lines) into a shared test-support/s3_fixture.rs included via
  #[path] from both crates/cli/tests/e2e_serverless.rs and crates/server/tests/s3_backend.rs.
  Delete both original copies and their now-empty directories.
- TASK-002: dedupe the 11-line xorshift `test_data(len: usize, seed: u64) -> Vec<u8>`
  helper duplicated 6x (crates/core/src/chunker.rs:111-121, crates/core/benches/core.rs:15-25,
  crates/cli/tests/e2e_filter.rs:70-80, e2e_full.rs:103-113, e2e_serverless.rs:78-88,
  e2e_ssh.rs:79-89) into an unconditionally-compiled `pub mod test_util` in chunker.rs
  (not cfg(test)-gated — other crates' test binaries don't share this crate's cfg(test)).
  Update chunker.rs's own tests plus all 5 external call sites to use
  git_cdc_core::chunker::test_util::test_data (re-export from lib.rs if chunker is
  private), deleting every duplicate definition.
- TASK-003: dedupe the ~120-line e2e harness (git(), cdc(), setup_repo()) duplicated
  across crates/cli/tests/e2e_full.rs:39-101, e2e_serverless.rs:16-76, e2e_ssh.rs:14-77,
  and the lighter e2e_filter.rs:10-68, into crates/cli/tests/support/mod.rs — shared
  git()/cdc()/base_setup_repo(), each file keeping only its 2-3 lines of remote-specific
  config.
- TASK-004: dedupe the bearer-auth reqwest client() builder duplicated at
  crates/server/tests/integration.rs:23-30 and inline in opendal_backend.rs:96-101 into
  crates/server/tests/support.rs, both files calling support::client().
- TASK-005: read crates/cli/src/sync.rs fully, then dedupe cmd_push's S3/SSH branches
  (~76-92, ~93-112) via an extracted pure `pending_uploads(...)` diff function, and
  cmd_gc's S3/SSH branches (~261-288, ~289-313, and reuse for the local sweep at
  ~220-241 if it fits) via an extracted pure `sweep_decision(...)` function returning a
  KeepLive/KeepGrace/Delete enum. Each remote keeps its own I/O calls — only the pure
  decision logic is shared. Do not touch the Remote::Http branches.

Key files: crates/cli/tests/s3_fixture/mod.rs, crates/server/tests/s3_fixture/mod.rs,
crates/core/src/chunker.rs, crates/core/benches/core.rs, crates/cli/tests/e2e_*.rs,
crates/server/tests/integration.rs, crates/server/tests/opendal_backend.rs,
crates/cli/src/sync.rs

Completion criteria: cargo test --workspace passes; cargo fmt --all --check clean;
cargo clippy --workspace --all-targets -- -D warnings clean; grep -rn "fn test_data"
crates/ returns exactly one definition; no duplicate s3_fixture/harness/client()
copies remain.

When done: git add -u && git commit -m "refactor: remove duplicated test fixtures, harness, and sync.rs branches" — no Co-authored-by.
Reply with a one-paragraph summary and commit SHA.
Do NOT push, open PRs, or modify PLAN.md.
```

## 3. Testing

- [ ] TEST-001: `cargo test --workspace` — every existing test (unit + integration +
  e2e) still passes unchanged; this is a refactor, so no new test cases are required,
  but none may be deleted or weakened.
- [ ] TEST-002: `cargo fmt --all --check` and
  `cargo clippy --workspace --all-targets -- -D warnings` both clean.
- [ ] TEST-003: `grep -rn "fn test_data" crates/` returns exactly one hit; `find
  crates -iname "s3_fixture" ` shows only the files under the new shared location (no
  duplicate directories left in `cli/tests` and `server/tests` simultaneously).

## 4. Risks & Assumptions

- **RISK-001**: `#[path]` includes are relative to the file they're written in, not the
  crate root — a wrong `../` count silently fails to compile. Mitigation: verify each
  new `#[path]` module with a scoped `cargo test -p <crate> --test <file>` before moving
  to the next task.
- **RISK-002**: extracting `sweep_decision`/`pending_uploads` in `sync.rs` could
  silently change GC/push semantics (e.g. off-by-one in grace-period comparison).
  Mitigation: existing integration tests (`gc_deletes_orphans_past_grace_only`,
  `gc_grace_survives_skewed_store_clock`, `gc_accepts_large_live_sets`, S3/SSH e2e
  suites) already cover this — treat any change in their outcome as a bug, not an
  acceptable refactor side effect.
- **ASSUMPTION-001**: this is pure internal test/CLI plumbing with no external API
  surface — none of the five duplications are part of a published crate's public API,
  so consolidating them carries no compatibility risk.
- **ASSUMPTION-002**: `test-support/s3_fixture.rs` as a workspace-root plain file (not a
  crate) is acceptable — simplest option that avoids a new `Cargo.toml`/dev-dependency
  wiring for a 60-line helper.
