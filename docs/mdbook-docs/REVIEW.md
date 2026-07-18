---
date: 2026-07-18
feature: mdbook-docs
diff: main...mdbook-docs
verdict: Approve
---

# Code Review: mdbook-docs

## Findings

- **LOW-001 (accepted)** — the spec chapter's `{{#include ../../spec/manifest.md}}`
  couples the book to the spec file's location; a future move breaks the
  build loudly (mdbook errors), which is the desired failure mode.
- **LOW-002 (accepted)** — no CI publishes the book (no CI exists in the
  repo); `mdbook serve docs/book` is the consumption path for now. GitHub
  Pages deploy is a natural follow-up when CI lands.
- **INFO-001** — chapters duplicate some README prose deliberately (README
  stays a self-contained front page; the book is the deep guide).

## Machine-Readable Verdict

```yaml
verdict: Approve
critical: 0
high: 0
medium: 0
low: 2
```
