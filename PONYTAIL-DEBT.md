# Ponytail Debt Ledger

Deliberate shortcuts marked with `ponytail:` comments in the source. Each
names its ceiling (the limit it holds until) and the trigger to revisit.
Regenerate with `grep -rnE '(#|//) ?ponytail:' .` (skip `target/`,
`docs/book/book/`).

Last scan: 2026-07-19 · 0 markers. Clean ledger.

(no open markers)

## Resolved

- ~~GC grace trusted store timestamps~~ — server now records upload times
  on its own clock and GC prefers them; store timestamps only cover chunks
  from before a restart, and a regression test backdates a chunk's mtime a
  year to prove a skewed store clock can't defeat the grace. Serverless-CLI
  bucket sweeps still use `LastModified` (there is no server to record
  times) — the CLI's `--grace-secs` remains the guard there. 2026-07-19.

- ~~Sequential HTTP push uploads~~ — bounded concurrency (4 workers pulling
  from a shared index), 2026-07-19. S3 push remains sequential by choice:
  its negotiation is one listing, and the serverless path is typically
  same-region/local where round-trips don't dominate.
- ~~amd64-only server image~~ — native arm64 runner (`ubuntu-24.04-arm`) +
  manifest merge in the release workflow, 2026-07-19.

## Related deferred scope (documented, unmarked)

Not `ponytail:` markers, but deliberate v2 deferrals recorded in
`docs/git-cdc-mvp/DESIGN.md` and the book's Development chapter: git
transfer adapters / pre-signed URL offload, SSH
transport, compression, per-branch ACLs via the batch `ref` field,
constant-time token comparison.
