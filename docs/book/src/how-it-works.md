# How It Works

## Content-defined chunking

git-cdc uses **FastCDC** (the 2020 "normalized chunking" variant). A rolling
gear hash fingerprints the last few dozen bytes at every position in the
file; when the fingerprint matches a bit mask, that position becomes a chunk
boundary. Three parameters shape the result:

- **min** (default 512 KiB) — after a cut, this many bytes are skipped
  before boundary search resumes. No chunk is smaller (except a file's last
  chunk).
- **avg** (default 2 MiB) — sets the mask's bit count and therefore the cut
  probability (~1 in 2²¹ per byte for 2 MiB). Chunk sizes scatter around
  this value.
- **max** (default 8 MiB) — if no boundary appears within this many bytes,
  a cut is forced. Bounds per-chunk memory and upload size.

Because boundaries are functions of content, inserting or editing bytes
only reshapes the chunk(s) around the edit — everything after it re-aligns
to the same boundaries and keeps the same hashes. That's the property that
makes dedup work across versions.

All three are configurable per repo — see
[Configuration](configuration.md#chunk-size-tuning).

## Hashing

Every chunk is hashed with **BLAKE3**; the whole file gets its own BLAKE3
`oid`, computed in the same streaming pass as the chunking (no second read).
Chunks are stored and addressed purely by hash — identical content across
files, branches, or history is stored exactly once. Storage:

- locally in `.git/cdc/objects/<xx>/<yy>/<hex>` (sharded like git's own
  object store),
- remotely as flat `<prefix><hex>` keys in S3, or the same sharded layout
  on a server's disk.

Every read path re-verifies: chunk hashes on fetch and on smudge, the
whole-file oid after reassembly. A corrupt chunk can fail a checkout but
can never silently materialize wrong bytes.

At rest and on the wire, chunks travel in a small **envelope** — zstd-
compressed automatically when that saves more than ~5%, raw otherwise (so
already-compressed media pays no decompress cost). Identity is always the
uncompressed BLAKE3, so compression is invisible to manifests, dedup, and
GC; pre-compression stores keep working. Format:
`docs/spec/chunk-storage.md` in the repository.

## The filter pipeline

git-cdc plugs into git's clean/smudge filter machinery, exactly like
git-lfs — via the long-running **filter-process protocol**: git starts one
`git-cdc filter-process` per operation and streams every tracked file
through it over pkt-line framing, so a 500-file checkout costs one process,
not 500 (one-shot `clean`/`smudge` remain as the git < 2.11 fallback):

**clean** (runs on `git add` for tracked paths): reads file content from
stdin, chunks it, writes each chunk into the local store, and emits the
manifest — which is what git actually commits. If the input is *already* a
manifest (a fresh-clone worktree that was never materialized), it passes
through unchanged rather than chunking the manifest text itself.

**smudge** (runs on checkout): reads a manifest from stdin and emits the
reassembled file. Three outcomes:

1. All chunks local → reassemble, verify the oid, emit the bytes.
2. Chunks missing → emit the manifest text itself and print
   ``run `git cdc pull` to fetch file content`` — checkout never fails on
   absence.
3. A chunk is corrupt, or the reassembled oid mismatches → hard error.
   Absence is safe; corruption is loud.

Non-manifest input (a file committed before tracking) streams through
untouched.

## The manifest

A small, strictly-formatted text file (the analogue of an LFS pointer):

```text
version git-cdc/spec/v1
chunk-avg 2097152
chunk-max 8388608
chunk-min 524288
oid blake3:fabf914c…
size 31457280
chunk blake3:9f2a1e… 0 2097152
chunk blake3:7c4d02… 2097152 2583032
…
```

Manifests are byte-stable (sorted headers, LF-only, deterministic
encoding), so identical content always produces an identical blob, and two
versions of a binary diff as a few changed `chunk` lines. The full grammar
lives in the [Manifest Format Spec](spec.md).

## Sync

**push** walks *all* of history (`git rev-list --all --objects`), sniffs
out every manifest blob by its fixed first line, collects the referenced
chunk set, asks the remote which chunks it's missing (one batch call, or
one S3 listing), and uploads only those. The pre-push hook runs this
automatically, so manifests never reach the git remote ahead of their
chunks.

**pull** does the reverse for the *current checkout*: fetch missing chunks
into the local store, then materialize any tracked file whose worktree
still holds manifest text.

Branches need no special handling: the chunk store is branch-agnostic, and
push/gc walk every ref. Identical content on two branches is stored once.
