//! Shared e2e harness: run git / git-cdc against a scratch repo. Each test
//! file layers its remote-specific config (http token / s3 / ssh) on top of
//! `base_setup_repo`.

// Compiled once per test binary; not every binary uses every helper.
#![allow(dead_code)]

use std::path::Path;
use std::process::Command;

pub const BIN: &str = env!("CARGO_BIN_EXE_git-cdc");

/// Run git without asserting success (e.g. to test a blocked push).
/// Hooks invoke `git cdc push` via $PATH — put the freshly built binary first.
pub fn git_cmd(repo: &Path, args: &[&str]) -> std::process::Output {
    let bin_dir = Path::new(BIN).parent().unwrap();
    let path = format!("{}:{}", bin_dir.display(), std::env::var("PATH").unwrap());
    Command::new("git")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("PATH", path)
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap()
}

pub fn git(repo: &Path, args: &[&str]) -> String {
    let out = git_cmd(repo, args);
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

pub fn cdc(repo: &Path, args: &[&str]) -> String {
    let out = Command::new(BIN)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git-cdc {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Identity, hooks, and filter wiring common to every suite. `install`
/// writes `git-cdc clean` expecting the binary on $PATH; tests must run the
/// freshly built binary, so point the filter config at it directly.
pub fn base_setup_repo(repo: &Path) {
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Test"]);
    cdc(repo, &["install"]);
    git(
        repo,
        &["config", "filter.cdc.clean", &format!("{BIN} clean")],
    );
    git(
        repo,
        &["config", "filter.cdc.smudge", &format!("{BIN} smudge")],
    );
    git(
        repo,
        &[
            "config",
            "filter.cdc.process",
            &format!("{BIN} filter-process"),
        ],
    );
}
