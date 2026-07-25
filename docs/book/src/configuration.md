# Configuration

All client settings are plain git config. Repo-local values override
globals, so you can keep machinery global and per-project choices local.

## Remote selection

| Key | Meaning |
| --- | ------- |
| `cdc.url` | git-cdc-server base URL (server mode) |
| `cdc.token` | Bearer token for that server |
| `cdc.store.scheme` | Any OpenDAL service (`s3`, `azblob`, `gcs`, `dropbox`, ...) — **its presence selects serverless mode over `cdc.url`** |
| `cdc.store.option` | Service option as `KEY=VALUE`, repeatable (`git config --add`), e.g. `bucket=my-chunks` |
| `cdc.store.prefix` | Key prefix inside the service, e.g. `chunks/` (default) |
| `cdc.ssh.remote` | `user@host` for [SSH transport](ssh.md) |
| `cdc.ssh.path` | Chunk root directory on that host |
| `cdc.ssh.command` | Advanced: replace the whole ssh invocation |

Precedence when several are set: `cdc.store.scheme` > `cdc.ssh.remote` >
`cdc.url`.

Service credentials are **never** git config — they come from each
service's standard chain (e.g. for S3: `AWS_ACCESS_KEY_ID`/
`AWS_SECRET_ACCESS_KEY` env vars, `~/.aws` profiles, or instance metadata).

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


### `cdc.chunk.*` across a team

Chunk boundaries are reproducible only for a given parameter set, so a clone
configured differently would re-chunk files nobody edited — rewriting their
manifests on the next `git add` and uploading a second copy of content the
remote already has under different hashes.

git-cdc prevents that: **a file already tracked is re-chunked with the
parameters its committed manifest records**, not with this clone's
`cdc.chunk.*`. A clone set to `avg 1m` pulling a repo committed at `avg 2m`
keeps producing the 2 MiB-average manifests, so `git status` stays clean and
`push` uploads nothing new.

What the local config still governs:

- **New files.** Nothing is committed to inherit from, so they use this repo's
  settings. That is the one way a repo ends up with mixed parameters, and
  `git cdc pull` prints a note when it sees the mismatch, along with the
  config that matches the rest of the repo.
- **Everything, when you opt out.** Set `cdc.chunk.inherit false` to re-chunk
  deliberately onto new settings:

  ```sh
  git config cdc.chunk.inherit false
  git config cdc.chunk.avg 1m
  git add --renormalize .        # re-chunks every tracked file
  ```

Inheriting reads the committed manifest through a single long-lived
`git cat-file --batch`, so a filter run over thousands of files spawns one
child process, not thousands. A manifest whose recorded parameters fastcdc
would reject is ignored rather than inherited.

Nothing here affects correctness: reassembly goes by chunk hash, so `pull` and
`checkout` reproduce exact bytes whatever the parameters were. What is at
stake is dedup.
