# Getting Started

## Install

Build from source (Rust toolchain required):

```sh
git clone https://github.com/amjadjibon/git-cdc
cd git-cdc
cargo install --path crates/core      # installs `git-cdc` into ~/.cargo/bin
cargo install --path crates/server    # optional: `git-cdc-server`
```

With `git-cdc` on your `PATH`, git resolves `git cdc <command>`
automatically (the standard `git-<name>` extension mechanism).

## First repo, serverless (5 minutes)

The quickest path needs no server — just an S3-compatible bucket. For a
local trial, MinIO or RustFS in docker works:

```sh
docker run -d --name minio -p 9000:9000 -p 9001:9001 minio/minio server /data --console-address :9001
export AWS_ACCESS_KEY_ID=minioadmin AWS_SECRET_ACCESS_KEY=minioadmin
aws --endpoint-url http://127.0.0.1:9000 s3api create-bucket --bucket my-chunks
```

Then in your repo:

```sh
git cdc install                                  # filters + pre-push hook
git config cdc.s3.bucket my-chunks
git config cdc.s3.endpoint http://127.0.0.1:9000 # MinIO only — omit for AWS
git config cdc.s3.force-path-style true         # MinIO only
git cdc track '*.bin' '*.dat'                    # writes .gitattributes

git add . && git commit -m "add assets"
git push                                         # hook uploads chunks first
```

## First repo, with a server

If you want central token auth instead of IAM:

```sh
git-cdc-server --root /srv/cdc --token my-secret &

git cdc install
git config cdc.url http://127.0.0.1:8077
git config cdc.token my-secret
git cdc track '*.bin'
git add . && git commit -m "add assets" && git push
```

## Cloning a repo that uses git-cdc

A fresh clone contains manifests, not file content — and that's fine:
checkout succeeds, tracked files hold manifest text, and git-cdc tells you
what to do:

```sh
git clone <repo> && cd <repo>
git cdc install          # per-clone: filters are repo-local
git config cdc.url http://your-server:8077    # or the cdc.s3.* trio
git config cdc.token <secret>
git cdc pull             # fetch chunks, materialize tracked files
```

`git cdc pull` fetches only the chunks the current checkout needs, verifies
every hash, and rewrites the tracked files in place.

## What `git cdc install` actually does

- Sets `filter.cdc.process = git-cdc filter-process` — one long-running
  filter process per git operation (the gitattributes filter-process
  protocol), instead of one process per file. `filter.cdc.clean` /
  `filter.cdc.smudge` are also registered as the fallback for git < 2.11.
  (Repo-local; `--global` available.)
- Writes a pre-push hook running `git cdc push`, so chunks always reach the
  remote store before the manifests referencing them reach the git remote.
  An existing pre-push hook is never overwritten — you get a warning and
  wire it in yourself.
- Deliberately does **not** set `filter.cdc.required`: that's what lets a
  chunk-less clone check out safely instead of hard-failing.
