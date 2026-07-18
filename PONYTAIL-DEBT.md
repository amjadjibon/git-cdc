# Ponytail Debt Ledger

Deliberate shortcuts marked with `ponytail:` comments in the source. Each
names its ceiling (the limit it holds until) and the trigger to revisit.
Regenerate with `grep -rnE '(#|//) ?ponytail:' .` (skip `target/`,
`docs/book/book/`).

Last scan: 2026-07-19 · 1 marker, 0 with no trigger.

| Where | Shortcut | Ceiling | Upgrade trigger |
| ----- | -------- | ------- | --------------- |
| `crates/server/src/lib.rs` (gc handler) | GC grace trusts store timestamps (disk mtime / S3 `LastModified`) | A skewed store clock could sweep early or retain garbage | GC runs against a store this team doesn't control, or a sweep deletes a chunk younger than its grace → switch to server-recorded upload times |

## Resolved

- ~~Sequential HTTP push uploads~~ — bounded concurrency (4 workers pulling
  from a shared index), 2026-07-19. S3 push remains sequential by choice:
  its negotiation is one listing, and the serverless path is typically
  same-region/local where round-trips don't dominate.
- ~~amd64-only server image~~ — native arm64 runner (`ubuntu-24.04-arm`) +
  manifest merge in the release workflow, 2026-07-19.

## Related deferred scope (documented, unmarked)

Not `ponytail:` markers, but deliberate v2 deferrals recorded in
`docs/git-cdc-mvp/DESIGN.md` and the book's Development chapter: git
filter-process protocol, transfer adapters / pre-signed URL offload, SSH
transport, compression, per-branch ACLs via the batch `ref` field,
constant-time token comparison.
