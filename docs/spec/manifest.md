# git-cdc manifest spec, v1

Version identifier: `git-cdc/spec/v1`

This is the normative spec for the manifest format git-cdc commits to git in
place of large file content (the analogue of a git-lfs pointer file). The
reference implementation is `crates/core/src/manifest.rs`; where they
disagree, this document wins and the code has a bug.

## Example

```text
version git-cdc/spec/v1
chunk-avg 2097152
chunk-max 8388608
chunk-min 524288
oid blake3:a1b2c3...64 hex chars...
size 31457280
chunk blake3:d4e5f6... 0 2097152
chunk blake3:0a1b2c... 2097152 1835008
...
```

## Encoding

- UTF-8, LF (`\n`) line endings only — a CR byte anywhere makes the file
  invalid. Every line, including the last, ends with `\n`.
- Each line is `{key} {value}` — key, one ASCII space, value. Empty lines
  are invalid.
- The file has two sections in order: **header lines**, then **chunk lines**.
  A header key appearing after the first chunk line is invalid.

## Detection

A blob is treated as a manifest iff its first line is exactly
`version git-cdc/spec/v1` (byte-for-byte, terminated by `\n`). Anything
else is passed through untouched by the filters — this is the safety
property that makes fresh clones and mixed repos work. The version value is
an opaque format tag, not a URL; it changes only on incompatible format
revisions.

## Header

The `version` line is always first. The remaining header keys are sorted
ascending by byte value. Required keys:

| Key | Value |
| --- | ----- |
| `chunk-avg` | FastCDC average chunk size in bytes, decimal |
| `chunk-max` | FastCDC maximum chunk size in bytes, decimal |
| `chunk-min` | FastCDC minimum chunk size in bytes, decimal |
| `oid` | Whole-file hash: `blake3:` + 64 lowercase hex chars |
| `size` | Total file size in bytes, decimal |

Header keys must match `[a-z0-9.-]+`. Unknown keys are not an error:
parsers MUST preserve them verbatim so a rewrite by an older tool never
drops fields added by a newer one (forward compatibility). Writers MUST
emit them in the same sorted order as the required keys.

The reference writer uses chunk-min 524288 (512 KiB), chunk-avg 2097152
(2 MiB), chunk-max 8388608 (8 MiB); readers take the values from the
manifest, not from constants.

## Chunk lines

One line per chunk, in file byte order:

```text
chunk blake3:<64 hex> <offset> <length>
```

- `offset` — decimal byte offset of the chunk in the original file.
- `length` — decimal chunk length in bytes.
- Chunks are contiguous and non-overlapping: the first offset is 0, each
  subsequent offset is the previous offset + length, and the lengths MUST
  sum exactly to `size`. A mismatch makes the manifest invalid.
- A zero-byte file has zero chunk lines.

## Hashes

All hashes are BLAKE3, written as `blake3:` followed by 64 lowercase hex
characters. `oid` hashes the entire original file; each chunk hash hashes
that chunk's bytes. The chunk store is keyed by chunk hash
(content-addressed), and readers re-verify both chunk hashes on fetch and
the whole-file `oid` after reassembly.

## Byte stability

Encoding is deterministic: parsing a valid manifest and re-encoding it
MUST reproduce the input byte-for-byte. Git stores manifests as blobs, so
any instability would show up as spurious diffs.

## Versioning

Backward-compatible additions (new header keys) do not change the version
string — unknown-key preservation covers them. Any change that would make
a v1 parser misread a manifest requires a new version value, which v1
parsers will correctly treat as non-manifest content.
