# Code Review: storage-backends

## Iteration 2 — S3 migrated onto OpenDAL (user-requested scope addition)

Base: `abfd94f...HEAD`. `S3Store` + `make_client` (aws-sdk-s3) deleted;
`S3Config` now maps the existing `--s3-*` flags / `cdc.s3.*` git config onto
`OpendalStore` with the `s3` scheme. `Backend` is back to two variants.
Net: −1030 lines, aws-sdk-s3 + aws-config out of the dependency tree
(verified with `cargo tree`; remaining `aws-*` are rustls' aws-lc and
OpenDAL's reqsign signer).

### Checks performed

- **Addressing parity**: opendal-s3 defaults to path-style; `S3Config` sets
  `enable_virtual_host_style` unless `force_path_style` — same behavior as
  the old aws-sdk default per flag state. ✔ (verified against
  opendal-service-s3 0.57.0 source)
- **Credential parity**: OpenDAL's DefaultCredentialProvider loads env →
  `~/.aws` profile → IMDS, matching the old chain minus SSO. ✔
- **Signing works end to end**: the serverless e2e (`e2e_serverless.rs`)
  pushes/pulls/GCs through opendal-s3 against the in-process s3s-fs server
  with env credentials. ✔
- **Storage-format compatibility**: keys are the same `{prefix}{hex}`,
  objects the same envelope; existing buckets keep working. ✔

### Findings

- **MED-001** (accepted, documented): region now resolves from explicit
  config or `AWS_REGION`/`AWS_DEFAULT_REGION` env only — OpenDAL does not
  read the profile's `region`. Users whose region lives solely in
  `~/.aws/config` must export `AWS_REGION`. Documented in the README
  serverless section; a profile parser is not worth the code.
- **LOW-003** (accepted): SSO credential sessions no longer supported
  (aws-sdk feature, not in reqsign). Documented in README.

### Machine-Readable Verdict (iteration 2)

```yaml
verdict: Approve
critical: 0
high: 0
medium: 1
low: 1
info: 0
```

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
