# Serverless Mode

No server process at all: the CLI talks straight to a remote object store
via [OpenDAL](https://opendal.apache.org) — S3-compatible buckets (AWS,
MinIO, RustFS, Cloudflare R2), Azure Blob/Files, GCS, Dropbox, B2, SFTP/FTP,
WebDAV, Google Drive, OneDrive, Swift — the way restic or DVC do. Service
credentials replace the bearer token; the service's own access control
applies.

```sh
git cdc install
git config cdc.opendal.scheme s3
git config --add cdc.opendal.option bucket=my-chunks
git config --add cdc.opendal.option region=us-east-1                 # required unless AWS_REGION is set
git config --add cdc.opendal.option enable_virtual_host_style=true   # real AWS S3 — omit (or false) for MinIO
git config --add cdc.opendal.option endpoint=http://127.0.0.1:9000   # MinIO/R2 only
git config cdc.opendal.prefix chunks/                                # optional, default chunks/
git cdc track '*.dat'
```

`cdc.opendal.option` may be set multiple times — one `KEY=VALUE` pair per
service option, passed straight through to OpenDAL. If `cdc.opendal.scheme`
is set, it wins over `cdc.url` — unset it to switch back to server mode.

## How the serverless paths differ from server mode

- **push** — instead of a batch negotiation, the CLI does one paginated
  listing of the prefix, diffs against the local set, and uploads only the
  missing chunks.
- **pull** — one fetch per missing chunk, hash-verified on read.
- **gc** — the CLI lists the prefix with each object's last-modified time
  and deletes unreferenced chunks older than `--grace-secs`. (There is no
  server to own the grace period, so the CLI's flag applies.)

## Trust model

There is no server-side verification of uploads in this mode — anyone with
write access to the service can write anything. That is the same trust
boundary the service already has; what protects *you* is the read path:
every chunk is re-hashed on fetch and the whole-file oid is checked after
reassembly, so corrupt or tampered objects fail loudly instead of reaching
your worktree.

Foreign objects under the prefix (keys that aren't valid BLAKE3 hex) are
ignored by listing and never deleted by gc.

## Credentials (S3 example)

The standard AWS chain, in the usual order: environment variables, shared
config/credentials files (`~/.aws`), then instance metadata (IMDS). For
MinIO:

```sh
export AWS_ACCESS_KEY_ID=minioadmin
export AWS_SECRET_ACCESS_KEY=minioadmin
```

Unlike credentials, OpenDAL's S3 service does **not** default the region —
set `AWS_REGION`/`AWS_DEFAULT_REGION` or an explicit `region`
`cdc.opendal.option`; connecting fails loudly if neither is present. Every
other OpenDAL service authenticates the same way it always does (its own
env vars, files, or `cdc.opendal.option` entries) — see
[OpenDAL's service docs](https://opendal.apache.org/docs/category/services/)
for the option names each scheme accepts.
