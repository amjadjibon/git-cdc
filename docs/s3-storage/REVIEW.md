---
date: 2026-07-17
feature: s3-storage
branch: s3-storage
diff: main...HEAD
reviewer: Claude
iteration: 1
---

# Code Review: s3-storage

## Findings

### [LOW-001] Serverless push lists the whole bucket every time
**File**: `crates/core/src/bin/git-cdc.rs` (`cmd_push`, S3 arm)
**Issue**: one `ListObjectsV2` page per 1000 chunks on every push, even when
nothing changed. Cheap at MVP scale (a 100 GB store ≈ 50 pages) and it was a
deliberate trade against per-chunk `HeadObject`; noting the ceiling.
**Fix (deferred)**: local cache of known-remote hashes when push latency
ever matters.

### [LOW-002] `Remote::S3` builds a fresh tokio runtime + client per command
**File**: `crates/core/src/bin/git-cdc.rs` (`remote`)
**Issue**: ~tens of ms of setup per CLI invocation; irrelevant next to
network time. Fine.

### [INFO-001] `make_client` cannot fail — bad credentials surface at first request
The AWS SDK defers credential resolution; a misconfigured chain produces a
context-wrapped request error rather than a startup error. Acceptable;
noted in QA gaps.

## What's Good
- One `S3Store` implementation serves both the server backend and the
  serverless CLI — no drift risk between the two paths, and it carries the
  same put/get hash-verification guards as `DiskStore`.
- `Backend`/`Remote` as two-variant enums instead of trait objects keeps
  both dispatch sites greppable and adds zero async-trait machinery.
- GC portability fell out cleanly: `list()` returning `(hash, modified)`
  made the grace-period logic backend-agnostic (disk mtime / S3
  LastModified) with one code path.
- Bucket listing skips keys that don't parse as blake3 hex — foreign
  objects sharing a bucket can't be deleted by our GC.

## Machine-Readable Verdict

```yaml
verdict: Approve
critical: 0
high: 0
medium: 0
low: 2
info: 1
ids: [LOW-001, LOW-002, INFO-001]
```
