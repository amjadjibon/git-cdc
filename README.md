# git-cdc

A Git LFS replacement built on **content-defined chunking**. Instead of
addressing large files by whole-file hash (where a 1-byte edit re-uploads the
entire file), git-cdc splits files into variable-size chunks at content-defined
boundaries (FastCDC) and stores them in a content-addressable store keyed by
BLAKE3. Edit 4 bytes in a 30 MiB file and only the one changed chunk (~2 MiB)
is uploaded — the other 12 are already on the server.

```text
git add ──▶ clean filter ──▶ chunker ──▶ manifest (committed to git)
                 │
                 ▼
          local chunk store ──▶ git cdc push ──▶ batch API ──▶ server CAS
                                                    (only missing chunks)

git checkout ──▶ smudge filter ──▶ read manifest ──▶ reassemble from chunks
```

What lands in git is a small text **manifest** (one line per chunk) instead of
the file content, so binary changes diff cleanly:

```text
version https://git-cdc.dev/spec/v1
chunk-avg 2097152
chunk-max 8388608
chunk-min 524288
oid blake3:fabf914c…
size 31457280
chunk blake3:9f2a1e… 0 2097152
chunk blake3:7c4d02… 2097152 2583032
…
```

## Install

```sh
cargo build --release
# put both binaries on $PATH; `git cdc <cmd>` works via git's
# git-<name> extension mechanism, same as git-lfs
cp target/release/git-cdc target/release/git-cdc-server ~/.local/bin/
```

## Quick start

Run a chunk server somewhere (all state lives in `--root`):

```sh
git-cdc-server --root /srv/git-cdc --token <secret> --listen 0.0.0.0:8077
```

In your repo:

```sh
git cdc install                 # registers the clean/smudge filter + pre-push hook
git config cdc.url http://your-server:8077
git config cdc.token <secret>
git cdc track '*.dat' '*.bin'   # writes .gitattributes

git add model.dat && git commit -m "add model"
git push                        # pre-push hook uploads chunks first, automatically
```

Cloning:

```sh
git clone <repo> && cd <repo>
git cdc install
git config cdc.url http://your-server:8077
git config cdc.token <secret>
git cdc pull                    # fetch chunks, materialize tracked files
```

A fresh clone always succeeds even before `git cdc pull` — tracked files hold
the manifest text until chunks are fetched (same passthrough model as git-lfs
pointers). git-cdc never emits wrong bytes: every chunk is hash-verified on
write and read, and reassembled files are checked against the whole-file oid.

## Commands

| Command | What it does |
|---|---|
| `git cdc install [--global]` | Register the filter driver; local install also adds a pre-push hook |
| `git cdc track <pattern>…` | Add patterns to `.gitattributes` |
| `git cdc push` | Upload chunks referenced by any local manifest (only ones the server is missing) |
| `git cdc pull` | Fetch missing chunks for the current checkout and materialize files |
| `git cdc gc [--dry-run] [--grace-secs N]` | Mark-and-sweep unreferenced chunks, locally and on the server |
| `git cdc diff <a> <b>` | Chunk-level changelist between two manifest files |

## Layout

```text
crates/
├── core/     # git-cdc-core: chunker, manifest, chunk store, batch client + git-cdc CLI
└── server/   # git-cdc-server: axum batch API + chunk storage
```

- **Chunking**: FastCDC, 512 KiB min / 2 MiB avg / 8 MiB max, streamed (bounded memory).
- **Hashing**: BLAKE3 for chunks and whole files.
- **Protocol**: Git-LFS-shaped batch API (`POST /objects/batch` with `hash_algo`, `ref`), `basic` HTTP transfer, static bearer auth.
- **GC**: client-driven — the client enumerates every manifest reachable from any ref and sends the live set; the server deletes the rest after a grace period (default 24 h) that protects in-flight uploads.

## Development

```sh
cargo test --workspace   # unit + git-integration + full network e2e
```

Design and plan documents live in [`docs/git-cdc-mvp/`](docs/git-cdc-mvp/).
Out of scope for this MVP (see the plan): S3 backends, the git filter-process
protocol, transfer adapters, SSH transport, and compression.
