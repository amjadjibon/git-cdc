# git-cdc chunk storage format, v1

How chunk bytes are stored at rest (disk stores, S3 objects) and carried on
the wire (chunk upload/download bodies, stdio transport). Reference
implementation: `crates/core/src/store/envelope.rs`.

Chunk *identity* is out of scope here: chunks are always keyed by the
BLAKE3 hash of their **uncompressed** content (see
[manifest.md](manifest.md)). This format only governs the stored/transferred
representation.

## Envelope

Every stored object is a 1-byte tag followed by the payload:

| Tag | Payload |
| --- | ------- |
| `0x00` | The raw chunk bytes |
| `0x01` | One zstd frame; decompresses to the chunk bytes |

Writers SHOULD use `0x01` only when compression saves meaningfully
(the reference writer requires > 5% at zstd level 3) and `0x00` otherwise,
so already-compressed content never pays a decompress cost on read.
A decompressed payload MUST NOT exceed 16777216 bytes (the chunk-size
protocol ceiling).

## Legacy objects

Stores written before this format hold bare chunk bytes with no tag.
Readers MUST detect them hash-first: if the entire object hashes to the
expected chunk id, it is a legacy raw chunk; only otherwise is the first
byte interpreted as an envelope tag. Both interpretations end in hash
verification, so misdetection is impossible — an object that satisfies
neither is corrupt and MUST be rejected.

Writers MAY emit legacy raw bodies (an old client uploading to a new
server is accepted through the same rule); new writers SHOULD always emit
the envelope.

## Verification

Whatever the representation, readers MUST verify the uncompressed bytes
against the chunk id before use, and store-side receivers MUST verify
before admitting an upload. Compression never weakens the
content-addressing guarantees.

## Versioning

New tag values extend the format; readers reject unknown tags as corrupt
(fail-loud, never fail-wrong). A change that alters the meaning of
existing tags would be a new store format version — none is anticipated.
