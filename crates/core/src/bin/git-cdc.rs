use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Command as Git;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use git_cdc_core::chunker::{chunk_stream, ChunkParams};
use git_cdc_core::manifest::{is_manifest, Manifest};
use git_cdc_core::protocol::{ObjectSpec, Operation};
use git_cdc_core::store::{ChunkStore, DiskStore};

#[derive(Parser)]
#[command(name = "git-cdc", bin_name = "git cdc", version)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Register the cdc filter driver (repo-local, or --global) and, for
    /// repo-local installs, a pre-push hook running `git cdc push`
    Install {
        #[arg(long)]
        global: bool,
    },
    /// Add file patterns to .gitattributes
    Track { patterns: Vec<String> },
    /// Clean filter: file content on stdin -> manifest on stdout
    #[command(hide = true)]
    Clean,
    /// Smudge filter: manifest on stdin -> file content on stdout
    #[command(hide = true)]
    Smudge,
    /// Fetch chunks for the current checkout and materialize tracked files
    Pull,
    /// Upload chunks referenced by any local manifest ahead of `git push`
    Push,
    /// Sweep unreferenced chunks from local and remote stores
    Gc {
        #[arg(long)]
        dry_run: bool,
        /// Unreferenced chunks younger than this survive (protects
        /// just-cleaned, not-yet-committed chunks and in-flight uploads)
        #[arg(long, default_value_t = 24 * 3600)]
        grace_secs: u64,
    },
    /// Diff two manifest files (added/removed chunks and bytes)
    Diff { from: PathBuf, to: PathBuf },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Cmd::Install { global } => cmd_install(global),
        Cmd::Track { patterns } => cmd_track(&patterns),
        Cmd::Clean => cmd_clean(),
        Cmd::Smudge => cmd_smudge(),
        Cmd::Pull => cmd_pull(),
        Cmd::Push => cmd_push(),
        Cmd::Gc { dry_run, grace_secs } => cmd_gc(dry_run, grace_secs),
        Cmd::Diff { from, to } => cmd_diff(&from, &to),
    }
}

fn git_out(args: &[&str]) -> Result<String> {
    let out = Git::new("git").args(args).output().context("running git")?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8(out.stdout)?.trim().to_string())
}

fn repo_root() -> Result<PathBuf> {
    Ok(PathBuf::from(git_out(&["rev-parse", "--show-toplevel"])?))
}

fn git_dir() -> Result<PathBuf> {
    Ok(PathBuf::from(git_out(&["rev-parse", "--absolute-git-dir"])?))
}

fn local_store() -> Result<DiskStore> {
    Ok(DiskStore::new(git_dir()?.join("cdc").join("objects")))
}

/// `cdc.chunk.{min,avg,max}` from git config, defaults where unset.
/// `--type=int` expands git's k/m/g suffixes ("512k" → 524288).
fn chunk_params() -> Result<ChunkParams> {
    let get = |key: &str, default: u32| -> Result<u32> {
        if git_out(&["config", "--get", key]).is_err() {
            return Ok(default); // unset — a malformed value must NOT land here
        }
        let v = git_out(&["config", "--type=int", "--get", key])?;
        v.parse::<u64>().ok()
            .and_then(|v| u32::try_from(v).ok())
            .with_context(|| format!("{key} = {v:?} is not a valid size"))
    };
    use git_cdc_core::chunker::{AVG_SIZE, MAX_SIZE, MIN_SIZE};
    ChunkParams {
        min: get("cdc.chunk.min", MIN_SIZE)?,
        avg: get("cdc.chunk.avg", AVG_SIZE)?,
        max: get("cdc.chunk.max", MAX_SIZE)?,
    }
    .validate()
}

const HOOK_MARKER: &str = "git cdc push";

fn cmd_install(global: bool) -> Result<()> {
    let scope = if global { "--global" } else { "--local" };
    for (key, value) in [
        ("filter.cdc.clean", "git-cdc clean"),
        ("filter.cdc.smudge", "git-cdc smudge"),
        // filter.cdc.required deliberately NOT set: smudge passes manifests
        // through when chunks are missing so fresh clones succeed (PLAN 4.3).
    ] {
        git_out(&["config", scope, key, value])?;
    }

    if !global {
        let hook = git_dir()?.join("hooks").join("pre-push");
        if hook.exists() {
            let existing = fs::read_to_string(&hook).unwrap_or_default();
            if !existing.contains(HOOK_MARKER) {
                eprintln!(
                    "warning: {} already exists; add `git cdc push` to it manually",
                    hook.display()
                );
            }
        } else {
            fs::create_dir_all(hook.parent().unwrap())?;
            fs::write(&hook, "#!/bin/sh\n# installed by git-cdc\ngit cdc push || exit 1\n")?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&hook, fs::Permissions::from_mode(0o755))?;
            }
        }
    }
    eprintln!("git-cdc filter installed ({})", if global { "global" } else { "local" });
    Ok(())
}

fn cmd_track(patterns: &[String]) -> Result<()> {
    if patterns.is_empty() {
        bail!("usage: git cdc track <pattern>...");
    }
    let path = repo_root()?.join(".gitattributes");
    let mut content = fs::read_to_string(&path).unwrap_or_default();
    for pattern in patterns {
        let line = format!("{pattern} filter=cdc -text");
        if content.lines().any(|l| l.trim() == line) {
            continue;
        }
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&line);
        content.push('\n');
        eprintln!("tracking {pattern}");
    }
    fs::write(&path, content)?;
    Ok(())
}

fn cmd_clean() -> Result<()> {
    let stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();

    // Peek far enough to recognize a manifest: re-cleaning passed-through
    // manifest text (fresh clone worktree state) must not chunk the manifest
    // itself — pass it through unchanged, same as git-lfs does for pointers.
    let mut reader = stdin;
    let mut head = Vec::with_capacity(64);
    (&mut reader).take(64).read_to_end(&mut head)?;
    if is_manifest(&head) {
        stdout.write_all(&head)?;
        std::io::copy(&mut reader, &mut stdout)?;
        return Ok(());
    }

    let params = chunk_params()?;
    let store = local_store()?;
    let input = head.as_slice().chain(reader);
    let (chunks, oid, size) = chunk_stream(input, params, |c, bytes| store.put(&c.hash, bytes))?;
    stdout.write_all(Manifest::new(oid, size, chunks, params).encode().as_bytes())?;
    Ok(())
}

fn cmd_smudge() -> Result<()> {
    let mut reader = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();

    // Same 64-byte peek as clean: the passthrough case (file committed
    // before tracking) can be arbitrarily large and must stream, not buffer.
    let mut input = Vec::with_capacity(64);
    (&mut reader).take(64).read_to_end(&mut input)?;
    if !is_manifest(&input) {
        stdout.write_all(&input)?;
        std::io::copy(&mut reader, &mut stdout)?;
        return Ok(());
    }
    reader.read_to_end(&mut input)?; // manifests are small
    let m = Manifest::parse(&input)?;
    let store = local_store()?;

    if m.chunks.iter().any(|c| !store.has(&c.hash)) {
        // Fresh clone / chunks not fetched yet: write the manifest through
        // so checkout succeeds, and tell the user how to materialize.
        stdout.write_all(&input)?;
        eprintln!("git-cdc: chunks not in local store; run `git cdc pull` to fetch file content");
        return Ok(());
    }

    let mut hasher = blake3::Hasher::new();
    for c in &m.chunks {
        let data = store.get(&c.hash)?; // hard-errors on corrupt chunk
        hasher.update(&data);
        stdout.write_all(&data)?;
    }
    if hasher.finalize() != m.oid {
        bail!("reassembled file does not match manifest oid — refusing to emit corrupt data");
    }
    Ok(())
}

/// Where chunks live remotely: a git-cdc-server (batch API) or, serverless,
/// an S3 bucket the CLI talks to directly with IAM credentials.
enum Remote {
    Http(git_cdc_core::client::Client),
    S3 {
        store: git_cdc_core::store::s3::S3Store,
        rt: tokio::runtime::Runtime,
    },
}

fn remote() -> Result<Remote> {
    if let Ok(bucket) = git_out(&["config", "--get", "cdc.s3.bucket"]) {
        let config = git_cdc_core::store::s3::S3Config {
            bucket,
            prefix: git_out(&["config", "--get", "cdc.s3.prefix"]).unwrap_or_default(),
            endpoint: git_out(&["config", "--get", "cdc.s3.endpoint"]).ok(),
            force_path_style: git_out(&["config", "--get", "cdc.s3.force-path-style"])
                .map(|v| v == "true")
                .unwrap_or(false),
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let store = rt.block_on(git_cdc_core::store::s3::S3Store::connect(&config));
        return Ok(Remote::S3 { store, rt });
    }
    let url = git_out(&["config", "--get", "cdc.url"]).context(
        "no remote configured; set cdc.url + cdc.token (server) \
         or cdc.s3.bucket (serverless S3)",
    )?;
    let token = git_out(&["config", "--get", "cdc.token"])
        .context("cdc.token is not configured; set it with `git config cdc.token <token>`")?;
    Ok(Remote::Http(git_cdc_core::client::Client::new(&url, &token)))
}

fn oid_str(hash: &blake3::Hash) -> String {
    format!("blake3:{}", hash.to_hex())
}

/// Every manifest blob across history (PLAN 5.2): `git rev-list --all
/// --objects` for reachability, `cat-file --batch-check` to keep only
/// plausibly-sized blobs, `cat-file --batch` to read and sniff them by
/// their fixed first line — path/attribute matching would miss renamed
/// or historical files.
fn all_manifests() -> Result<Vec<Manifest>> {
    use std::io::{BufRead, BufReader, BufWriter};
    use std::process::Stdio;

    let list = git_out(&["rev-list", "--all", "--objects"])?;
    let mut shas: Vec<&str> = list
        .lines()
        .filter_map(|l| l.split(' ').next())
        .filter(|s| !s.is_empty())
        .collect();
    shas.sort_unstable();
    shas.dedup();

    let mut cat = Git::new("git")
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("spawning git cat-file")?;
    let mut to_git = BufWriter::new(cat.stdin.take().unwrap());
    let mut from_git = BufReader::new(cat.stdout.take().unwrap());

    // Manifests are ~90 bytes per chunk line; even a 100 GB file stays well
    // under this cap. Anything bigger can't be ours — skip without reading.
    const MAX_MANIFEST_SIZE: u64 = 16 * 1024 * 1024;

    let mut manifests = Vec::new();
    for sha in shas {
        // Sequential request/response per object — no pipe deadlock.
        writeln!(to_git, "{sha}")?;
        to_git.flush()?;
        let mut header = String::new();
        from_git.read_line(&mut header)?;
        let mut parts = header.split_whitespace();
        let (_sha, typ, size) = (
            parts.next().unwrap_or_default(),
            parts.next().unwrap_or_default(),
            parts.next().unwrap_or_default(),
        );
        if typ == "missing" {
            continue;
        }
        let size: u64 = size.parse().with_context(|| format!("bad cat-file header: {header:?}"))?;
        let mut body = vec![0u8; size as usize + 1]; // content + trailing LF
        from_git.read_exact(&mut body)?;
        body.pop();
        if typ != "blob" || size > MAX_MANIFEST_SIZE || !is_manifest(&body) {
            continue;
        }
        if let Ok(m) = Manifest::parse(&body) {
            manifests.push(m);
        }
    }
    drop(to_git);
    cat.wait()?;
    Ok(manifests)
}

/// Tracked (filter=cdc) paths in the index, with their staged manifests.
fn index_manifests() -> Result<Vec<(String, Manifest)>> {
    let files = git_out(&["ls-files"])?;
    let mut out = Vec::new();
    for path in files.lines() {
        let attr = git_out(&["check-attr", "filter", "--", path])?;
        if !attr.ends_with(": filter: cdc") {
            continue;
        }
        let blob = Git::new("git")
            .args(["show", &format!(":{path}")])
            .output()?;
        if !blob.status.success() {
            continue;
        }
        if is_manifest(&blob.stdout) {
            if let Ok(m) = Manifest::parse(&blob.stdout) {
                out.push((path.to_string(), m));
            }
        }
    }
    Ok(out)
}

fn cmd_push() -> Result<()> {
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
            let mut uploaded = 0usize;
            for obj in &resp.objects {
                let Some(action) = obj.actions.as_ref().and_then(|a| a.upload.as_ref()) else {
                    continue; // server already has it — the dedup win
                };
                let hash = git_cdc_core::manifest::parse_hash(&obj.oid)?;
                let data = store.get(&hash).with_context(|| {
                    format!("server wants {} but it is not in the local store — run `git cdc pull` first", obj.oid)
                })?;
                // ponytail: sequential uploads; add bounded concurrency when
                // a real repo shows transfer time dominated by round-trips.
                client.upload(&action.href, data)?;
                uploaded += 1;
            }
            uploaded
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
                let data = store.get(hash).with_context(|| {
                    format!("bucket needs {} but it is not in the local store — run `git cdc pull` first", oid_str(hash))
                })?;
                s3.put(hash, &data).await?;
                uploaded += 1;
            }
            anyhow::Ok(uploaded)
        })?,
    };
    eprintln!("git-cdc: pushed {uploaded} of {total} chunks ({} already remote)", total - uploaded);
    Ok(())
}

fn cmd_pull() -> Result<()> {
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
                    .map(|(h, size)| ObjectSpec { oid: oid_str(h), size: *size })
                    .collect();
                let resp = client.batch(Operation::Download, objects)?;
                for obj in &resp.objects {
                    if let Some(err) = &obj.error {
                        bail!("server cannot provide {}: {} {}", obj.oid, err.code, err.message);
                    }
                    let href = &obj
                        .actions
                        .as_ref()
                        .and_then(|a| a.download.as_ref())
                        .with_context(|| format!("no download action for {}", obj.oid))?
                        .href;
                    let hash = git_cdc_core::manifest::parse_hash(&obj.oid)?;
                    let data = client.download(href)?;
                    store.put(&hash, &data)?; // verifies hash before admitting
                }
            }
            Remote::S3 { store: s3, rt } => rt.block_on(async {
                for hash in missing.keys() {
                    let data = s3.get(hash).await?; // verifies hash on read
                    store.put(hash, &data)?;
                }
                anyhow::Ok(())
            })?,
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

fn cmd_gc(dry_run: bool, grace_secs: u64) -> Result<()> {
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
    eprintln!("git-cdc: local gc {} {swept} unreferenced chunks ({} live)",
        if dry_run { "would remove" } else { "removed" }, live.len());

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
        Err(_) => eprintln!("git-cdc: no remote configured, skipped remote gc"),
    }
    Ok(())
}

fn cmd_diff(from: &PathBuf, to: &PathBuf) -> Result<()> {
    let a = Manifest::parse(&fs::read(from)?)?;
    let b = Manifest::parse(&fs::read(to)?)?;
    let a_set: std::collections::HashMap<_, _> =
        a.chunks.iter().map(|c| (c.hash, c.length)).collect();
    let b_set: std::collections::HashMap<_, _> =
        b.chunks.iter().map(|c| (c.hash, c.length)).collect();

    let added: u64 = b_set
        .iter()
        .filter(|(h, _)| !a_set.contains_key(*h))
        .map(|(_, l)| *l as u64)
        .sum();
    let removed: u64 = a_set
        .iter()
        .filter(|(h, _)| !b_set.contains_key(*h))
        .map(|(_, l)| *l as u64)
        .sum();
    let added_n = b_set.keys().filter(|h| !a_set.contains_key(*h)).count();
    let removed_n = a_set.keys().filter(|h| !b_set.contains_key(*h)).count();

    println!(
        "{} of {} chunks changed, +{added} B / -{removed} B",
        added_n.max(removed_n),
        b.chunks.len()
    );
    println!("added: {added_n} chunks (+{added} bytes)");
    println!("removed: {removed_n} chunks (-{removed} bytes)");
    Ok(())
}
