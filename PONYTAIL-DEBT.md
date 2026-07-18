# Ponytail Debt Ledger

Deliberate shortcuts marked with `ponytail:` comments in the source. Each
names its ceiling (the limit it holds until) and the trigger to revisit.
Regenerate with `grep -rnE '(#|//) ?ponytail:' .` (skip `target/`,
`docs/book/book/`).

Last scan: 2026-07-19 · 3 markers, 1 with no trigger.

| Where | Shortcut | Ceiling | Upgrade trigger |
| ----- | -------- | ------- | --------------- |
| `crates/core/src/bin/git-cdc.rs:405` | HTTP push uploads chunks sequentially | Transfer time dominated by round-trips on high-latency links | A real repo shows round-trips dominating → bounded concurrency |
| `crates/server/src/lib.rs:198` | GC grace trusts store timestamps (disk mtime / S3 `LastModified`) | A skewed store clock could sweep early or retain garbage | ⚠️ `no-trigger` — "fine for MVP"; suggested: revisit if GC runs against a store not owned by the same team, or a sweep ever deletes a chunk younger than its grace |
| `.github/workflows/release.yml:52` | Server image is linux/amd64 only | arm64 hosts (Apple Silicon, Graviton) emulate or can't run it | Someone needs arm64 → native `ubuntu-24.04-arm` job + manifest merge |

## Related deferred scope (documented, unmarked)

Not `ponytail:` markers, but deliberate v2 deferrals recorded in
`docs/git-cdc-mvp/DESIGN.md` and the book's Development chapter: git
filter-process protocol, transfer adapters / pre-signed URL offload, SSH
transport, compression, per-branch ACLs via the batch `ref` field,
constant-time token comparison.
