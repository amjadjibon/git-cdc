---
date: 2026-07-18
feature: configurable-chunking
status: Concluded
recommendation: Read cdc.chunk.{min,avg,max} via `git config --type=int` in cmd_clean only; validate against fastcdc's hard bounds ourselves (the crate only debug_asserts); no server changes beyond raising the body limit to the 16 MiB protocol ceiling.
---

# Research: configurable chunking via gitconfig

## Findings

- **FIND-001 — fastcdc v2020 has hard parameter bounds.** `MINIMUM_MIN=64`,
  `MINIMUM_MAX=1 MiB` (min size); `AVERAGE_MIN=256`, `AVERAGE_MAX=4 MiB`;
  `MAXIMUM_MIN=1 KiB`, `MAXIMUM_MAX=16 MiB`. Current defaults (512 KiB /
  2 MiB / 8 MiB) sit inside all of them.
- **FIND-002 — the crate only `debug_assert`s these bounds.** A release build
  accepts out-of-range values and chunks incorrectly instead of panicking.
  git-cdc must validate: each value in range, and `min ≤ avg ≤ max`, with a
  clear error naming the offending key.
- **FIND-003 — `git config --type=int` parses size suffixes.** `512k`, `2m`,
  `1g` expand to bytes, so users write `git config cdc.chunk.avg 1m`.
- **FIND-004 — only `clean` chunks; everything else reads the manifest.**
  Manifests already carry `chunk-min/avg/max` headers, and smudge/pull/diff
  reassemble purely from chunk hashes. Old manifests stay valid whatever the
  config says — the config is read at chunking time only (`cmd_clean`).
- **FIND-005 — server body limit must cover the protocol ceiling.** The
  server currently caps request bodies at the compiled-in 8 MiB + slack. With
  configurable max up to fastcdc's 16 MiB, the server accepts up to
  16 MiB + slack unconditionally — the protocol ceiling, no flag needed.

## Caveats to document

- **Different configs → different manifests for identical content.** Two
  clients with different `cdc.chunk.*` re-cleaning the same file produce
  different (both valid) manifests, which shows up as spurious diffs. Teams
  should set the config once, repo-locally, for everyone (or leave defaults).
- Changing config never corrupts anything: oid is chunking-independent, and
  existing manifests re-clean via passthrough (content unchanged → filter
  not re-run on unmodified files anyway).

## Open Questions

None. Server-side per-chunk validation of declared sizes stays out of scope
(the server never re-chunks; it only stores hash-verified blobs).
