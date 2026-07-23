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
- `push`/`pull`/`gc` with no remote configured name every option in the
  error: `cdc.url` (server), `cdc.store.scheme` (serverless), or
  `cdc.ssh.remote` (ssh).
- `clean` fails hard on invalid `cdc.chunk.*` config, naming the key.
- `smudge` with missing chunks emits manifest text and succeeds (safe
  degradation); on corrupt data it fails loudly instead of emitting bytes.

## Switching branches: do I need to `pull` again?

`git checkout <branch>` reconstructs tracked files from whatever's already
in the **local** chunk store — it never touches the remote. So:

- **Chunks already local** (you made that commit yourself, or already
  pulled/checked out that version on this machine): checkout materializes
  the real file immediately, no `pull` needed.
- **Chunks missing locally** (a fresh clone, a branch you've never checked
  out before, or one whose chunks a local `gc` swept away): checkout still
  succeeds — as a safe degradation the file is left holding manifest text
  instead of real content, with a note on stderr:
  `chunks not in local store; run 'git cdc pull' to fetch file content`.
  Run `git cdc pull` afterward to fetch what's missing and materialize the
  real bytes.

Rule of thumb: after fetching commits you haven't worked with locally
before, run `git cdc pull` once you're on the branch you want — switching
between branches you've already visited on this machine needs no further
pull.

**Gotcha:** right after a `pull` materializes a file, `git checkout
<other-branch>` can refuse with *"Your local changes... would be
overwritten"* — even though the content is provably unchanged (re-running
`git cdc clean` on it reproduces the exact manifest already committed).
This is git's stat cache being stale, not a real conflict: `pull` writes
real bytes straight to the worktree without going through git's own
add/checkout machinery, so git's fast "did this file change" check (file
size/mtime) sees a mismatch and assumes the worst without actually
re-running the clean filter to check. Fix: run `git add <path>` first — it
detects there's no real change to stage, just refreshes git's cache — then
the checkout proceeds normally.
