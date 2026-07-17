---
date: 2026-07-17
feature: s3-storage
branch: s3-storage
verdict: Pass
---

# QA Report: s3-storage

28 tests: 26 pre-existing (all still green — regression gate for the Backend
and Remote refactors) + 2 new env-gated S3 suites, both verified against a
real MinIO container during development:

| Suite | Covers |
|---|---|
| `git-cdc-server --test s3_backend` | S3Store has/put/get/remove round trip, corrupt-put rejection, list with LastModified age |
| `git-cdc-core --test e2e_serverless` | Full serverless cycle: track → commit → direct-to-bucket push (dedup asserted: 1-byte edit ≤2 chunks) → fresh clone passthrough → pull materialization → dry-run + real bucket GC |

Also smoke-verified manually: `git-cdc-server --backend s3` against MinIO
(startup flag validation, PUT/GET chunk round trip through the bucket).

## Gaps accepted (not blocking)

- Gated tests don't run in the default environment (no S3) — the disk/HTTP
  regression suite always runs; MinIO instructions are in the README.
- No test for AWS credential-chain failure modes (bad creds surface as a
  request error with context — manual verification only).
- `e2e_serverless` clears its hardcoded test bucket prefix on start; safe
  for the dedicated `git-cdc-test-serverless` bucket, would bite if someone
  reused that name for real data.
