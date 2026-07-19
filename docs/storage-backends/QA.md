# QA: storage-backends

## Coverage analysis

| Surface | Covered by |
|---------|-----------|
| `OpendalStore` put/has/get/get_encoded/remove | `opendal_backend.rs::round_trip_remove_and_gc_listing` |
| Upload-poisoning guards (wrong hash, corrupt envelope) | `opendal_backend.rs::upload_poisoning_guards` |
| `list()` mtime + foreign-key skip + empty-store NotFound | `opendal_backend.rs::round_trip_remove_and_gc_listing` |
| Prefix normalization (no trailing slash) | `fs_store` helper uses `"chunks"` deliberately |
| CLI arg validation (`--opendal-scheme` required, KEY=VALUE parsing) | `main.rs::tests::opendal_backend_requires_scheme` |
| `Backend::Opendal` arms through HTTP (upload/download/GC) | `opendal_backend.rs::server_round_trip_and_gc_over_opendal` (added by QA) |

## Gaps accepted (not tested)

- **Real cloud services** (azblob, gcs, sftp, ...): transport is OpenDAL's
  contract; the `fs` scheme exercises all store logic. Testing seven live
  services needs credentials/accounts — out of scope (RESEARCH ASSUMPTION-001/002).
- **`list()` stat-fallback branch** (service listing omits timestamps): not
  triggerable with `fs`; the GC contract tolerates `None` anyway.

## Verdict

`cargo test --workspace`: all suites green. Coverage of the new code is
complete at the logic level; remaining gaps are external-service transport
only. **Pass.**
