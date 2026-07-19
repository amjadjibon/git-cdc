---
date: 2026-07-19
feature: filter-process
status: Concluded
recommendation: Implement filter.cdc.process speaking the v2 pkt-line protocol with clean+smudge capabilities (not delay, v2 of this feature); stream content through pkt-line Read/Write adapters so memory stays bounded by chunk size. Real git in the e2e suite is the protocol conformance test.
---

# Research: git filter-process protocol

## Why

One-shot clean/smudge spawns a process per tracked file per git operation
(plus several `git config` shell-outs each). `filter.<driver>.process` keeps
ONE filter process alive for the whole operation — the checkout-speed
ceiling named in the MVP design.

## Protocol facts (gitattributes(5), stable since git 2.11)

- **pkt-line framing**: 4 ASCII hex digits = total length including the 4
  prefix bytes, then payload. Flush packet is `0000`. Max payload per
  packet: 65516 bytes.
- **Handshake**: git sends `git-filter-client` + `version=2` + flush; the
  filter answers `git-filter-server` + `version=2` + flush; git sends its
  capability list (`capability=clean`, `capability=smudge`,
  `capability=delay`) + flush; the filter answers with the subset it
  supports + flush. Key lines are `key=value` with trailing `\n`.
- **Per file**: git sends `command=clean|smudge`, `pathname=<path>`
  (possibly more keys — ignore unknown ones), flush, then raw content
  packets, flush. The filter answers `status=success` + flush, content
  packets, flush, then an **empty list** (a lone flush) meaning "status
  unchanged". Error before content: `status=error` + flush. Fatal:
  `status=abort`. Filter exit kills the whole git operation.
- **When `process` is configured, `clean`/`smudge` keys are ignored** for
  that driver (kept anyway as fallback for git < 2.11).
- `delay` capability (async smudge) explicitly deferred — needs
  `list_available_blobs` bookkeeping; separate feature.

## Design consequences

- **FIND-001 — stream, don't buffer**: content arrives as a packet
  sequence; a `Read` adapter over packets (and a `Write` adapter emitting
  ≤65516-byte packets) lets the existing chunker/smudge logic run
  unmodified with memory bounded by chunk size, not file size.
- **FIND-002 — refactor, not fork**: `cmd_clean`/`cmd_smudge` bodies move
  to `clean_stream(reader, writer, …)` / `smudge_stream(reader, writer, …)`
  shared by both the one-shot commands and the process loop. Chunk params
  and the store open once per process (kills the per-file `git config`
  shell-outs).
- **FIND-003 — error discipline**: smudge passthrough (missing chunks)
  stays a `status=success` with manifest bytes; corruption becomes
  `status=error` for that file (git reports it and the checkout of other
  files continues; with no `required` flag git keeps the blob content).
- **FIND-004 — real git is the conformance test**: the e2e suite drives
  the filter through actual `git add`/`git checkout` with
  `filter.cdc.process` configured (local git 2.39 ≥ 2.11). A handshake or
  framing mistake fails the suite immediately.

## Constant-time token compare (folded in)

`blake3::Hash` equality is already constant-time (the crate uses
`constant_time_eq`). Hashing both the presented and expected token and
comparing the hashes gives a constant-time check with zero new
dependencies.
