use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Command as Git;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use git_cdc_core::chunker::chunk_stream;
use git_cdc_core::manifest::{is_manifest, Manifest};
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
        Cmd::Pull => bail!("pull is not implemented yet (plan phase 5)"),
        Cmd::Push => bail!("push is not implemented yet (plan phase 5)"),
        Cmd::Gc { .. } => bail!("gc is not implemented yet (plan phase 5)"),
        Cmd::Diff { from, to } => cmd_diff(&from, &to),
    }
}

// ---- git plumbing -----------------------------------------------------

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

// ---- install / track ---------------------------------------------------

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

// ---- clean / smudge ----------------------------------------------------

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

    let store = local_store()?;
    let input = head.as_slice().chain(reader);
    let (chunks, oid, size) = chunk_stream(input, |c, bytes| store.put(&c.hash, bytes))?;
    stdout.write_all(Manifest::new(oid, size, chunks).encode().as_bytes())?;
    Ok(())
}

fn cmd_smudge() -> Result<()> {
    let mut input = Vec::new();
    std::io::stdin().lock().read_to_end(&mut input)?;
    let mut stdout = std::io::stdout().lock();

    if !is_manifest(&input) {
        // Not ours (e.g. file committed before tracking) — pass through.
        stdout.write_all(&input)?;
        return Ok(());
    }
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

// ---- diff ----------------------------------------------------------------

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
