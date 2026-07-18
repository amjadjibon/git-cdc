---
date: 2026-07-18
feature: mdbook-docs
---

# QA Report: mdbook-docs

Docs-only feature — QA is build health + factual accuracy against the code.

- `mdbook build docs/book` — clean, no missing-file warnings; the
  `{{#include}}` of the spec resolved (version identifier present 3× in
  rendered spec.html).
- `mdbook test docs/book` — passes (no runnable rust blocks; command blocks
  are `sh`/`text`).
- Accuracy spot-checks against source: command set matches the `Cmd` enum;
  server flag table matches `Args` in `crates/server/src/main.rs`;
  chunk-size ranges match `ChunkParams::validate`; API routes match
  `app()`; S3 config keys match `remote()`; region fallback and
  foreign-object-skip claims match `store/s3.rs`.
- Build output `docs/book/book/` gitignored; `git status` clean post-build.
