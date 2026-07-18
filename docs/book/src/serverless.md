# Serverless S3 Mode

No server process at all: the CLI talks straight to an S3-compatible bucket
(AWS S3, MinIO, RustFS, Cloudflare R2), the way restic or DVC do. IAM
credentials replace the bearer token; bucket policy is your access control.

```sh
git cdc install
git config cdc.s3.bucket my-chunks
git config cdc.s3.prefix chunks/                       # optional
git config cdc.s3.endpoint http://127.0.0.1:9000       # MinIO/R2 only
git config cdc.s3.force-path-style true                # MinIO only
git cdc track '*.dat'
```

If `cdc.s3.bucket` is set, it wins over `cdc.url` — unset it to switch back
to server mode.

## How the S3 paths differ from server mode

- **push** — instead of a batch negotiation, the CLI does one paginated
  `ListObjectsV2` of the prefix (1 request per 1000 chunks), diffs against
  the local set, and uploads only the missing chunks.
- **pull** — straight `GetObject` per missing chunk, hash-verified on read.
- **gc** — the CLI lists the prefix with each object's `LastModified` and
  deletes unreferenced chunks older than `--grace-secs`. (There is no
  server to own the grace period, so the CLI's flag applies.)

## Trust model

There is no server-side verification of uploads in this mode — anyone with
bucket write access can write anything. That is the same trust boundary the
bucket already has; what protects *you* is the read path: every chunk is
re-hashed on fetch and the whole-file oid is checked after reassembly, so
corrupt or tampered objects fail loudly instead of reaching your worktree.

Foreign objects under the prefix (keys that aren't valid BLAKE3 hex) are
ignored by listing and never deleted by gc.

## Credentials

The standard AWS chain, in the usual order: environment variables, shared
config/credentials files (`~/.aws`), then instance metadata (IMDS). For
MinIO:

```sh
export AWS_ACCESS_KEY_ID=minioadmin
export AWS_SECRET_ACCESS_KEY=minioadmin
```

The region falls back to `us-east-1` when the chain provides none —
S3-compatible stores ignore it, and the SDK just needs something to sign
with.
