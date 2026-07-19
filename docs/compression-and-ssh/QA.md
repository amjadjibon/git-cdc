---
date: 2026-07-19
feature: compression-and-ssh
---

# QA Report: compression-and-ssh

## Tests Added

- `envelope` unit suite (5): compressible round trip (<10% of raw),
  incompressible stays raw (+1 byte), legacy bare bytes decode (including
  legacy bytes that *look* like an envelope — hash wins first), corrupt
  payload / unknown tag / empty buffer all rejected, empty chunk.
- `disk::legacy_bare_chunk_files_still_read` — planted pre-envelope store
  file reads; legacy raw wire body accepted by `put_encoded`.
- `e2e_ssh::ssh_push_clone_pull_gc` — the full cycle over the stdio
  protocol via `cdc.ssh.command` (identical code path to real ssh):
  dedup on second push (≤2 chunks for a 1-byte edit), fresh-clone
  passthrough, pull materialization, dry-run + real gc to the exact v1
  set.
- `e2e_ssh::compressible_content_stores_smaller_than_raw` — 8 MiB of
  repeated text stores < 10% of raw on the remote AND round-trips
  byte-identically after a full store wipe.

## Coverage by construction

Every pre-existing suite (11 targets) runs on the envelope format now —
HTTP e2e, in-process-S3 e2e, server integration (upload verification,
413s, GC grace/skew) — because the envelope is the universal storage and
wire representation. Old wire-shape tests pass through the legacy
hash-first rule, which is itself the old-client compatibility guarantee.

## Remaining Gaps

- Real sshd is not exercised (CI has no ssh loopback); `cdc.ssh.command`
  covers the protocol and process plumbing — the ssh binary itself only
  contributes transport. Manual test: any host with git-cdc installed.
- No compression escape hatch (`cdc.compression=none`) — deferred until
  someone is CPU-bound; the >5% heuristic covers the common cases.
