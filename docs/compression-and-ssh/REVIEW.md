---
date: 2026-07-19
feature: compression-and-ssh
diff: main...compression-and-ssh
verdict: Approve
---

# Code Review: compression-and-ssh

## Findings

- **LOW-001 (accepted)** — the server's `put_chunk` decodes the envelope
  to verify (422 path) and `put_encoded` decodes again inside the backend
  (its own contract). Double decode of a ≤16 MiB body costs milliseconds;
  collapsing it would leak the handler's status-code concern into the
  store API.
- **LOW-002 (accepted)** — `cdc.ssh.command` is whitespace-split (no
  shell quoting), so chunk roots with spaces need the `cdc.ssh.remote` +
  `cdc.ssh.path` form. Documented as an advanced/test hook.
- **LOW-003 (accepted)** — ssh transfers are sequential (no equivalent of
  HTTP push's worker pool): the transport is one pipe; multiplexing it
  buys little against ssh's own windowing. Revisit with real-latency
  numbers if ssh push feels slow.
- **INFO-001** — compression level fixed at zstd-3; no config knob by
  design (YAGNI until a CPU-bound case appears).

## Machine-Readable Verdict

```yaml
verdict: Approve
critical: 0
high: 0
medium: 0
low: 3
```
