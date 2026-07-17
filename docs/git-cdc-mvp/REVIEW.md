---
date: 2026-07-17
feature: git-cdc-mvp
branch: git-cdc-mvp
diff: main...bc1a4b3
reviewer: Claude
iteration: 1
---

# Code Review: git-cdc-mvp

## Findings

### [MED-001] smudge buffers entire passthrough input in memory
**File**: `crates/core/src/bin/git-cdc.rs` (`cmd_smudge`)
**Issue**: `read_to_end` slurps all of stdin before the manifest check. For a
tracked path whose blob is *not* a manifest (file committed before tracking,
or `git cdc install` on a pre-existing repo), smudge passes through a
potentially multi-GB binary via a full in-memory copy — exactly what the
design (§16.1) and `cmd_clean` (which peeks 64 bytes then streams) avoid.
**Fix**: peek 64 bytes like clean; stream-copy the passthrough case; only
`read_to_end` once the input is known to be a manifest (manifests are small).

### [LOW-001] `all_manifests` reads every object body
**File**: `crates/core/src/bin/git-cdc.rs` (`all_manifests`)
**Issue**: one `cat-file --batch` pass reads full content of every reachable
object (commits, trees, all blobs) to sniff manifests; the size cap is checked
only after the body is already consumed from the pipe. In a cdc-tracked repo
big files live in the CAS, not git, so blobs are small — acceptable at MVP
scale.
**Fix (deferred)**: a `--batch-check` pre-pass to request bodies only for
blobs ≤ cap. Not blocking.

### [LOW-002] Bearer token comparison is not constant-time
**File**: `crates/server/src/lib.rs` (`auth`)
**Issue**: `==` on the token string is theoretically timing-observable.
Static-token MVP over HTTPS/localhost; real ACLs are declared v2 scope.
**Fix (deferred)**: constant-time compare or token hashing when auth grows
beyond a single static token.

### [INFO-001] `index_manifests` spawns two git processes per tracked file
Fine for MVP file counts; batch `check-attr --stdin` when repos get big.

## What's Good
- The three-level `smudge(clean(x)) == x` guarantee is real: unit, git-e2e,
  and network-e2e all enforce it, including the fresh-clone passthrough state.
- Store admits nothing unverified: hash checked on `put` (client and server
  reuse the same guard), re-checked on `get`, atomic rename prevents
  half-written chunks.
- oid parsing (`blake3::Hash::from_hex`) doubles as path-traversal protection
  on `/chunks/{oid}` — no string ever reaches the filesystem layer.

## Machine-Readable Verdict

```yaml
verdict: Request Changes
critical: 0
high: 0
medium: 1
low: 2
info: 1
ids: [MED-001, LOW-001, LOW-002, INFO-001]
```
