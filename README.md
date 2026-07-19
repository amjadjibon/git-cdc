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

Full format definition: [docs/spec/manifest.md](docs/spec/manifest.md).

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

### Serverless mode (S3, no server)

Skip the server entirely and let the CLI talk straight to an S3-compatible
bucket (AWS S3, MinIO, R2) — credentials come from the standard AWS chain
(env vars, `~/.aws`, IMDS), so IAM replaces the bearer token:

```sh
git cdc install
git config cdc.s3.bucket my-chunks
git config cdc.s3.prefix chunks/                       # optional
git config cdc.s3.endpoint http://127.0.0.1:9000       # MinIO/R2 only
git config cdc.s3.force-path-style true                # MinIO only
git cdc track '*.dat'
```

`push`/`pull`/`gc` then negotiate against the bucket directly (one listing
instead of a batch call). If `cdc.s3.bucket` is set it wins over `cdc.url`.

### SSH transport (no server, no bucket)

Any host you can ssh into (with `git-cdc` installed there) can hold the
chunks — the same model as git itself over SSH:

```sh
git cdc install
git config cdc.ssh.remote user@host
git config cdc.ssh.path /srv/cdc-chunks
git cdc track '*.dat'
```

The CLI runs `ssh user@host git-cdc stdio --root /srv/cdc-chunks` and
speaks a pkt-line protocol over the pipe; your ssh config (keys, agents,
jump hosts) applies as-is. Precedence: `cdc.s3.bucket` >
`cdc.ssh.remote` > `cdc.url`.

### Compression

Chunks are stored and transferred zstd-compressed automatically whenever
it saves more than ~5% — already-compressed media (PNG, MP4) is detected
and kept raw. Identity stays the uncompressed BLAKE3, so manifests, dedup,
and existing stores are unaffected (pre-compression stores keep working).
Format: [docs/spec/chunk-storage.md](docs/spec/chunk-storage.md).

### Server with S3 storage

The server itself can also store chunks in a bucket instead of local disk —
central token auth stays, S3 holds the bytes:

```sh
git-cdc-server --backend s3 --s3-bucket my-chunks \
  --s3-endpoint http://127.0.0.1:9000 --s3-force-path-style \
  --token <secret>
```

### Server with Azure, GCS, SFTP, FTP, Google Drive, WebDAV, OneDrive

The `opendal` backend routes chunk storage through
[Apache OpenDAL](https://opendal.apache.org/): pick a scheme and pass its
options as repeatable `--opendal-option KEY=VALUE` flags (passed to OpenDAL
verbatim — see the [service docs](https://docs.rs/opendal/latest/opendal/services/)
for each scheme's keys):

```sh
# Azure Blob
git-cdc-server --backend opendal --opendal-scheme azblob \
  --opendal-option container=my-chunks \
  --opendal-option account_name=me --opendal-option account_key=... \
  --token <secret>

# Google Cloud Storage
git-cdc-server --backend opendal --opendal-scheme gcs \
  --opendal-option bucket=my-chunks \
  --opendal-option credential_path=/path/to/sa.json --token <secret>

# Nextcloud (or any WebDAV server)
git-cdc-server --backend opendal --opendal-scheme webdav \
  --opendal-option endpoint=https://cloud.example.com/remote.php/dav/files/me \
  --opendal-option username=me --opendal-option password=<app-password> \
  --token <secret>

# SFTP (unix only, SSH key auth only — no passwords)
git-cdc-server --backend opendal --opendal-scheme sftp \
  --opendal-option endpoint=ssh://me@host --opendal-option key=~/.ssh/id_ed25519 \
  --token <secret>
```

`ftp`, `gdrive`, and `onedrive` work the same way. Google Drive and OneDrive
need an OAuth `refresh_token` + `client_id` + `client_secret` (access-token-only
setups expire after ~1h) and have API quotas that make them a "works, not
recommended" tier for chunk traffic. Plain `ftp` sends credentials in the
clear — prefer FTPS or anything else on this list.

Chunks land under `--opendal-prefix` (default `chunks/`). Alternatively, skip
all of this: `rclone serve s3 remote:` fronts every one of these services with
an S3 API, and the existing `--backend s3` (or serverless mode) works against
it unchanged.

Cloning:

```sh
git clone <repo> && cd <repo>
git cdc install
git config cdc.url http://your-server:8077
git config cdc.token <secret>
git cdc pull                    # fetch chunks, materialize tracked files
```

### Global setup with an include file

[`.gitconfig.cdc`](.gitconfig.cdc) in this repo is a commented sample of
every git-cdc setting. Point your global gitconfig at a copy of it:

```sh
cp .gitconfig.cdc ~/.gitconfig.cdc
git config --global include.path ~/.gitconfig.cdc
```

Then uncomment/edit the sections you need in `~/.gitconfig.cdc` — the filter
registration is safe to keep global (it only activates for tracked paths);
remote and chunk settings are better kept repo-local unless all your repos
share one chunk server or bucket. Per-repo `git config` values override the
included globals. `git cdc install` is still needed once per repo for the
pre-push hook.

### Chunk size tuning

FastCDC bounds are configurable per repo (defaults: 512 KiB / 2 MiB / 8 MiB).
Smaller chunks dedup finer-grained edits at the cost of more objects; larger
chunks mean fewer round-trips for huge, rarely-edited assets:

```sh
git config cdc.chunk.min 64k    # 64 B – 1 MiB
git config cdc.chunk.avg 256k   # 256 B – 4 MiB
git config cdc.chunk.max 1m     # 1 KiB – 16 MiB   (min ≤ avg ≤ max)
```

Values are bytes; git's `k`/`m`/`g` suffixes work. Out-of-range or misordered
values fail the clean filter with an error naming the key. The settings apply
when files are *chunked* (`git add`) — existing manifests are self-describing
and stay valid, so changing them never breaks history. One caveat: all
clients of a repo should use the same values, otherwise re-cleaning the same
content on different machines produces different (equally valid) manifests,
which shows up as spurious diffs. Set them repo-locally, not `--global`.

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

## Documentation

The full user guide lives in [`docs/book/`](docs/book/) as an
[mdBook](https://rust-lang.github.io/mdBook/):

```sh
cargo install mdbook
mdbook serve docs/book        # http://localhost:3000
```

## Development

```sh
cargo test --workspace   # unit + git-integration + network e2e + S3 e2e
```

The S3 suites self-host an in-process S3 server ([s3s-fs](https://crates.io/crates/s3s-fs))
— no docker or MinIO needed. To run them against a real S3-compatible store
(MinIO, RustFS, AWS) instead:

```sh
GIT_CDC_TEST_S3_ENDPOINT=http://127.0.0.1:9000 \
AWS_ACCESS_KEY_ID=… AWS_SECRET_ACCESS_KEY=… \
cargo test --workspace
```

Design and plan documents live in [`docs/`](docs/). Out of scope so far
(see the plans): transfer adapters / pre-signed URL offload and the
filter-process `delay` capability.
