# git-cdc — Content-Defined Chunking Git LFS Replacement

**Design document — Rust implementation**

## 1. Motivation

Standard Git LFS addresses objects by whole-file SHA-256. A single-byte
change to a large binary re-uploads and re-stores the entire file. This
design replaces whole-file addressing with content-defined chunking (CDC),
so only the chunks that actually changed are transferred and stored —
similar to restic, borg, and Perkeep.

## 2. Goals

- Byte-level dedup across file versions and across files.
- Drop-in compatibility with the Git filter/smudge workflow.
- Server storage backend agnostic (local disk, S3/MinIO).
- Garbage collection via manifest reference counting.

## 3. Architecture Overview

```
git add ──▶ clean filter ──▶ chunker ──▶ manifest ──▶ tracked blob (in git)
                                │
                                ▼
                        batch API (which chunks missing?)
                                │
                                ▼
                        upload missing chunks ──▶ CAS storage

git checkout ──▶ smudge filter ──▶ read manifest ──▶ fetch chunks ──▶ reassemble
```

Four components:

1. **Chunker** — splits file content into variable-size chunks at
   content-defined boundaries.
2. **Manifest format** — ordered list of chunk hashes replacing the LFS
   pointer file.
3. **Git filter driver** — clean/smudge binary invoked by Git.
4. **Server** — batch negotiation + content-addressable storage (CAS).

## 4. Chunking Algorithm

**FastCDC** (Xia et al.), gear-hash based, O(n) with no backtracking.

- Target chunk size: 2 MiB (tunable per repo via config)
- Min chunk size: 512 KiB
- Max chunk size: 8 MiB
- Rolling hash: gear hash (64-bit, table-driven) — fast on both x86 and ARM

Rust crate: `fastcdc` (pure Rust, streaming iterator API) is a good starting
point; can vendor/fork if custom mask tuning is needed.

```rust
use fastcdc::v2020::FastCDC;

fn chunk_file(data: &[u8]) -> Vec<Chunk> {
    let chunker = FastCDC::new(data, MIN_SIZE, AVG_SIZE, MAX_SIZE);
    chunker
        .map(|entry| {
            let bytes = &data[entry.offset..entry.offset + entry.length];
            Chunk {
                hash: blake3::hash(bytes),
                offset: entry.offset as u64,
                length: entry.length as u32,
            }
        })
        .collect()
}
```

For files streamed from disk (avoid loading multi-GB files into memory),
use a buffered reader and FastCDC's streaming mode, or chunk in fixed-size
read windows with an overlap buffer if the crate doesn't support true
streaming.

## 5. Chunk Hashing

Use **BLAKE3** instead of SHA-256:

- ~10x faster on modern CPUs, SIMD-friendly
- 256-bit output, collision-resistant
- Native Rust crate (`blake3`) with no C dependency

Chunk hash is the CAS key. Manifest and server both speak BLAKE3 hex
strings.

## 6. Manifest Format

> **Superseded**: the header format below is replaced by the stricter
> LFS-pointer-style encoding in section 15.1, which is the single
> normative spec. This section is kept for the design rationale only.
> The whole-file `oid` in 15.1 is computed with a running
> `blake3::Hasher` fed alongside chunking — one pass, no re-read.

Replaces the single-oid LFS pointer. Stored as the tracked Git blob (small,
diffs cleanly since only affected chunk-hash lines change on edits near a
boundary).

```
cdc-manifest-v1
size 41943040
chunk-size-avg 2097152
chunk b3:9f2a1e...  offset 0        length 2097152
chunk b3:7c4d02...  offset 2097152  length 2093012
chunk b3:1a88ff...  offset 4190164  length 2101344
...
```

Design notes:
- Text format for git-diff friendliness (reviewers can see which chunk
  entries changed).
- `size` field allows quick sanity check without fetching all chunks.
- Chunk order is the file's byte order — reassembly is a simple concat.

## 7. Protocol: Extending the LFS Batch API

Reuse the existing LFS batch endpoint shape but negotiate at chunk
granularity instead of file granularity.

```
POST /objects/batch
{
  "operation": "upload",
  "objects": [
    { "oid": "b3:9f2a1e...", "size": 2097152 },
    { "oid": "b3:7c4d02...", "size": 2093012 }
  ]
}
```

Server responds only with actions for chunks it doesn't already have
(the dedup win — across files, across branches, across the whole repo):

```
{
  "objects": [
    { "oid": "b3:7c4d02...", "actions": {
        "upload": { "href": "https://cas.example.com/put/b3:7c4d02...", "expires_in": 900 }
    }}
  ]
}
```

Chunks already present server-side are simply omitted from the response —
client skips uploading them.

## 8. Server Storage (CAS)

Minimal interface:

```rust
#[async_trait]
trait ChunkStore {
    async fn has(&self, hash: &Blake3Hash) -> Result<bool>;
    async fn put(&self, hash: &Blake3Hash, data: Bytes) -> Result<()>;
    async fn get(&self, hash: &Blake3Hash) -> Result<Bytes>;
    async fn refcount_inc(&self, hash: &Blake3Hash) -> Result<()>;
    async fn refcount_dec(&self, hash: &Blake3Hash) -> Result<u64>;
}
```

Backends:
- **Local disk**: shard by hash prefix (`b3/9f/2a/9f2a1e...`) to avoid
  huge directories.
- **S3/MinIO**: key = hash, use pre-signed PUT/GET URLs so the server
  brokers the batch negotiation but doesn't proxy bytes.
- **Postgres** (small deployments): `bytea` column, fine under a few GB
  per chunk table before S3 becomes worth it.

## 9. Garbage Collection

Manifests are the only thing that reference chunks. GC sweep:

1. Walk all reachable Git refs, extract all manifest blobs.
2. Parse each manifest, collect the set of live chunk hashes.
3. Any chunk in the store not in that set (and past a grace period, to
   avoid racing in-flight uploads) is deleted.

Reference counting at write time is an optimization (avoids full sweeps)
but a full mark-and-sweep GC should remain as the source of truth.

## 10. Git Filter Integration

`.gitattributes`:
```
*.bin filter=cdc -text
```

`.git/config`:
```
[filter "cdc"]
    clean = git-cdc clean %f
    smudge = git-cdc smudge %f
    process = git-cdc process   ; long-running process mode, avoids per-file spawn cost
```

Note the filter *name* (`cdc`, matched by `filter=cdc` in `.gitattributes`)
is independent of the *binary* name (`git-cdc`) — Git just runs whatever
command string is configured here, it doesn't need to match the filter
name at all. Keeping them visually similar is purely for readability.

Prefer Git's **filter process protocol** (long-running, packet-line based)
over the simple clean/smudge subprocess model — avoids the fork/exec cost
per file that plain LFS pointer swaps already accept but that would be
much more expensive with a CDC pass on every checkout.

## 11. Suggested Rust Crate Layout

```
git-cdc/
├── crates/
│   ├── chunker/        # FastCDC wrapper, chunk struct, manifest (de)serialization
│   ├── protocol/        # batch API request/response types (serde)
│   ├── store/            # ChunkStore trait + disk/s3/postgres backends
│   ├── server/           # axum-based batch + transfer endpoints
│   ├── filter/           # git filter-process client binary
│   └── cli/              # init, gc, status, config commands
└── Cargo.toml            # workspace
```

Suggested stack, consistent with your existing Rust projects: `axum` for
the server, `tokio`, `blake3`, `fastcdc`, `serde`/`serde_json` for the
batch protocol, `sqlx` if using Postgres for refcounts/metadata.

## 12. Open Design Questions

- **Compression**: compress chunks at rest (zstd) before storage? Adds
  CPU cost per chunk but helps text-adjacent binaries (e.g. uncompressed
  game assets).
- **Cross-repo dedup**: single global CAS across repos vs. per-repo —
  global is more storage-efficient but complicates GC and access control.
- **Delta chunks**: FastCDC handles insertions/deletions well already;
  a further optimization is chunk-level delta-encoding against a similar
  chunk, but this adds real complexity for marginal gain — likely v2 scope.

## 13. Production-Grade Improvements (Epic BuildPatchServices patterns)

Epic's BuildPatchServices (the system behind Fortnite / Epic Games Store
patching) solves the same chunked-CAS-over-CDN problem at game-content
scale. These patterns translate directly.

### 13.1 Local Chunk Cache Reservoir

Before fetching anything from the server, scan chunks the client already
has on disk — not just from the previous version of *this* file, but any
local install that might share content (other branches, other repos using
the same chunk store, prior checkouts). If a chunk hash is already local,
skip the network entirely.

```rust
struct LocalReservoir {
    root: PathBuf, // sharded local chunk store, same layout as remote CAS
}

impl LocalReservoir {
    /// Returns hashes not found locally — only these need negotiating
    /// with the server.
    fn missing(&self, wanted: &[Blake3Hash]) -> Vec<Blake3Hash> {
        wanted
            .iter()
            .filter(|h| !self.has(h))
            .cloned()
            .collect()
    }

    fn has(&self, hash: &Blake3Hash) -> bool {
        self.path_for(hash).exists()
    }

    fn path_for(&self, hash: &Blake3Hash) -> PathBuf {
        let hex = hash.to_hex();
        self.root.join(&hex[0..2]).join(&hex[2..4]).join(hex.as_str())
    }
}
```

The batch API request only needs to include chunks the reservoir doesn't
already have — cuts both bandwidth and batch payload size on repeat
checkouts.

### 13.2 A→B Delta Patch Precomputation

For predictable, high-traffic upgrade paths (most users moving from
manifest A to manifest B), precompute the diff server-side once instead
of making every client re-derive it.

```rust
struct DeltaPatch {
    from_manifest: ManifestId,
    to_manifest: ManifestId,
    added_chunks: Vec<Blake3Hash>,   // present in B, absent in A
    removed_chunks: Vec<Blake3Hash>, // present in A, absent in B — GC candidates
}

fn compute_delta(a: &Manifest, b: &Manifest) -> DeltaPatch {
    let a_set: HashSet<_> = a.chunks.iter().map(|c| c.hash).collect();
    let b_set: HashSet<_> = b.chunks.iter().map(|c| c.hash).collect();

    DeltaPatch {
        from_manifest: a.id.clone(),
        to_manifest: b.id.clone(),
        added_chunks: b_set.difference(&a_set).cloned().collect(),
        removed_chunks: a_set.difference(&b_set).cloned().collect(),
    }
}
```

Cache `DeltaPatch` results keyed by `(from, to)` server-side. A client
requesting an update between two manifests that already has a cached
delta gets the `added_chunks` list directly — no per-client set diff.

### 13.3 Manifest Diffing as a First-Class Operation

Since the manifest is just an ordered chunk-hash list, `compute_delta`
above doubles as your changelist generator — expose it as a CLI command
(`git cdc diff <manifest-a> <manifest-b>`) for CI size reporting and
code review ("this commit changed 3 of 40 chunks, +6.1 MiB / -2.0 MiB").

### 13.4 Per-Chunk Compression

Compress each chunk independently with zstd before it's written to the
CAS, so random-access fetch of a single chunk doesn't require
decompressing neighbors.

```rust
fn store_chunk(store: &impl ChunkStore, hash: &Blake3Hash, raw: &[u8]) -> Result<()> {
    let compressed = zstd::encode_all(raw, /* level */ 9)?;
    store.put(hash, Bytes::from(compressed))
}

fn load_chunk(store: &impl ChunkStore, hash: &Blake3Hash) -> Result<Bytes> {
    let compressed = store.get(hash)?;
    Ok(Bytes::from(zstd::decode_all(&compressed[..])?))
}
```

Note: the chunk hash is computed on *raw* bytes, before compression, so
dedup still works across chunks regardless of compression ratio
variance.

### 13.5 CDN-Backed Distribution

Keep the application server out of the data path entirely: batch
negotiation returns pre-signed CDN URLs (S3/CloudFront, MinIO/Cloudflare,
etc.) for both upload and download actions. The server's job is purely
metadata (which chunks exist, who's allowed to fetch them) — bandwidth
scales with the CDN, not your app tier.

### 13.6 File Tagging / Selective Sync

Extend the manifest with a tag field per file entry so clients can opt
out of groups they don't need (debug symbols, alternate-language assets,
platform-specific binaries):

```
cdc-manifest-v1
file assets/textures/hero_diffuse.png tags=textures,required
file docs/api-reference.pdf tags=docs,optional
chunk b3:9f2a1e... offset 0 length 2097152
...
```

Client-side sync command takes `--tags required,textures` to filter what
gets materialized on checkout.

### 13.7 Mutable Build Labels

Point a human-readable label (`live`, `staging`, `canary`) at a specific
manifest ID rather than distributing manifest IDs directly. Rollback
becomes "repoint the label," not "re-upload a build" — the old manifest
and its chunks are still in the CAS (until GC'd), so rollback is
effectively instant.

## 14. Building It as a Git Extension

Git has a built-in extension mechanism you get for free: any executable
named `git-<name>` on `$PATH` becomes invocable as `git <name>`. This is
exactly how `git-lfs` itself works — there's no special registration step,
Git just execs it and forwards argv.

### 14.1 The Mechanism

```
$ which git-lfs
/usr/local/bin/git-lfs

$ git lfs install
# Git resolved "lfs" → executed `git-lfs install` under the hood
```

So the whole "extension" surface is just: ship a single binary named
`git-cdc`, put it on `$PATH` (via cargo install, a package
manager, or a release tarball), and `git cdc <subcommand>` works
immediately — no Git-side registration, no plugin API, no recompiling Git.

### 14.2 CLI Command Surface (clap)

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "git-cdc", bin_name = "git cdc")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Register the filter driver (repo-local, or --global)
    Install {
        #[arg(long)]
        global: bool,
    },
    /// Add file patterns to .gitattributes
    Track {
        patterns: Vec<String>,
    },
    /// Show pending push/pull chunk state
    Status,
    /// Fetch chunk content for the current checkout
    Pull,
    /// Push local chunk content ahead of `git push`
    Push,
    /// List tracked files
    LsFiles,
    /// Diff two manifests (added/removed chunks)
    Diff { from: String, to: String },
    /// Sweep unreferenced chunks from the store
    Gc { #[arg(long)] dry_run: bool },
    // Internal — invoked by Git itself via the filter-process protocol,
    // not typically run by hand.
    #[command(hide = true)]
    Process,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Install { global } => cmd_install(global),
        Command::Track { patterns } => cmd_track(&patterns),
        Command::Status => cmd_status(),
        Command::Pull => cmd_pull(),
        Command::Push => cmd_push(),
        Command::LsFiles => cmd_ls_files(),
        Command::Diff { from, to } => cmd_diff(&from, &to),
        Command::Gc { dry_run } => cmd_gc(dry_run),
        Command::Process => run_filter_process(), // long-running, see 10
    }
}
```

### 14.3 Talking to the Surrounding Git Repo

The extension needs repo context (root dir, config values) the same way
any Git subcommand does — shell out to Git itself rather than
reimplementing repo discovery:

```rust
use std::process::Command as ProcessCommand;

fn repo_root() -> anyhow::Result<PathBuf> {
    let out = ProcessCommand::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()?;
    if !out.status.success() {
        anyhow::bail!("not inside a git repository");
    }
    Ok(PathBuf::from(String::from_utf8(out.stdout)?.trim()))
}

fn git_config_set(key: &str, value: &str, global: bool) -> anyhow::Result<()> {
    let mut cmd = ProcessCommand::new("git");
    cmd.arg("config");
    if global {
        cmd.arg("--global");
    }
    cmd.args([key, value]);
    cmd.status()?;
    Ok(())
}
```

`cmd_install` then just calls `git_config_set` for the three filter
entries from section 10 (`filter.cdc.clean`,
`filter.cdc.smudge`, `filter.cdc.process`), scoped
`--local` or `--global` depending on the flag — mirroring `git lfs
install [--global]` exactly.

### 14.4 Packaging and Distribution

- `cargo install --path .` for local dev.
- Prebuilt release binaries (GitHub Releases via `cargo-dist` or plain
  `cross` builds) named `git-cdc-<target>` so a install script can
  pick the right one and symlink it to `git-cdc` on `$PATH`.
- Homebrew tap / `.deb`/`.rpm` for OS package manager installs, same
  distribution shape `git-lfs` itself uses.
- Shell completions: clap's `clap_complete` crate generates them from the
  `Cli` struct directly — ship `git-cdc completions <shell>` as a
  hidden subcommand.

### 14.5 Fallback: Git Alias (no PATH install required)

For environments where dropping a binary on `$PATH` isn't convenient
(e.g. CI containers), a Git alias pointing at an absolute path works
identically:

```
git config --global alias.cdc '!/opt/tools/cdc'
```

Useful as a documented fallback, but the `git-<name>`-on-`$PATH` route
should be the primary, documented install path since it needs no extra
config and matches how `git-lfs` ships.

## 15. Alignment with the Git LFS Spec & Proposals

Cross-checked against the actual [git-lfs spec](https://github.com/git-lfs/git-lfs/blob/main/docs/spec.md)
and [proposals](https://github.com/git-lfs/git-lfs/tree/main/docs/proposals)
— a few of their design decisions are worth adopting directly rather than
reinventing, and one gives us a cleaner integration path than a fully
separate protocol.

### 15.1 Tighten the Manifest Encoding to Pointer-File Rules

LFS's pointer file spec is deliberately strict so that pointer blobs are
byte-for-byte reproducible (git diffs cleanly, and two implementations
hashing the same content produce an identical blob). Rules worth copying
into the manifest format (section 6):

- Text file, **UTF-8 only**.
- Each line `{key} {value}\n`, single space, LF only.
- Keys restricted to `[a-z0-9.-]`.
- `version` key always first; all other keys **sorted alphabetically**.
- Values MUST NOT contain CR/LF.
- **Unknown keys MUST be preserved** by any tool that parses and rewrites
  a manifest — this is what lets you add optional fields (e.g. the `tags`
  field from section 13.6) later without breaking older parsers.

Revised manifest header, matching this discipline:

```
version https://git-cdc.dev/spec/v1
chunk-avg 2097152
chunk-max 8388608
chunk-min 524288
oid blake3:9c1f...        <- whole-file oid, kept for compatibility/verification
size 41943040
```

followed by the chunk list (which isn't part of the strict key/value
header — same separation LFS uses between the pointer header and any
trailing pointer-extension lines).

### 15.2 Batch API: Match the Real Field Names

The actual [Batch API](https://github.com/git-lfs/git-lfs/blob/main/docs/api/batch.md)
request shape is:

```json
{
  "operation": "download",
  "transfers": ["basic"],
  "ref": { "name": "refs/heads/main" },
  "objects": [{ "oid": "...", "size": 123 }],
  "hash_algo": "sha256"
}
```

Two fields worth lifting directly into section 7's batch design:

- **`hash_algo`** — don't hardcode BLAKE3 in the protocol; advertise it as
  `"hash_algo": "blake3"` in the request, same as LFS advertises
  `"sha256"` today. Costs nothing now, and gives you algorithm agility
  later (e.g. if a server wants SHA-256 for compliance reasons) without a
  protocol version bump.
- **`ref`** — LFS added this so servers can apply per-branch auth/ACLs.
  Worth including from day one rather than retrofitting: chunk access
  control (e.g. "only CI can push chunks referenced from `main`") is a
  realistic need once this is multi-user.

### 15.3 Transfer Adapter Model — Adopt Instead of Reinventing

LFS's [transfer adapter proposal](https://github.com/git-lfs/git-lfs/blob/main/docs/proposals/transfer_adapters.md)
already solves "how do we support more than plain HTTP GET/PUT" cleanly,
and it's a better fit than hardcoding CDN pre-signed URLs as *the* answer
in section 13.5:

- Client advertises supported mechanisms via `accept-transfers` in the
  batch request (analogous to `transfers` above); server picks one and
  echoes it back via `transfer` in the response.
- The **default/required adapter stays simple HTTP GET/PUT** — cdc
  should guarantee this works with zero extra config, exactly as LFS
  keeps `basic` as the mandatory fallback.
- Additional adapters (CDN pre-signed URLs, resumable range requests,
  a custom chunk-aware adapter) are opt-in and negotiated, not assumed.
- **External adapter process model** is directly reusable: for a
  transfer mechanism cdc doesn't implement natively, spawn a
  long-lived external process and talk to it over stdin/stdout with a
  small line-based protocol (oid in → progress/completion events out).
  This is the same shape as the filter-process protocol in section 10 —
  worth sharing one framing/codec between the two rather than building
  two different stream protocols.

Practical effect on the doc: rename what section 13.5 calls "CDN-backed
distribution" to be **one transfer adapter among several**, not a
hardcoded assumption — the batch response's `actions` block already
carries per-object URLs, so nothing else needs to change, just the
framing.

### 15.4 SSH Transport as a Second-Class Citizen, Not an Afterthought

LFS also has a documented [SSH-based transfer proposal](https://github.com/git-lfs/git-lfs/blob/main/docs/proposals/ssh_adapter.md)
using Git's own pkt-line format over an SSH-invoked remote command
(`ssh {server} git-lfs-transfer {path} {operation}`), avoiding HTTPS
entirely. Since chunk negotiation is just "which oids do you have," this
maps cleanly onto cdc too:

```
ssh {user}@{server} git-cdc-transfer {repo-path} {operation}
```

with a pkt-line capability advertisement (`version=1`, optional
`locking`) followed by batch-style oid/action lines. Worth keeping as a
documented alternative transport in the same spirit LFS treats it — not
required for v1, but the batch protocol (section 7) should stay
transport-agnostic enough that an SSH framing can sit underneath it
without a redesign.

### 15.5 Summary of Doc Changes to Carry Forward

| Section | Change |
|---|---|
| 6 (Manifest Format) | Adopt strict LFS-style key encoding + preserve-unknown-keys rule |
| 7 (Batch API) | Add `hash_algo` and `ref` fields to request/response |
| 13.5 (CDN Distribution) | Reframe as one transfer adapter, negotiated via `accept-transfers`/`transfer`, not hardcoded |
| New | Document SSH pkt-line transport as an optional, spec-compatible alternative to HTTPS batch |

## 16. Performance: Concurrency and Memory Efficiency

The naive version of every step in this doc so far reads a whole file
into memory, chunks it, hashes it, and then transfers chunks one at a
time. None of that scales past a few hundred MB. This section covers
what changes for large files and large repos.

### 16.1 Stream/mmap Instead of Loading Whole Files

Never `fs::read()` a multi-GB file into a `Vec<u8>`. Two options,
depending on file size and platform:

- **Memory-mapped I/O** (`memmap2` crate) — let the OS page cache do the
  work; FastCDC can scan over the mapped slice without a manual read
  loop, and the OS evicts pages under memory pressure instead of your
  process OOMing.
- **Streaming reads** for very large files or when mmap isn't available
  (e.g. some CI/network filesystems) — read in bounded windows with a
  small overlap buffer so chunk boundaries at window edges are still
  found correctly.

```rust
use memmap2::Mmap;
use std::fs::File;

fn chunk_file_mmap(path: &Path) -> anyhow::Result<Vec<Chunk>> {
    let file = File::open(path)?;
    let mmap = unsafe { Mmap::map(&file)? }; // read-only, zero-copy view
    let chunker = FastCDC::new(&mmap, MIN_SIZE, AVG_SIZE, MAX_SIZE);
    Ok(chunker
        .map(|e| Chunk::from_slice(&mmap[e.offset..e.offset + e.length]))
        .collect())
}
```

Either way, hashing should happen on slices/views into the mapped or
buffered region — never a fresh heap copy per chunk. `blake3::hash()`
operates directly on `&[u8]`, so a chunk only needs `(offset, length)`
into the mmap until it's actually queued for upload.

### 16.2 Parallel Chunk Hashing

Chunk *boundary detection* (FastCDC) is inherently sequential — the
rolling hash needs to scan forward — but **hashing already-cut chunks is
embarrassingly parallel**. Split the two phases:

```rust
use rayon::prelude::*;

fn hash_chunks_parallel(mmap: &Mmap, boundaries: &[ChunkBoundary]) -> Vec<Chunk> {
    boundaries
        .par_iter()
        .map(|b| {
            let bytes = &mmap[b.offset..b.offset + b.length];
            Chunk {
                hash: blake3::hash(bytes),
                offset: b.offset as u64,
                length: b.length as u32,
            }
        })
        .collect()
}
```

BLAKE3 itself is already SIMD-parallel *within* a single hash for large
inputs (`blake3::Hasher::update_rayon` with the `rayon` feature enabled
uses a work-stealing tree internally for big chunks) — so for your
~2 MiB average chunk size, letting `rayon` parallelize *across* chunks
is the bigger win; within-chunk SIMD parallelism matters more once
individual chunks get large (tens of MB), which shouldn't happen given
the max-chunk-size bound from section 4.

### 16.3 Bounded Concurrent Network I/O

Don't fire off one upload/download task per chunk unbounded — that
either exhausts file descriptors/sockets or overwhelms the server.
Bound concurrency with a semaphore and let `tokio` schedule the rest:

```rust
use tokio::sync::Semaphore;
use std::sync::Arc;

async fn upload_chunks(
    client: &reqwest::Client,
    chunks: Vec<(Blake3Hash, Bytes, String)>, // hash, data, presigned URL
    max_concurrent: usize,
) -> anyhow::Result<()> {
    let sem = Arc::new(Semaphore::new(max_concurrent));
    let mut tasks = Vec::with_capacity(chunks.len());

    for (hash, data, url) in chunks {
        let sem = sem.clone();
        let client = client.clone();
        tasks.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap(); // released on drop
            client.put(&url).body(data).send().await?.error_for_status()?;
            Ok::<_, anyhow::Error>(hash)
        }));
    }

    for t in tasks {
        t.await??;
    }
    Ok(())
}
```

`max_concurrent` should be configurable (`cdc.concurrenttransfers`,
matching LFS's own `lfs.concurrenttransfers` setting from section 15) —
default around 8, since that's roughly where diminishing returns kick in
against a typical CDN before you're just adding contention.

### 16.4 Zero-Copy Buffers End to End

Use `bytes::Bytes` (reference-counted, cheaply cloneable) rather than
`Vec<u8>` for chunk payloads once they leave the mmap. Cloning a `Bytes`
bumps a refcount instead of copying the buffer — matters when the same
chunk needs to go to both the local reservoir cache (13.1) and the
network upload task simultaneously.

```rust
// Cheap: shares the underlying allocation
let payload: Bytes = Bytes::copy_from_slice(&mmap[range]); // one copy, unavoidable at the mmap boundary
let for_cache = payload.clone();   // refcount bump only
let for_upload = payload.clone();  // refcount bump only
```

The one unavoidable copy is at the mmap→owned-buffer boundary (mmap
pages aren't `'static` and can't be handed to a spawned async task
directly); everything downstream of that should clone the `Bytes`
handle, never the bytes themselves.

### 16.5 Chunk Size vs. Parallelism Tradeoff

Smaller average chunk size → more dedup granularity but more
per-chunk overhead (hash computation calls, HTTP requests, manifest
entries). Larger average chunk size → fewer, bigger transfers but worse
dedup on small edits. Concretely:

| Avg chunk size | Chunks per 1 GB file | Dedup granularity | HTTP overhead |
|---|---|---|---|
| 512 KiB | ~2000 | Fine | High (many small requests) |
| 2 MiB (default) | ~500 | Balanced | Balanced |
| 8 MiB | ~125 | Coarse | Low |

Make this configurable per `.gitattributes` pattern (large uncompressible
binaries like game assets can use bigger chunks; frequently-hand-edited
binaries benefit from smaller ones) rather than one global constant.

### 16.6 Connection Reuse and Multiplexing

Use a single `reqwest::Client` (or equivalent) for the whole session —
it pools connections internally. Prefer HTTP/2 where the server supports
it (`reqwest::Client::builder().http2_prior_knowledge()` or ALPN
negotiation) so many small chunk requests multiplex over one TCP
connection instead of paying a new TLS handshake per request.

### 16.7 Fast Local-Existence Checks (Reservoir, section 13.1)

`LocalReservoir::has()` as sketched earlier does a filesystem `stat()`
per hash — fine for hundreds of chunks, slow for hundreds of thousands.
Front it with an in-memory **Bloom filter** built once at startup from a
directory listing, so most "definitely not here" checks skip the
syscall entirely:

```rust
struct LocalReservoir {
    root: PathBuf,
    bloom: bloomfilter::Bloom<Blake3Hash>, // false positives OK, false negatives not
}

impl LocalReservoir {
    fn has(&self, hash: &Blake3Hash) -> bool {
        // Bloom says "definitely absent" -> skip the stat() entirely.
        if !self.bloom.check(hash) {
            return false;
        }
        self.path_for(hash).exists() // confirm on possible-positive
    }
}
```

Rebuild or incrementally update the bloom filter whenever chunks are
added to the local store (e.g. after a pull), not on every lookup.

### 16.8 Measuring Before Tuning

Before adjusting any of the above, instrument with `tracing` +
`tracing-flame` or `tokio-console` to find the actual bottleneck first —
CDC boundary-finding, hashing, disk I/O, and network transfer have very
different scaling characteristics, and it's easy to parallelize the part
that wasn't the bottleneck. As a starting benchmark harness,
`criterion` on the chunker alone (boundary-finding + hashing, no
network) isolates CPU-bound cost from I/O-bound cost cleanly.
