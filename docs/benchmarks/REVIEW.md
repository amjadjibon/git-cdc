---
date: 2026-07-19
feature: benchmarks
diff: main...benchmarks
verdict: Approve
---

# Code Review: benchmarks

## Findings

- **LOW-001 (accepted)** — `disk_store/put_2MiB_new` clones + rehashes the
  buffer per iteration inside the timed loop, inflating the measured put
  cost by one memcpy + hash (~0.6 ms of the 2.5 ms). Consistent across
  runs, so it's still a valid regression baseline; noted in case absolute
  numbers are ever quoted.
- **LOW-002 (accepted)** — benches accumulate files in a tempdir across
  iterations (`put` never cleans); a full criterion run writes ~100 MB of
  temp data, removed with the tempdir. Harmless.
- **INFO-001** — no CI bench run by design (timing noise on shared
  runners); clippy compile-checks the target.

## Machine-Readable Verdict

```yaml
verdict: Approve
critical: 0
high: 0
medium: 0
low: 2
```
