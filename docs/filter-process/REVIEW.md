---
date: 2026-07-19
feature: filter-process
diff: main...filter-process
verdict: Approve
---

# Code Review: filter-process

## Findings

- **LOW-001 (accepted)** — `cmd_filter_process` buffers each file's
  *output* in memory before answering (so a mid-stream failure can become
  a clean per-file `status=error` instead of a truncated success). Peak
  memory = largest tracked file during smudge. The fully-streaming
  alternative gives up per-file error recovery; revisit if multi-GB
  tracked files become the norm.
- **LOW-002 (accepted)** — smudge inside the process still reads the
  manifest and chunks per file with no cross-file chunk cache; the shared
  store handle already avoids re-opening, and chunks are read once each in
  practice.
- **INFO-001** — capability response omits `delay`; git degrades to
  synchronous smudge per protocol. Deferred deliberately.
- **INFO-002** — auth now hashes both header and expectation per request
  (two BLAKE3 calls of <100 bytes — nanoseconds; correctness over
  micro-optimization).

## Machine-Readable Verdict

```yaml
verdict: Approve
critical: 0
high: 0
medium: 0
low: 2
```
