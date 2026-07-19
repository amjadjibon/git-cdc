---
date: 2026-07-19
feature: compression-and-ssh
status: Concluded
recommendation: (1) zstd per chunk behind a 1-byte envelope, keyed by the uncompressed BLAKE3, with hash-verification-based legacy fallback; envelope is both the storage and wire format. (2) SSH transport as `ssh host git-cdc stdio --root <path>` speaking a small pkt-line request protocol — the exact mechanism git itself uses over SSH; a config-overridable transport command makes it testable without sshd.
---

# Research: chunk compression + SSH transport

## Compression decisions

- **FIND-001 — the envelope, not sniffing.** Compressed objects can't be
  detected by content sniffing (a raw chunk may legitimately begin with a
  zstd magic). Every stored object gets a 1-byte header: `0x00` = raw,
  `0x01` = zstd frame. Ambiguity with *legacy* (pre-envelope) stores is
  resolved by verification: if the whole file hashes to the expected oid
  it IS a legacy raw chunk; otherwise the first byte is the envelope tag.
  The hash check every read already does doubles as the format detector —
  misdetection is impossible because both interpretations are verified.
- **FIND-002 — identity unchanged.** Chunks stay keyed by the
  *uncompressed* BLAKE3; manifests, dedup, GC, and the batch protocol are
  untouched. `size` fields remain uncompressed sizes.
- **FIND-003 — envelope is also the wire format.** Client↔server and
  client↔bucket carry the envelope, so transfer gets the same savings as
  storage. The server decodes to verify the uncompressed hash on upload
  (poisoning guard intact) and stores the received envelope without
  re-compressing. Wire change is pre-adoption-safe (no external users).
- **FIND-004 — ratio heuristic.** If zstd (level 3) saves < ~5%, store
  raw (`0x00`) — already-compressed media (PNG/MP4/JPEG) skips the
  decompress cost forever after paying one compression attempt at write.
- `zstd` crate v0.13 (libzstd bindings) — both directions, battle-tested.

## SSH decisions

- **FIND-005 — git's own model:** run our binary on the remote over ssh
  (`ssh host git-cdc stdio --root /srv/cdc`), like `git-upload-pack`. No
  new protocol invention: requests/responses ride the existing
  `core::pktline` framing (text command lines + content packets).
  Requires git-cdc installed on the remote host — the same requirement
  git itself has.
- **FIND-006 — testable without sshd:** `cdc.ssh.command` (advanced/test
  hook) replaces the `ssh <remote>` invocation with an arbitrary argv;
  e2e runs the stdio server as a local subprocess. Real ssh exercises the
  identical code path.
- **FIND-007 — remote selection precedence:** `cdc.s3.bucket` >
  `cdc.ssh.remote` > `cdc.url`; gc grace is CLI-owned (like S3) using
  remote file mtimes from `list`.

## Protocol sketch (stdio, v1)

Client: `git-cdc-stdio version=1` → server: `ok`. Then per request:
`has <hex>` → `yes|no` · `put <hex>` + envelope packets + flush →
`ok|err <msg>` · `get <hex>` → `ok` + envelope packets + flush, or
`err not-found` · `list` → `chunk <hex> <mtime-secs|->` lines + flush ·
`remove <hex>` → `ok`. EOF ends the session.
