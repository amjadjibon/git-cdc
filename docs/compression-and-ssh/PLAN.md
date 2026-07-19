---
status: Ready
version: 1.0
date: 2026-07-19
feature: compression-and-ssh
research: docs/compression-and-ssh/RESEARCH.md
---

# Plan: zstd chunk compression + SSH transport

Two deferred design items (user: "do 1. and 3."). Decisions per RESEARCH.

## Assumptions

- Envelope per RESEARCH FIND-001..004; wire format change is safe
  pre-adoption. Legacy raw stores keep working via hash-first detection.
- SSH per FIND-005..007: our binary on the remote, pkt-line stdio
  protocol, `cdc.ssh.command` as the sshd-free test hook.
- No compression config knob (heuristic handles incompressible data);
  escape hatch deferred until someone is CPU-bound.

## Phase 1: envelope in the stores

- [x] **1.1** `store::envelope`: `encode(raw) -> Vec<u8>` (zstd-3 with
  <5%-savings → raw tag) and `decode(bytes, expected) -> Result<Vec<u8>>`
  (legacy hash-first, then tag dispatch, always verified). Unit tests:
  compressible/incompressible/legacy/corrupt/empty.
- [x] **1.2** `DiskStore` and `S3Store` store envelopes (`put` encodes,
  `get` decodes+verifies); `put_encoded`/`get_encoded` for the server's
  pass-through. Legacy chunks in existing stores still read.
  *Done when*: store unit suites + S3 suites pass; a legacy-format file
  planted in a store round-trips.

## Phase 2: envelope on the wire

- [x] **2.1** HTTP: client uploads/downloads envelopes; server `put_chunk`
  decodes to verify then `put_encoded`; `get_chunk` serves stored bytes.
  *Done when*: e2e_full green; server integration tests updated for the
  envelope wire format.

## Phase 3: SSH transport

- [x] **3.1** Hidden `git-cdc stdio --root <path>` serving the RESEARCH
  protocol over stdin/stdout (DiskStore + envelope underneath).
- [x] **3.2** `Remote::Ssh` in the CLI: spawn `ssh <remote> git-cdc stdio
  --root <path>` (or `cdc.ssh.command` override), speak the protocol;
  push/pull/gc arms (list→diff→put; get; mtime grace like S3). Selection:
  s3 > ssh > http, config keys `cdc.ssh.remote`, `cdc.ssh.path`,
  `cdc.ssh.command`.
  *Done when*: e2e cycle (track→push→clone→pull→gc) passes with the
  command-override transport.

## Phase 4: tests + docs

- [x] **4.1** e2e_ssh suite; compression assertion (store bytes for a
  compressible asset < raw size; incompressible stays ~raw); legacy-store
  upgrade test.
- [x] **4.2** Docs: `docs/spec/chunk-storage.md` (envelope format,
  versioning); README + book (SSH quick start, compression note,
  config keys); `.gitconfig.cdc` ssh section; remove both items from
  out-of-scope lists.

## Verification

```sh
cargo test --workspace && cargo clippy --workspace --all-targets && cargo fmt --all --check
```
