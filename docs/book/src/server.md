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

## S3 backend

The server itself can keep its bytes in a bucket — clients still speak the
batch API and never see S3:

```sh
git-cdc-server --backend s3 --s3-bucket my-chunks \
  --s3-endpoint http://127.0.0.1:9000 --s3-force-path-style \
  --token my-secret
```

## Flags

| Flag | Default | Meaning |
| ---- | ------- | ------- |
| `--backend` | `disk` | `disk` or `s3` |
| `--root` | — | chunk directory (required for disk) |
| `--s3-bucket` | — | bucket (required for s3) |
| `--s3-prefix` | `""` | key prefix |
| `--s3-endpoint` | — | MinIO/R2 endpoint override |
| `--s3-force-path-style` | off | path-style addressing (MinIO) |
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
