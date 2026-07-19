//! Chunk movement between local and remote stores: push, pull, gc.

use std::fs;

use anyhow::{Context, Result, bail};
use git_cdc_core::manifest::is_manifest;
use git_cdc_core::protocol::{ObjectSpec, Operation};
use git_cdc_core::store::ChunkStore;

use crate::git::{all_manifests, index_manifests, local_store, oid_str, repo_root};
use crate::remote::{Remote, remote};

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
                        "server wants {} but it is not in the local store — run `git cdc pull` first",
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
        Remote::S3 { store: s3, rt } => rt.block_on(async {
            // One paginated listing beats a HeadObject per chunk.
            let present: std::collections::HashSet<blake3::Hash> =
                s3.list().await?.into_iter().map(|(h, _)| h).collect();
            let mut uploaded = 0usize;
            for hash in chunks.keys() {
                if present.contains(hash) {
                    continue;
                }
                let data = store.get_encoded(hash).with_context(|| {
                    format!("bucket needs {} but it is not in the local store — run `git cdc pull` first", oid_str(hash))
                })?;
                s3.put_encoded(hash, data).await?;
                uploaded += 1;
            }
            anyhow::Ok(uploaded)
        })?,
        Remote::Ssh(mut ssh) => {
            // Same shape as S3: one listing, then upload the diff.
            let present: std::collections::HashSet<blake3::Hash> =
                ssh.list()?.into_iter().map(|(h, _)| h).collect();
            let mut uploaded = 0usize;
            for hash in chunks.keys() {
                if present.contains(hash) {
                    continue;
                }
                let data = store.get_encoded(hash).with_context(|| {
                    format!(
                        "ssh remote needs {} but it is not in the local store — run `git cdc pull` first",
                        oid_str(hash)
                    )
                })?;
                ssh.put_encoded(hash, &data)?;
                uploaded += 1;
            }
            uploaded
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
            Remote::S3 { store: s3, rt } => rt.block_on(async {
                for hash in missing.keys() {
                    let data = s3.get_encoded(hash).await?;
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
    let now = std::time::SystemTime::now();
    let mut swept = 0usize;
    for hash in store.list()? {
        if live.contains(&hash) {
            continue;
        }
        let old_enough = fs::metadata(store.path_for(&hash))
            .and_then(|md| md.modified())
            .ok()
            .and_then(|mtime| now.duration_since(mtime).ok())
            .is_some_and(|age| age >= grace);
        if !old_enough {
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
        Ok(Remote::S3 { store: s3, rt }) => {
            // Serverless: no server to own the sweep — the CLI's grace applies.
            let (deleted, kept_live, kept_grace) = rt.block_on(async {
                let (mut deleted, mut kept_live, mut kept_grace) = (0usize, 0usize, 0usize);
                for (hash, modified) in s3.list().await? {
                    if live.contains(&hash) {
                        kept_live += 1;
                        continue;
                    }
                    let old_enough = modified
                        .and_then(|mtime| now.duration_since(mtime).ok())
                        .is_some_and(|age| age >= grace);
                    if !old_enough {
                        kept_grace += 1;
                        continue;
                    }
                    if !dry_run {
                        s3.remove(&hash).await?;
                    }
                    deleted += 1;
                }
                anyhow::Ok((deleted, kept_live, kept_grace))
            })?;
            eprintln!(
                "git-cdc: bucket gc {} {deleted} chunks ({kept_live} live, {kept_grace} in grace period)",
                if dry_run { "would remove" } else { "removed" },
            );
        }
        Ok(Remote::Ssh(mut ssh)) => {
            // Like S3: no server owns the sweep — CLI grace + remote mtimes.
            let (mut deleted, mut kept_live, mut kept_grace) = (0usize, 0usize, 0usize);
            for (hash, modified) in ssh.list()? {
                if live.contains(&hash) {
                    kept_live += 1;
                    continue;
                }
                let old_enough = modified
                    .and_then(|mtime| now.duration_since(mtime).ok())
                    .is_some_and(|age| age >= grace);
                if !old_enough {
                    kept_grace += 1;
                    continue;
                }
                if !dry_run {
                    ssh.remove(&hash)?;
                }
                deleted += 1;
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
