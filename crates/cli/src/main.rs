use std::fs;
use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use git_cdc_core::manifest::Manifest;

mod filter;
mod git;
mod remote;
mod sync;

use git::{git_dir, git_out, repo_root};

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
    /// Long-running clean+smudge filter (gitattributes filter-process protocol)
    #[command(hide = true)]
    FilterProcess,
    /// Serve a chunk store over stdin/stdout (the far end of ssh transport)
    #[command(hide = true)]
    Stdio {
        #[arg(long)]
        root: PathBuf,
    },
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
    /// Diff two manifest files (added/removed chunks and bytes); with no
    /// arguments, diff every tracked file between HEAD and the index
    Diff {
        from: Option<PathBuf>,
        to: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Cmd::Install { global } => cmd_install(global),
        Cmd::Track { patterns } => cmd_track(&patterns),
        Cmd::Clean => filter::cmd_clean(),
        Cmd::Smudge => filter::cmd_smudge(),
        Cmd::FilterProcess => filter::cmd_filter_process(),
        Cmd::Stdio { root } => remote::cmd_stdio(&root),
        Cmd::Pull => sync::cmd_pull(),
        Cmd::Push => sync::cmd_push(),
        Cmd::Gc {
            dry_run,
            grace_secs,
        } => sync::cmd_gc(dry_run, grace_secs),
        Cmd::Diff { from, to } => match (from, to) {
            (Some(from), Some(to)) => cmd_diff(&from, &to),
            (None, None) => cmd_diff_repo(),
            _ => bail!("git cdc diff takes two manifest files, or none for HEAD vs index"),
        },
    }
}

const HOOK_MARKER: &str = "git cdc push";

fn cmd_install(global: bool) -> Result<()> {
    let scope = if global { "--global" } else { "--local" };
    for (key, value) in [
        // git ≥ 2.11 uses the long-running process; clean/smudge are the
        // documented fallback for older git and stay registered.
        ("filter.cdc.process", "git-cdc filter-process"),
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
            fs::write(
                &hook,
                "#!/bin/sh\n# installed by git-cdc\ngit cdc push || exit 1\n",
            )?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&hook, fs::Permissions::from_mode(0o755))?;
            }
        }
    }
    eprintln!(
        "git-cdc filter installed ({})",
        if global { "global" } else { "local" }
    );
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

/// (added_n, added_bytes, removed_n, removed_bytes, total chunks in `b`).
/// `None` means "no manifest" (e.g. file absent at HEAD) — an empty side.
fn diff_stats(a: Option<&Manifest>, b: Option<&Manifest>) -> (usize, u64, usize, u64, usize) {
    let set = |m: Option<&Manifest>| -> std::collections::HashMap<_, _> {
        m.map(|m| m.chunks.iter().map(|c| (c.hash, c.length)).collect())
            .unwrap_or_default()
    };
    let (a_set, b_set) = (set(a), set(b));
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
    (added_n, added, removed_n, removed, b_set.len())
}

fn cmd_diff(from: &PathBuf, to: &PathBuf) -> Result<()> {
    let a = Manifest::parse(&fs::read(from)?)?;
    let b = Manifest::parse(&fs::read(to)?)?;
    let (added_n, added, removed_n, removed, total) = diff_stats(Some(&a), Some(&b));

    println!(
        "{} of {total} chunks changed, +{added} B / -{removed} B",
        added_n.max(removed_n),
    );
    println!("added: {added_n} chunks (+{added} bytes)");
    println!("removed: {removed_n} chunks (-{removed} bytes)");
    Ok(())
}

/// No-argument form: every tracked file, HEAD manifest vs index manifest.
fn cmd_diff_repo() -> Result<()> {
    use git_cdc_core::manifest::is_manifest;
    let mut changed = 0usize;
    for (path, index) in git::index_manifests()? {
        let head = std::process::Command::new("git")
            .args(["show", &format!("HEAD:{path}")])
            .output()?;
        let head = (head.status.success() && is_manifest(&head.stdout))
            .then(|| Manifest::parse(&head.stdout))
            .transpose()?;
        let (added_n, added, removed_n, removed, total) = diff_stats(head.as_ref(), Some(&index));
        if added_n == 0 && removed_n == 0 {
            continue;
        }
        println!(
            "{path}: {} of {total} chunks changed, +{added} B / -{removed} B",
            added_n.max(removed_n),
        );
        changed += 1;
    }
    if changed == 0 {
        println!("no tracked files changed between HEAD and index");
    }
    Ok(())
}
