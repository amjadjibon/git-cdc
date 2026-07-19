---
date: 2026-07-19
feature: filter-process
---

# QA Report: filter-process

## Tests Added

- `pktline` unit suite (5): packet round trip, text-line newline framing,
  oversized write splits at 65516, `PktReader` streaming across packets and
  positioning exactly after the flush, malformed length rejection
  (`0001`, non-hex, > MAX_PAYLOAD).
- e2e `filter_process_alone_round_trips_many_files` — repo configured with
  ONLY `filter.cdc.process` (no fallback keys): 5 files (each > one 64 KiB
  packet, one spanning multiple chunks) added and checked out
  byte-identically; every byte provably crossed the pkt-line protocol.
- e2e `one_shot_filters_still_work_without_process` — process key unset →
  the git < 2.11 clean/smudge fallback path stays covered.

## Coverage by construction

The e2e fixtures (`e2e_filter`, `e2e_full`, `e2e_serverless`) now set
`filter.cdc.process`, and git prefers it — so the entire pre-existing
suite (byte-identical round trips, manifest passthrough, corrupt-store
refusal, chunk-size config, full network cycle, serverless cycle) runs
through the long-running filter against real git. A handshake or framing
bug fails ~15 tests instantly; the suite passed on first run after the
EOF-handling fix.

## Constant-time compare

Covered functionally by `auth_is_enforced` (valid, wrong, and missing
token). Timing behavior is by construction (`blake3::Hash` eq uses
`constant_time_eq`), not asserted by test — timing assertions in CI are
noise.

## Remaining Gaps

- `status=error` mid-file (corrupt store) through the process path is
  exercised by `smudge_never_emits_corrupt_data` (fixtures route it through
  the process filter), but the specific "other files continue after one
  errors" claim is implicit, not separately asserted.
- `delay` capability: not implemented (deferred by design).
