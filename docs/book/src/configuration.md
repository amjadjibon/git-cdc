# Configuration

All client settings are plain git config. Repo-local values override
globals, so you can keep machinery global and per-project choices local.

## Remote selection

| Key | Meaning |
| --- | ------- |
| `cdc.url` | git-cdc-server base URL (server mode) |
| `cdc.token` | Bearer token for that server |
| `cdc.s3.bucket` | S3 bucket (serverless mode) — **its presence selects S3 mode over `cdc.url`** |
| `cdc.s3.prefix` | Key prefix inside the bucket, e.g. `chunks/` (optional) |
| `cdc.s3.endpoint` | Custom endpoint for MinIO/RustFS/R2 — omit for AWS |
| `cdc.s3.force-path-style` | `true` for MinIO (path-style addressing) |
| `cdc.ssh.remote` | `user@host` for [SSH transport](ssh.md) |
| `cdc.ssh.path` | Chunk root directory on that host |
| `cdc.ssh.command` | Advanced: replace the whole ssh invocation |

Precedence when several are set: `cdc.s3.bucket` > `cdc.ssh.remote` >
`cdc.url`.

S3 credentials are **never** git config — they come from the standard AWS
chain (`AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY` env vars, `~/.aws`
profiles, or instance metadata).

## Filter registration

Written by `git cdc install`:

```ini
[filter "cdc"]
	process = git-cdc filter-process
	clean = git-cdc clean
	smudge = git-cdc smudge
```

`process` keeps one filter alive per git operation (fast checkouts with
many tracked files); `clean`/`smudge` are the fallback for git < 2.11.

Safe to keep global — filters only activate for paths tracked in a repo's
`.gitattributes`.

## Chunk size tuning

FastCDC bounds are configurable per repo (defaults 512 KiB / 2 MiB /
8 MiB):

```sh
git config cdc.chunk.min 64k    # 64 B – 1 MiB
git config cdc.chunk.avg 256k   # 256 B – 4 MiB
git config cdc.chunk.max 1m     # 1 KiB – 16 MiB   (min ≤ avg ≤ max)
```

Values are bytes; git's `k`/`m`/`g` suffixes work. Out-of-range or
misordered values fail the clean filter with an error naming the key —
invalid config never silently falls back to defaults.

Guidance:

- **Smaller chunks** → finer dedup (small edits invalidate less), but more
  objects: ~90 bytes of manifest per chunk, one store object and one upload
  each. Good for files of a few MB that change often.
- **Larger chunks** → fewer round-trips and less overhead for huge,
  rarely-edited assets.
- The conventional shape is `min = avg/4`, `max = avg×4`; usually you only
  really choose `avg`.

The settings apply when files are *chunked* (`git add`). Manifests record
the values used, and readers reassemble purely by chunk hash — so changing
the config never invalidates existing history.

> **Caveat:** all clients of a repo should use the same values. Different
> configs re-clean identical content into different (equally valid)
> manifests, which shows up as spurious diffs. Set them repo-locally, not
> `--global`.

## Global setup with an include file

The repo ships a commented sample of every setting,
[`.gitconfig.cdc`](https://github.com/amjadjibon/git-cdc/blob/main/.gitconfig.cdc).
Wire it into your global config with git's native include mechanism:

```sh
cp .gitconfig.cdc ~/.gitconfig.cdc
git config --global include.path ~/.gitconfig.cdc
```

Then edit `~/.gitconfig.cdc`. The pre-push hook still needs a one-time
`git cdc install` per repo (git hooks are per-repo).

## GC grace period

`git cdc gc --grace-secs <n>` (default 86400): unreferenced chunks younger
than this survive a sweep, protecting just-cleaned but not-yet-committed
chunks and in-flight uploads. The server has its own `--grace-secs` flag
for server-side sweeps; in serverless mode the CLI's value applies to the
bucket. See [Garbage Collection](gc.md).
