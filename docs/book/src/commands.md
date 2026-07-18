# Command Reference

All commands run as `git cdc <command>` (git finds the `git-cdc` binary on
`PATH`).

| Command | What it does |
| ------- | ------------ |
| `git cdc install [--global]` | Register the clean/smudge filter driver; repo-local installs also get a pre-push hook running `git cdc push`. Never overwrites an existing hook. |
| `git cdc track <pattern>...` | Append `<pattern> filter=cdc -text` lines to `.gitattributes` (idempotent). |
| `git cdc pull` | Fetch chunks missing for the current checkout, then materialize tracked files whose worktree still holds manifest text. Verifies every chunk hash and each file's oid. |
| `git cdc push` | Collect every chunk referenced by any manifest in history, negotiate with the remote, upload only what's missing. Run automatically by the pre-push hook. |
| `git cdc gc [--dry-run] [--grace-secs N]` | Mark-and-sweep unreferenced chunks locally and remotely. See [Garbage Collection](gc.md). |
| `git cdc diff <a> <b>` | Compare two manifest files: added/removed chunks and byte counts. |
| `git cdc clean` / `git cdc smudge` | The filter endpoints (hidden; invoked by git, not by hand). |

## Exit behavior worth knowing

- `push` fails hard if the remote wants a chunk the local store doesn't
  have (e.g. a clone that never pulled) — the error says to run
  `git cdc pull` first. Combined with the pre-push hook, this guarantees a
  git remote never references chunks the chunk store lacks.
- `push`/`pull`/`gc` with no remote configured name both options in the
  error: `cdc.url` (server) or `cdc.s3.bucket` (serverless).
- `clean` fails hard on invalid `cdc.chunk.*` config, naming the key.
- `smudge` with missing chunks emits manifest text and succeeds (safe
  degradation); on corrupt data it fails loudly instead of emitting bytes.
