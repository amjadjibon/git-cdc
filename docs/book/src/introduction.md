# Introduction

**git-cdc** stores large files in git using **content-defined chunking**
(FastCDC) instead of whole-file storage. It is a Git LFS alternative built
around one observation: most edits to big files change a small part of them,
so re-uploading and re-storing the whole file on every change is waste.

## The problem with whole-file storage

Git LFS replaces a large file with a pointer and stores the file itself on a
server — but as one opaque blob per version. Edit 4 bytes in a 30 MiB asset
and LFS uploads and stores a fresh 30 MiB copy. Ten small edits cost ten full
copies: 300 MiB of transfer and storage for a few dozen changed bytes.

## What git-cdc does instead

git-cdc splits each file into variable-size chunks (512 KiB – 8 MiB,
~2 MiB average by default) at **content-defined boundaries** — positions
chosen by a rolling hash of the bytes themselves, not by fixed offsets.
Each chunk is content-addressed by its BLAKE3 hash.

Because boundaries depend on content, an edit only changes the chunk (or
two) it touches; every other chunk keeps its hash and is never transferred
or stored again. That same 4-byte edit uploads one ~2 MiB chunk, not 30 MiB
— and the dedup applies across versions, across branches, and across files
that happen to share content.

```text
git add ──▶ clean filter ──▶ chunker ──▶ manifest (committed to git)
                                │
                                ▼
          local chunk store ──▶ git cdc push ──▶ chunk store (server or S3)
                                                    (only missing chunks)

git checkout ──▶ smudge filter ──▶ read manifest ──▶ reassemble from chunks
```

What lands in git history is a small text **manifest** — one header block
plus one line per chunk. Binary changes therefore diff cleanly in git: two
manifest versions differ only in the lines for changed chunks.

## Design principles

- **Safe by default.** A fresh clone without chunks checks out successfully
  (manifest text in the worktree, a hint to run `git cdc pull`); corruption
  is always detected before bytes reach your worktree — never silently
  materialized.
- **Everything is content-addressed.** BLAKE3 for chunks and whole files;
  every read path re-verifies hashes.
- **Two deployment shapes.** A small server with token auth and an
  LFS-style batch API — or no server at all, with the CLI talking straight
  to an S3-compatible bucket (AWS, MinIO, RustFS, R2) using IAM credentials.
- **git-native UX.** Clean/smudge filters, `.gitattributes` tracking, a
  pre-push hook — `git add`, `git commit`, `git push` work as always.

## Where to start

- [Getting Started](getting-started.md) — install and first repo in five
  minutes.
- [How It Works](how-it-works.md) — chunking, manifests, and the filter
  pipeline in depth.
