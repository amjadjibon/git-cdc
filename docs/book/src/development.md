# Development

## Layout

```text
crates/
├── core/         # git-cdc-core: chunker, manifest, stores, protocol, CLI
│   └── src/bin/git-cdc.rs
└── server/       # git-cdc-server: axum batch API over disk or S3
docs/
├── book/         # this book (mdbook)
├── spec/         # normative manifest spec
└── <feature>/    # per-feature plan/QA/review artifacts
```

## Testing

```sh
cargo test --workspace
```

That single command runs everything — including the S3 suites, which
self-host an in-process S3 server
([`s3s-fs`](https://crates.io/crates/s3s-fs) over a temp dir) so no docker
or MinIO is needed. The suites:

- unit tests: chunker (bounds, dedup, params), manifest (round-trip,
  strictness), stores, protocol wire format;
- `e2e_filter`: real `git add`/`git checkout` against scratch repos —
  byte-identical restore, passthrough safety, corrupt-store refusal,
  chunk-size config;
- `e2e_full`: an in-process server — dedup on second push, pre-push hook
  guard, fresh clone, pull, GC;
- `e2e_serverless`: the same cycle straight against a bucket;
- server integration: auth, negotiation, upload verification, body limits,
  GC grace.

Every test subprocess runs with `GIT_CONFIG_GLOBAL=/dev/null` — your real
gitconfig never leaks into scratch repos.

To point the S3 suites at a real store instead (worth one run per release):

```sh
GIT_CDC_TEST_S3_ENDPOINT=http://127.0.0.1:9000 \
AWS_ACCESS_KEY_ID=… AWS_SECRET_ACCESS_KEY=… \
cargo test --workspace
```

## Coverage

```sh
cargo install cargo-llvm-cov
cargo llvm-cov --workspace
```

## This book

```sh
mdbook serve docs/book    # live-reload at http://localhost:3000
mdbook build docs/book    # static site in docs/book/book/
```

## Deliberately out of scope (so far)

The git filter-process protocol (long-running filter), transfer adapters /
pre-signed URL offload, SSH transport, compression, and per-branch access
control (the batch API's `ref` field is reserved for it). Design rationale
for all of these lives in `docs/git-cdc-mvp/DESIGN.md`.
