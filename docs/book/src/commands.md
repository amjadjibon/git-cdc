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
| `git cdc drop [--chunks] [--dry-run] --all \| <path>...` | Reclaim local disk: rewrite tracked files back to their committed manifest, and with `--chunks` remove local chunks the remote already holds. Reversed by `git cdc pull`. |
| `git cdc diff <a> <b>` | Compare two manifest files: added/removed chunks and byte counts. |
| `git cdc clean` / `git cdc smudge` | The filter endpoints (hidden; invoked by git, not by hand). |

## Reclaiming local disk with `drop`

A tracked file occupies local disk **twice**: the hydrated copy in your
worktree, and the chunks backing it in `.git/cdc/objects`. `git cdc drop`
reclaims both halves.

```sh
git cdc drop --chunks big.bin    # one file (paths may be cwd-relative)
git cdc drop --chunks --all      # every tracked file in the index
git cdc drop --dry-run --all     # what would it free?
git cdc pull                     # rehydrate
```

Dropping rewrites the file with its committed manifest — byte for byte the
blob that is already in git, and exactly the state a fresh clone is in
before `pull`. Because the clean filter passes manifest text through
unchanged, git still sees an unmodified worktree.

Without `--chunks` only the worktree half is reclaimed; the chunks stay
local and `pull` restores the file without touching the network.

### What it refuses to do

- **Delete a chunk the remote doesn't have.** `--chunks` asks the remote
  what it holds and keeps anything it can't confirm, naming each one. A
  chunk that exists only locally is unrecoverable once deleted, so
  `--chunks` requires a configured remote and aborts — before touching any
  file — when there isn't one.
- **Overwrite uncommitted changes.** Edits that were never `git add`ed were
  never chunked either, so they live in no store, local or remote. `drop`
  compares each file against its manifest's `oid` and skips mismatches.
- **Drop everything by default.** A bare `git cdc drop` is an error; say
  `--all` or name paths.

`drop` is the mirror image of [`gc`](gc.md): gc sweeps chunks that nothing
references any more, while drop deletes chunks that *are* referenced,
deliberately, because the remote can serve them again.

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
