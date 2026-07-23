# Running a Server

`git-cdc-server` is a small axum HTTP server exposing an LFS-shaped batch
API over a chunk store. Use it when you want central auth (one bearer
token) instead of handing clients bucket credentials.

## Disk backend

```sh
git-cdc-server --root /srv/cdc --token my-secret --listen 0.0.0.0:8077
```

Chunks land in `/srv/cdc` using the same sharded content-addressed layout
as the client's local store.

## Store backend

The server itself can keep its bytes in a remote object-storage service —
s3, azblob, azfile, b2, dropbox, gcs, sftp, ftp, gdrive, swift, webdav,
onedrive (routed through [OpenDAL](https://opendal.apache.org), an
implementation detail none of the flags below name) — clients still speak
the batch API and never see the underlying service:

```sh
git-cdc-server --backend store --store-scheme s3 \
  --store-option bucket=my-chunks \
  --store-option region=us-east-1 \
  --store-option endpoint=http://127.0.0.1:9000 \
  --store-option enable_virtual_host_style=false \
  --token my-secret
```

`region` is required (or set `AWS_REGION`/`AWS_DEFAULT_REGION`) even for
S3-compatible services like MinIO that ignore its value — OpenDAL's S3
backend has no built-in fallback.

## Flags

| Flag | Default | Meaning |
| ---- | ------- | ------- |
| `--backend` | `disk` | `disk` or `store` |
| `--root` | — | chunk directory (required for disk) |
| `--store-scheme` | — | storage service (required for store), e.g. `s3`, `azblob`, `gcs` |
| `--store-option KEY=VALUE` | — | service option, repeatable |
| `--store-prefix` | `chunks/` | key prefix inside the service |
| `--token` | — | static bearer token (env: `GIT_CDC_TOKEN`) |
| `--listen` | `127.0.0.1:8077` | bind address (env: `GIT_CDC_LISTEN`) |
| `--grace-secs` | `86400` | GC grace period for server-side sweeps |

## API

All routes require `Authorization: Bearer <token>`, including `/healthz`.

| Route | Purpose |
| ----- | ------- |
| `POST /objects/batch` | LFS-style negotiation: which chunks to upload/download, with hrefs |
| `PUT /chunks/{oid}` | Upload one chunk — the server re-hashes the body and rejects mismatches (422) |
| `GET /chunks/{oid}` | Download one chunk |
| `POST /gc` | Client-driven mark-and-sweep: body carries the live oid set |
| `GET /healthz` | Liveness |

The batch protocol is LFS-shaped (`operation`, `objects[{oid,size}]`,
`hash_algo: "blake3"`, `basic` transfer, per-object `actions`/`error`) with
server-relative hrefs. Request bodies are capped at the 16 MiB protocol
ceiling (the largest chunk any client config can produce) plus slack.

Upload verification is the server's poisoning guard: a chunk whose bytes
don't hash to its claimed oid is refused, so one bad client can't corrupt
the store for everyone.
