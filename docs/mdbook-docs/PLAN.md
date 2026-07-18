---
status: Ready
version: 1.0
date: 2026-07-18
feature: mdbook-docs
---

# Plan: mdBook user documentation ("the git-cdc book")

An mdBook at `docs/book/` (rust-book style), sourced from what's already
written (README, spec, DESIGN.md) but reorganized as a book: narrative
first, reference later.

## Assumptions

- mdbook is available locally (verified: `~/.cargo/bin/mdbook`); CI is out
  of scope (no CI exists in this repo yet).
- The manifest spec chapter uses `{{#include}}` of `docs/spec/manifest.md`
  — one source of truth, no copy drift.
- Build output (`docs/book/book/`) is gitignored.

## Phase 1: skeleton + chapters

- [x] **1.1** `docs/book/book.toml` + `src/SUMMARY.md`: Introduction ·
  Getting Started · How It Works · Configuration · Serverless S3 ·
  Running a Server · Garbage Collection · Command Reference ·
  Manifest Spec (included) · Development.
- [x] **1.2** Write the chapters (substantive, reusing existing prose and
  examples; each chapter standalone-readable).
  *Done when*: `mdbook build docs/book` succeeds with no missing-file
  warnings.

## Phase 2: wire-up + docs

- [x] **2.1** Gitignore build output; README gets a "Documentation" section
  (`mdbook serve docs/book`).
  *Done when*: `git status` clean after a build; README links the book.

## Verification

```sh
mdbook build docs/book && mdbook test docs/book
```
