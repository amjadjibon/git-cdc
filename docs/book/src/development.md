# Development

## Layout

```text
crates/
├── core/         # git-cdc-core: chunker, manifest, stores, protocol, client
├── cli/          # git-cdc-cli: the `git-cdc` binary (filters, push/pull/gc, transports)
└── server/       # git-cdc-server: axum batch API over disk or any OpenDAL service
docs/
├── book/         # this book (mdbook)
├── spec/         # normative manifest spec
└── <feature>/    # per-feature plan/QA/review artifacts
```

## Testing

```sh
cargo test --workspace
```

That single command runs everything — including the serverless suite,
which exercises the OpenDAL remote against the `fs` scheme in a temp dir,
so no docker or MinIO is needed. The suites:

- unit tests: chunker (bounds, dedup, params), manifest (round-trip,
  strictness), stores, protocol wire format;
- `filter`: real `git add`/`git checkout` against scratch repos —
  byte-identical restore, passthrough safety, corrupt-store refusal,
  chunk-size config;
- `full`: an in-process server — dedup on second push, pre-push hook
  guard, fresh clone, pull, GC;
- `serverless`: the same cycle straight against an OpenDAL remote (`fs`
  scheme locally — any other service is OpenDAL's contract to verify);
- `ssh`: the same cycle over the stdio transport;
- server integration: auth, negotiation, upload verification, body limits,
  GC grace.

Every test subprocess runs with `GIT_CONFIG_GLOBAL=/dev/null` — your real
gitconfig never leaks into scratch repos.

## Benchmarks

```sh
cargo bench -p git-cdc-core
```

Criterion benches cover the hot paths — chunker throughput, envelope
encode/decode, manifest codec, pkt-line framing, disk store. Baselines
and interpretation live in `docs/benchmarks/RESULTS.md`; headline: the
chunker sustains ~760 MiB/s, everything else is faster, so real-world
performance is network-bound.

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

Transfer adapters / pre-signed URL offload, the filter-process `delay`
capability, and per-branch access control (the batch API's `ref` field is
reserved for it). Design rationale lives in `docs/git-cdc-mvp/DESIGN.md`.
