//! Chunk movement between local and remote stores: push, pull, gc.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, bail};
use git_cdc_core::manifest::is_manifest;
use git_cdc_core::protocol::{ObjectSpec, Operation};
use git_cdc_core::store::ChunkStore;

use crate::git::{all_manifests, git_out, index_manifests, local_store, oid_str, repo_root};
use crate::remote::{Remote, remote};

/// Chunks the local store has but a remote (identified by its own listing)
/// does not — the upload diff shared by every serverless remote kind.
fn pending_uploads(
    chunks: &HashMap<blake3::Hash, u64>,
    present: &HashSet<blake3::Hash>,
) -> Vec<blake3::Hash> {
    chunks
        .keys()
        .filter(|h| !present.contains(*h))
        .copied()
        .collect()
}

/// Which of `chunks` the configured remote already holds — the safety gate
/// for `drop --chunks`: a chunk the remote has is refetchable by `git cdc
/// pull`, so deleting the local copy is reversible no matter which other
/// files or commits share it. Unlike `cmd_gc`, a missing remote is an error
/// here, not a skipped step.
fn remote_present(chunks: &HashMap<blake3::Hash, u64>) -> Result<HashSet<blake3::Hash>> {
    if chunks.is_empty() {
        return Ok(HashSet::new());
    }
    // The serverless remotes answer with one listing, same as `cmd_push`.
    let intersect = |present: HashSet<blake3::Hash>| -> HashSet<blake3::Hash> {
        present
            .into_iter()
            .filter(|h| chunks.contains_key(h))
            .collect()
    };
    Ok(match remote()? {
        Remote::Http(client) => {
            let objects: Vec<ObjectSpec> = chunks
                .iter()
                .map(|(h, size)| ObjectSpec {
                    oid: oid_str(h),
                    size: *size,
                })
                .collect();
            let resp = client.batch(Operation::Download, objects)?;
            let mut present = HashSet::new();
            for obj in &resp.objects {
                // An error or a missing download action both mean "not there".
                if obj.error.is_some()
                    || obj
                        .actions
                        .as_ref()
                        .and_then(|a| a.download.as_ref())
                        .is_none()
                {
                    continue;
                }
                present.insert(git_cdc_core::manifest::parse_hash(&obj.oid)?);
            }
            present
        }
        Remote::Opendal { store, rt } => rt.block_on(async {
            anyhow::Ok(intersect(
                store.list().await?.into_iter().map(|(h, _)| h).collect(),
            ))
        })?,
        Remote::Ssh(mut ssh) => intersect(ssh.list()?.into_iter().map(|(h, _)| h).collect()),
    })
}

/// Tracked paths named by `args`, resolved through `git ls-files
/// --full-name` so a cwd-relative path (or a glob) from a subdirectory
/// matches the repo-relative paths `index_manifests` returns.
fn resolve_paths(
    args: &[String],
    tracked: &[(String, git_cdc_core::manifest::Manifest)],
) -> Result<Vec<usize>> {
    let mut out = Vec::new();
    for arg in args {
        let listed = git_out(&[
            "-c",
            crate::git::QUOTE_PATH,
            "ls-files",
            "--full-name",
            "--",
            arg,
        ])?;
        let hits: Vec<&str> = listed
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        if hits.is_empty() {
            bail!("{arg}: no such file tracked by git");
        }
        for hit in hits {
            let i = tracked
                .iter()
                .position(|(p, _)| p == hit)
                .with_context(|| {
                    format!("{hit} is not a git-cdc file (no filter=cdc manifest in the index)")
                })?;
            if !out.contains(&i) {
                out.push(i);
            }
        }
    }
    Ok(out)
}

/// What a GC sweep should do with one chunk: shared mark-and-sweep + grace
/// rule applied by the local sweep and every serverless remote sweep alike.
/// An unreadable/missing mtime always keeps the chunk — GC must never delete
/// something whose age it cannot confirm.
enum SweepAction {
    KeepLive,
    KeepGrace,
    Delete,
}

fn sweep_decision(
    is_live: bool,
    modified: Option<SystemTime>,
    grace: Duration,
    now: SystemTime,
) -> SweepAction {
    if is_live {
        return SweepAction::KeepLive;
    }
    let old_enough = modified
        .and_then(|mtime| now.duration_since(mtime).ok())
        .is_some_and(|age| age >= grace);
    if old_enough {
        SweepAction::Delete
    } else {
        SweepAction::KeepGrace
    }
}

pub fn cmd_push() -> Result<()> {
    let store = local_store()?;

    let mut chunks: std::collections::HashMap<blake3::Hash, u64> = Default::default();
    for m in all_manifests()? {
        for c in &m.chunks {
            chunks.insert(c.hash, c.length as u64);
        }
    }
    if chunks.is_empty() {
        eprintln!("git-cdc: no manifests found, nothing to push");
        return Ok(());
    }
    let total = chunks.len();

    let uploaded = match remote()? {
        Remote::Http(client) => {
            let objects: Vec<ObjectSpec> = chunks
                .iter()
                .map(|(h, size)| ObjectSpec { oid: oid_str(h), size: *size })
                .collect();
            let resp = client.batch(Operation::Upload, objects)?;
            let mut pending: Vec<(String, blake3::Hash)> = Vec::new();
            for obj in &resp.objects {
                let Some(action) = obj.actions.as_ref().and_then(|a| a.upload.as_ref()) else {
                    continue; // server already has it — the dedup win
                };
                let hash = git_cdc_core::manifest::parse_hash(&obj.oid)?;
                if !store.has(&hash) {
                    bail!(
                        "server wants {} but it is not in the local store — run `git cdc pull` first (a dropped chunk from `git cdc drop --chunks` needs refetching)",
                        obj.oid
                    );
                }
                pending.push((action.href.clone(), hash));
            }

            // Bounded concurrency: chunks are ~MBs, so round-trips dominate
            // on high-latency links; a few workers pulling from a shared
            // index overlap them without flooding the server.
            const UPLOAD_WORKERS: usize = 4;
            let next = std::sync::atomic::AtomicUsize::new(0);
            std::thread::scope(|scope| -> Result<()> {
                let workers: Vec<_> = (0..UPLOAD_WORKERS.min(pending.len()))
                    .map(|_| {
                        scope.spawn(|| -> Result<()> {
                            loop {
                                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                let Some((href, hash)) = pending.get(i) else {
                                    return Ok(());
                                };
                                client.upload(href, store.get_encoded(hash)?)?;
                            }
                        })
                    })
                    .collect();
                for w in workers {
                    w.join().expect("upload worker panicked")?;
                }
                Ok(())
            })?;
            pending.len()
        }
        Remote::Opendal { store: remote_store, rt } => rt.block_on(async {
            // One paginated listing beats a HeadObject per chunk.
            let present: HashSet<blake3::Hash> =
                remote_store.list().await?.into_iter().map(|(h, _)| h).collect();
            let pending = pending_uploads(&chunks, &present);
            for hash in &pending {
                let data = store.get_encoded(hash).with_context(|| {
                    format!("remote needs {} but it is not in the local store — run `git cdc pull` first (a dropped chunk from `git cdc drop --chunks` needs refetching)", oid_str(hash))
                })?;
                remote_store.put_encoded(hash, data).await?;
            }
            anyhow::Ok(pending.len())
        })?,
        Remote::Ssh(mut ssh) => {
            // Same shape as the opendal remote: one listing, then upload the diff.
            let present: HashSet<blake3::Hash> =
                ssh.list()?.into_iter().map(|(h, _)| h).collect();
            let pending = pending_uploads(&chunks, &present);
            for hash in &pending {
                let data = store.get_encoded(hash).with_context(|| {
                    format!(
                        "ssh remote needs {} but it is not in the local store — run `git cdc pull` first (a dropped chunk from `git cdc drop --chunks` needs refetching)",
                        oid_str(hash)
                    )
                })?;
                ssh.put_encoded(hash, &data)?;
            }
            pending.len()
        }
    };
    eprintln!(
        "git-cdc: pushed {uploaded} of {total} chunks ({} already remote)",
        total - uploaded
    );
    Ok(())
}

pub fn cmd_pull() -> Result<()> {
    let store = local_store()?;
    let root = repo_root()?;
    let tracked = index_manifests()?;
    if tracked.is_empty() {
        eprintln!("git-cdc: no tracked files in index, nothing to pull");
        return Ok(());
    }

    // Fetch chunks the local store is missing.
    let mut missing: std::collections::HashMap<blake3::Hash, u64> = Default::default();
    for (_, m) in &tracked {
        for c in &m.chunks {
            if !store.has(&c.hash) {
                missing.insert(c.hash, c.length as u64);
            }
        }
    }
    if !missing.is_empty() {
        match remote()? {
            Remote::Http(client) => {
                let objects = missing
                    .iter()
                    .map(|(h, size)| ObjectSpec {
                        oid: oid_str(h),
                        size: *size,
                    })
                    .collect();
                let resp = client.batch(Operation::Download, objects)?;
                for obj in &resp.objects {
                    if let Some(err) = &obj.error {
                        bail!(
                            "server cannot provide {}: {} {}",
                            obj.oid,
                            err.code,
                            err.message
                        );
                    }
                    let href = &obj
                        .actions
                        .as_ref()
                        .and_then(|a| a.download.as_ref())
                        .with_context(|| format!("no download action for {}", obj.oid))?
                        .href;
                    let hash = git_cdc_core::manifest::parse_hash(&obj.oid)?;
                    let data = client.download(href)?;
                    store.put_encoded(&hash, &data)?; // decodes envelope, verifies hash
                }
            }
            Remote::Opendal {
                store: remote_store,
                rt,
            } => rt.block_on(async {
                for hash in missing.keys() {
                    let data = remote_store.get_encoded(hash).await?;
                    store.put_encoded(hash, &data)?; // decodes envelope, verifies hash
                }
                anyhow::Ok(())
            })?,
            Remote::Ssh(mut ssh) => {
                for hash in missing.keys() {
                    let data = ssh.get_encoded(hash)?;
                    store.put_encoded(hash, &data)?; // decodes envelope, verifies hash
                }
            }
        }
        eprintln!("git-cdc: fetched {} chunks", missing.len());
    }

    // Materialize worktree files still in passed-through-manifest state.
    let mut materialized = 0usize;
    for (path, m) in &tracked {
        let abs = root.join(path);
        let worktree = fs::read(&abs).unwrap_or_default();
        if !is_manifest(&worktree) {
            continue; // already real content (or locally modified — leave it)
        }
        let mut out = Vec::with_capacity(m.size as usize);
        let mut hasher = blake3::Hasher::new();
        for c in &m.chunks {
            let data = store.get(&c.hash)?;
            hasher.update(&data);
            out.extend_from_slice(&data);
        }
        if hasher.finalize() != m.oid {
            bail!("{path}: reassembled content does not match manifest oid");
        }
        fs::write(&abs, out)?;
        materialized += 1;
    }
    // ponytail: materialized files read as modified in `git status` until
    // something re-runs the clean filter — git's stat cache recorded the
    // manifest's size, not the content's, and neither `update-index
    // --refresh` nor `--really-refresh` re-checks content (both stat-only).
    // Only `git add` settles it, and that re-chunks every file just pulled,
    // so it is deliberately not done here. Pre-existing since fresh clones
    // hit the same path; fix it when the re-chunk cost is worth paying.
    eprintln!("git-cdc: materialized {materialized} files");
    Ok(())
}

pub fn cmd_gc(dry_run: bool, grace_secs: u64) -> Result<()> {
    let store = local_store()?;
    let grace = std::time::Duration::from_secs(grace_secs);
    let live: std::collections::HashSet<blake3::Hash> = all_manifests()?
        .iter()
        .flat_map(|m| m.chunks.iter().map(|c| c.hash))
        .collect();

    // Local sweep: same mark-and-sweep + grace rule the server applies.
    let now = SystemTime::now();
    let mut swept = 0usize;
    for hash in store.list()? {
        let modified = fs::metadata(store.path_for(&hash))
            .and_then(|md| md.modified())
            .ok();
        if !matches!(
            sweep_decision(live.contains(&hash), modified, grace, now),
            SweepAction::Delete
        ) {
            continue;
        }
        if dry_run {
            eprintln!("would remove local {}", oid_str(&hash));
        } else {
            store.remove(&hash)?;
        }
        swept += 1;
    }
    eprintln!(
        "git-cdc: local gc {} {swept} unreferenced chunks ({} live)",
        if dry_run { "would remove" } else { "removed" },
        live.len()
    );

    // Remote sweep, if a remote is configured.
    match remote() {
        Ok(Remote::Http(client)) => {
            // Server owns the remote grace period (its --grace-secs).
            let resp = client.gc(live.iter().map(oid_str).collect(), dry_run)?;
            eprintln!(
                "git-cdc: remote gc {} {} chunks ({} live, {} in grace period)",
                if dry_run { "would remove" } else { "removed" },
                resp.deleted.len(),
                resp.kept_live,
                resp.kept_grace
            );
        }
        Ok(Remote::Opendal {
            store: remote_store,
            rt,
        }) => {
            // Serverless: no server to own the sweep — the CLI's grace applies.
            let (deleted, kept_live, kept_grace) = rt.block_on(async {
                let (mut deleted, mut kept_live, mut kept_grace) = (0usize, 0usize, 0usize);
                for (hash, modified) in remote_store.list().await? {
                    match sweep_decision(live.contains(&hash), modified, grace, now) {
                        SweepAction::KeepLive => kept_live += 1,
                        SweepAction::KeepGrace => kept_grace += 1,
                        SweepAction::Delete => {
                            if !dry_run {
                                remote_store.remove(&hash).await?;
                            }
                            deleted += 1;
                        }
                    }
                }
                anyhow::Ok((deleted, kept_live, kept_grace))
            })?;
            eprintln!(
                "git-cdc: remote gc {} {deleted} chunks ({kept_live} live, {kept_grace} in grace period)",
                if dry_run { "would remove" } else { "removed" },
            );
        }
        Ok(Remote::Ssh(mut ssh)) => {
            // Like the opendal remote: no server owns the sweep — CLI grace + remote mtimes.
            let (mut deleted, mut kept_live, mut kept_grace) = (0usize, 0usize, 0usize);
            for (hash, modified) in ssh.list()? {
                match sweep_decision(live.contains(&hash), modified, grace, now) {
                    SweepAction::KeepLive => kept_live += 1,
                    SweepAction::KeepGrace => kept_grace += 1,
                    SweepAction::Delete => {
                        if !dry_run {
                            ssh.remove(&hash)?;
                        }
                        deleted += 1;
                    }
                }
            }
            eprintln!(
                "git-cdc: ssh gc {} {deleted} chunks ({kept_live} live, {kept_grace} in grace period)",
                if dry_run { "would remove" } else { "removed" },
            );
        }
        Err(_) => eprintln!("git-cdc: no remote configured, skipped remote gc"),
    }
    Ok(())
}

/// Reclaim local disk. A tracked file costs it twice — the hydrated worktree
/// copy plus its chunks — so `drop` rewrites the file back to its committed
/// manifest (the state a fresh clone is in, which `cmd_pull` reverses) and,
/// with `--chunks`, removes chunks the remote provably holds.
pub fn cmd_drop(paths: &[String], all: bool, chunks: bool, dry_run: bool) -> Result<()> {
    let tracked = index_manifests()?;
    let selected: Vec<&(String, git_cdc_core::manifest::Manifest)> = if all {
        tracked.iter().collect()
    } else if paths.is_empty() {
        bail!("usage: git cdc drop [--chunks] [--dry-run] --all | <path>...");
    } else {
        resolve_paths(paths, &tracked)?
            .into_iter()
            .map(|i| &tracked[i])
            .collect()
    };
    if selected.is_empty() {
        eprintln!("git-cdc: no tracked files selected, nothing to drop");
        return Ok(());
    }

    let store = local_store()?;
    let root = repo_root()?;

    // Everything that can fail is resolved before the first byte is written:
    // `--chunks` without a usable remote must abort with no side effects,
    // worktree rewrites included.
    let candidates: HashMap<blake3::Hash, u64> = if chunks {
        selected
            .iter()
            .flat_map(|(_, m)| m.chunks.iter())
            .filter(|c| store.has(&c.hash))
            .map(|c| (c.hash, c.length as u64))
            .collect()
    } else {
        HashMap::new()
    };
    let removable = if chunks {
        match remote_present(&candidates) {
            Ok(present) => present,
            // A dry run deletes nothing, so it stays useful without a remote:
            // report the worktree half rather than refusing to run at all.
            Err(e) if dry_run => {
                eprintln!("git-cdc: no usable remote ({e:#}); chunks would be kept");
                HashSet::new()
            }
            Err(e) => return Err(e.context(
                "--chunks needs a remote: a chunk is only safe to delete locally once the remote has it",
            )),
        }
    } else {
        HashSet::new()
    };

    let (mut dropped, mut skipped, mut modified, mut freed) = (0usize, 0usize, 0usize, 0u64);
    let mut reclaimable: HashSet<blake3::Hash> = HashSet::new();
    for (path, m) in &selected {
        let abs = root.join(path);
        let Ok(content) = fs::read(&abs) else {
            skipped += 1; // absent from the worktree — nothing to reclaim
            continue;
        };
        if is_manifest(&content) {
            // Already dropped (or never materialized). Its chunks are still
            // reclaimable — `drop` then `drop --chunks` on the same file must
            // reach them on the second run.
            reclaimable.extend(m.chunks.iter().map(|c| c.hash));
            skipped += 1;
            continue;
        }
        // The one irreversible mistake this command could make: uncommitted
        // edits were never chunked (the clean filter runs at `git add`), so
        // they exist in no store and overwriting them destroys them.
        if blake3::hash(&content) != m.oid {
            eprintln!(
                "git-cdc: {path}: worktree content does not match the committed manifest (uncommitted changes?) — skipped"
            );
            skipped += 1;
            modified += 1;
            continue;
        }
        if !dry_run {
            // The index blob verbatim, never a re-encoded manifest: the blob
            // oid must not drift from what is committed.
            let blob = std::process::Command::new("git")
                .args(["show", &format!(":{path}")])
                .current_dir(&root)
                .output()
                .context("running git show")?;
            if !blob.status.success() {
                bail!("{path}: git show :{path} failed");
            }
            fs::write(&abs, &blob.stdout)?;
            // Mandatory: git distrusts the file-size drop and reports the
            // file modified until the clean filter re-runs. `git update-index
            // --refresh` does not clear it; re-adding does, and stages
            // nothing because manifest text cleans to the same blob.
            git_out(&["-C", &root.to_string_lossy(), "add", "--", path]).with_context(|| {
                format!("{path}: rewritten to its manifest but `git add` failed — run `git add -- {path}` to settle the index")
            })?;
        }
        reclaimable.extend(m.chunks.iter().map(|c| c.hash));
        dropped += 1;
        freed += m.size;
    }
    if modified == selected.len() {
        bail!("every selected file has uncommitted changes; nothing dropped");
    }

    // Only chunks belonging to files that actually reached manifest state —
    // a file skipped for uncommitted edits keeps its chunks, so a later
    // `git checkout` of it still restores content from the local store.
    let (mut removed, mut chunk_bytes) = (0usize, 0u64);
    for hash in removable.intersection(&reclaimable) {
        if !dry_run {
            store.remove(hash)?;
        }
        chunk_bytes += candidates.get(hash).copied().unwrap_or(0);
        removed += 1;
    }
    for hash in candidates.keys().filter(|h| !removable.contains(*h)) {
        eprintln!(
            "git-cdc: {} is not on the remote — kept locally",
            oid_str(hash)
        );
    }

    let verb = if dry_run { "would drop" } else { "dropped" };
    eprintln!(
        "git-cdc: {verb} {dropped} files ({skipped} skipped), {removed} of {} local chunks; {freed} bytes of worktree + {chunk_bytes} bytes of chunks freed",
        candidates.len(),
    );
    if dropped > 0 && !dry_run {
        eprintln!("git-cdc: run `git cdc pull` to restore file content");
    }
    Ok(())
}
