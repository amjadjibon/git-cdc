# Garbage Collection

Chunks referenced by no manifest anywhere in history are garbage — they
accumulate when branches are deleted, history is rewritten, or files are
re-chunked with different parameters. `git cdc gc` removes them from the
local store and the remote.

```sh
git cdc gc --dry-run --grace-secs 0   # see what would go
git cdc gc                            # sweep local + remote
```

## Mark and sweep, client-driven

The **live set** is computed on the client: walk every ref
(`git rev-list --all --objects`), read every manifest blob in history, and
collect every referenced chunk hash. Anything reachable from *any* branch,
tag, or remote-tracking ref is live — deleting a branch is precisely what
makes its unique chunks collectable.

- **Local sweep**: unreferenced chunks in `.git/cdc/objects` are removed.
- **Server mode**: the live set is POSTed to `/gc`; the server deletes
  unreferenced chunks past *its* grace period (`--grace-secs` server flag)
  and reports `deleted` / `kept_live` / `kept_grace`.
- **Serverless mode**: the CLI lists the bucket prefix and deletes
  unreferenced chunks whose `LastModified` is older than the CLI's
  `--grace-secs`.

## The grace period

A chunk can be legitimately unreferenced *for now*: just cleaned but not
yet committed, or mid-upload from another client. The grace period (default
24 h) keeps any unreferenced chunk younger than the threshold. Use
`--grace-secs 0` only when you know no other writes are in flight.

## Safety properties

- `--dry-run` reports without deleting, on both local and remote.
- GC never deletes anything in the live set, and foreign objects under an
  S3 prefix (non-hash keys) are never touched.
- Because the client computes liveness from *its* clone, run gc from a
  clone that has all refs (`git fetch --all --prune` first if in doubt).
  A stale clone's live set is smaller than reality — the grace period is
  the backstop against races, not against sweeping from a months-old
  checkout.
