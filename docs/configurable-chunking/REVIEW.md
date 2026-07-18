---
date: 2026-07-18
feature: configurable-chunking
diff: main...configurable-chunking
verdict: Approve
---

# Code Review: configurable-chunking

## Findings

- **MED-001 (fixed in-loop)** — `/gc` was registered *after* the
  `DefaultBodyLimit` layer, so it kept axum's 2 MB default; a live set of
  ~100k oids (routine with small-chunk configs) is a >2 MB JSON body and
  would 413. Route moved under the layer; regression test
  `gc_accepts_large_live_sets` (100k oids, dry run, 200). Pre-existing bug,
  surfaced by this feature's review.
- **LOW-001 (accepted)** — with `filter.cdc.required` deliberately unset,
  invalid `cdc.chunk.*` config fails the clean filter loudly but git then
  stages the raw file. Consistent with the fresh-clone passthrough design;
  the error names the key (e2e-asserted).
- **LOW-002 (accepted)** — `chunk_params()` shells `git config` up to six
  times per clean; filters are one-shot processes, negligible against
  chunk hashing.
- **INFO-001** — e2e suites previously inherited the developer's global
  gitconfig (live-MinIO hit during this loop); now isolated via
  `GIT_CONFIG_GLOBAL/SYSTEM=/dev/null`.

## Machine-Readable Verdict

```yaml
verdict: Approve
critical: 0
high: 0
medium: 1   # fixed in-loop
low: 2      # accepted, documented
```
