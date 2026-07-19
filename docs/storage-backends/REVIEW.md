# Code Review: storage-backends (iteration 1)

Base: `main...HEAD` — OpendalStore, Backend::Opendal arm, CLI flags, tests, README.

## Checks performed

- **Upload-poisoning guards**: `put` hash-verifies, `put_encoded` envelope-decodes
  before writing — identical to `S3Store`/`DiskStore`; covered by
  `upload_poisoning_guards` test. ✔
- **GC safety**: `list()` skips non-hex keys (foreign objects never deleted),
  maps `last_modified` for the grace check, returns empty on a never-written
  prefix instead of erroring. HTTP-level GC exercised end-to-end. ✔
- **Secrets**: `--opendal-option` values never logged; error contexts carry the
  scheme name only. ✔
- **Flag validation**: `required_if_eq` catches missing scheme at parse time;
  runtime bail covers the env/default path (same pattern as disk/s3); malformed
  `KEY=VALUE` rejected by the value parser with a unit test. ✔
- **Concurrency**: `Operator` methods take `&self`; no shared mutable state
  added. ✔
- **Clippy**: clean across both crates, all targets. ✔

## Findings

- **LOW-001** (fixed): README SFTP example used `key=~/.ssh/...` — tilde inside
  a flag value isn't shell-expanded; replaced with an absolute path.
- **LOW-002** (fixed): `list()`'s per-entry stat fallback is sequential; marked
  with a `ponytail:` ceiling comment (GC-only path, only for services whose
  listings omit timestamps).

No Critical/High/Medium findings.

## Machine-Readable Verdict

```yaml
verdict: Approve
critical: 0
high: 0
medium: 0
low: 2
info: 0
```
