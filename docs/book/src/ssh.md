# SSH Transport

Any host you can ssh into can hold the chunk store — no git-cdc server
process, no bucket. It's the same model git itself uses for
`git push` over SSH: the CLI runs your `ssh` and starts the counterpart
binary on the far end.

```sh
git cdc install
git config cdc.ssh.remote user@host
git config cdc.ssh.path /srv/cdc-chunks
git cdc track '*.dat'
```

`push`/`pull`/`gc` then spawn:

```text
ssh user@host git-cdc stdio --root /srv/cdc-chunks
```

and speak a small pkt-line request protocol over the pipe (`has`, `put`,
`get`, `list`, `remove`). Because it's your ssh, everything in
`~/.ssh/config` applies — keys, agents, ProxyJump, aliases.

## Requirements

- `git-cdc` installed on the remote host and on `PATH` for
  non-interactive ssh sessions (the same requirement git has for
  `git-upload-pack`).
- Write access to the chunk root directory. Access control *is* your ssh
  and filesystem permissions — there is no separate token.

## Semantics

- **push** — one `list` of the remote store, then uploads only the
  missing chunks (envelope/compressed form, no re-encoding).
- **pull** — fetches missing chunks, hash-verified locally before
  admission.
- **gc** — like serverless mode, the CLI owns the sweep: remote file
  mtimes from `list` feed the `--grace-secs` window.
- Remote selection precedence: `cdc.opendal.scheme` > `cdc.ssh.remote` >
  `cdc.url`.

`cdc.ssh.command` (advanced) replaces the entire ssh invocation with an
arbitrary command — useful for tests and unusual transports; the test
suite uses it to run the stdio server as a local subprocess.
