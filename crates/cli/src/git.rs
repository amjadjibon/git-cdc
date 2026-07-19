//! Git plumbing: running git, resolving repo paths/config, and walking
//! history/index for manifests.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Command as Git;

use anyhow::{Context, Result, bail};
use git_cdc_core::chunker::ChunkParams;
use git_cdc_core::manifest::{Manifest, is_manifest};
use git_cdc_core::store::DiskStore;

pub fn git_out(args: &[&str]) -> Result<String> {
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

pub fn repo_root() -> Result<PathBuf> {
    Ok(PathBuf::from(git_out(&["rev-parse", "--show-toplevel"])?))
}

pub fn git_dir() -> Result<PathBuf> {
    Ok(PathBuf::from(git_out(&[
        "rev-parse",
        "--absolute-git-dir",
    ])?))
}

pub fn local_store() -> Result<DiskStore> {
    Ok(DiskStore::new(git_dir()?.join("cdc").join("objects")))
}

/// `cdc.chunk.{min,avg,max}` from git config, defaults where unset.
/// `--type=int` expands git's k/m/g suffixes ("512k" → 524288).
pub fn chunk_params() -> Result<ChunkParams> {
    let get = |key: &str, default: u32| -> Result<u32> {
        if git_out(&["config", "--get", key]).is_err() {
            return Ok(default); // unset — a malformed value must NOT land here
        }
        let v = git_out(&["config", "--type=int", "--get", key])?;
        v.parse::<u64>()
            .ok()
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

pub fn oid_str(hash: &blake3::Hash) -> String {
    format!("blake3:{}", hash.to_hex())
}

/// Every manifest blob across history (PLAN 5.2): `git rev-list --all
/// --objects` for reachability, `cat-file --batch-check` to keep only
/// plausibly-sized blobs, `cat-file --batch` to read and sniff them by
/// their fixed first line — path/attribute matching would miss renamed
/// or historical files.
pub fn all_manifests() -> Result<Vec<Manifest>> {
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
        let size: u64 = size
            .parse()
            .with_context(|| format!("bad cat-file header: {header:?}"))?;
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
pub fn index_manifests() -> Result<Vec<(String, Manifest)>> {
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
        if is_manifest(&blob.stdout)
            && let Ok(m) = Manifest::parse(&blob.stdout)
        {
            out.push((path.to_string(), m));
        }
    }
    Ok(out)
}
