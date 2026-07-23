//! Serverless opendal e2e (PLAN 3.2): track → commit → push straight to a
//! remote store (no git-cdc server) → fresh clone → pull → gc. Exercises
//! the generic `cdc.opendal.*` remote against the `fs` scheme — the wire
//! shape is identical for every other OpenDAL service (s3, azblob, gcs,
//! ...), which is OpenDAL's contract to verify, not ours.

use std::fs;
use std::path::Path;
use std::process::Command;

use git_cdc_core::chunker::test_util::test_data;
use git_cdc_core::store::{OpendalConfig, OpendalStore};

mod common;

use common::{BIN, cdc, git};

fn setup_repo(repo: &Path, remote_root: &Path) {
    common::base_setup_repo(repo);
    git(repo, &["config", "cdc.opendal.scheme", "fs"]);
    git(
        repo,
        &[
            "config",
            "cdc.opendal.option",
            &format!("root={}", remote_root.display()),
        ],
    );
    git(repo, &["config", "cdc.opendal.prefix", "chunks/"]);
}

#[test]
fn serverless_push_clone_pull_gc() {
    let tmp = tempfile::tempdir().unwrap();
    let remote_root = tmp.path().join("remote-store");
    fs::create_dir_all(&remote_root).unwrap();
    let config = OpendalConfig {
        scheme: "fs".into(),
        options: vec![("root".into(), remote_root.to_str().unwrap().into())],
        prefix: "chunks/".into(),
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let count_remote = || {
        rt.block_on(async {
            OpendalStore::connect(&config)
                .unwrap()
                .list()
                .await
                .unwrap()
                .len()
        })
    };

    // origin: v1 + v2, push straight to the remote store.
    let repo = tmp.path().join("origin");
    fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    setup_repo(&repo, &remote_root);
    cdc(&repo, &["track", "*.bin"]);

    let mut data = test_data(12 * 1024 * 1024, 77);
    fs::write(repo.join("asset.bin"), &data).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "v1"]);
    cdc(&repo, &["push"]);
    let count_v1 = count_remote();
    assert!(count_v1 >= 2, "12 MiB should be several chunks");

    data[6_000_000] ^= 0xFF;
    fs::write(repo.join("asset.bin"), &data).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "v2"]);
    let log = cdc(&repo, &["push"]);
    assert!(
        count_remote() - count_v1 <= 2,
        "1-byte edit should upload few chunks ({log})"
    );

    // fresh clone via bare remote: passthrough, then pull from the remote store.
    let bare = tmp.path().join("remote.git");
    git(
        tmp.path(),
        &[
            "init",
            "-q",
            "-b",
            "main",
            "--bare",
            &bare.to_string_lossy(),
        ],
    );
    git(&repo, &["remote", "add", "origin", &bare.to_string_lossy()]);
    git(&repo, &["push", "-q", "origin", "main"]);

    let clone = tmp.path().join("clone");
    git(
        tmp.path(),
        &[
            "clone",
            "-q",
            &bare.to_string_lossy(),
            &clone.to_string_lossy(),
        ],
    );
    setup_repo(&clone, &remote_root);
    assert!(
        fs::read(clone.join("asset.bin"))
            .unwrap()
            .starts_with(b"version git-cdc/spec/v1\n")
    );
    cdc(&clone, &["pull"]);
    assert_eq!(
        fs::read(clone.join("asset.bin")).unwrap(),
        data,
        "v2 materialized from the remote store"
    );

    // gc: drop v2 everywhere (incl. the remote-tracking ref rev-list sees),
    // remote sweep removes its unique chunks.
    git(&repo, &["reset", "-q", "--hard", "HEAD~1"]);
    git(&repo, &["push", "-q", "--force", "origin", "main"]);
    let before = count_remote();
    cdc(&repo, &["gc", "--dry-run", "--grace-secs", "0"]);
    assert_eq!(count_remote(), before, "dry run deletes nothing");
    cdc(&repo, &["gc", "--grace-secs", "0"]);
    assert_eq!(
        count_remote(),
        count_v1,
        "remote store back to the v1 chunk set"
    );
}

/// `--grace-secs` must keep a freshly-unreferenced chunk around, not just
/// delete it outright — the serverless remote sweep owns this decision
/// itself (no server to defer to), so a huge grace window must survive
/// a real gc run, not only `--dry-run`.
#[test]
fn serverless_gc_keeps_chunks_within_grace_period() {
    let tmp = tempfile::tempdir().unwrap();
    let remote_root = tmp.path().join("remote-store");
    fs::create_dir_all(&remote_root).unwrap();
    let config = OpendalConfig {
        scheme: "fs".into(),
        options: vec![("root".into(), remote_root.to_str().unwrap().into())],
        prefix: "chunks/".into(),
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let count_remote = || {
        rt.block_on(async {
            OpendalStore::connect(&config)
                .unwrap()
                .list()
                .await
                .unwrap()
                .len()
        })
    };

    let repo = tmp.path().join("origin");
    fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    setup_repo(&repo, &remote_root);
    cdc(&repo, &["track", "*.bin"]);

    // v1 (to be dropped) then v2 (an edit) — `all_manifests()` walks
    // `rev-list --all`, so v1's chunks stay live only while v1 is still an
    // ancestor of a ref; the reset below is what actually orphans them.
    let mut data = test_data(12 * 1024 * 1024, 9);
    fs::write(repo.join("asset.bin"), &data).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "v1"]);
    cdc(&repo, &["push"]);
    let count_v1 = count_remote();
    assert!(count_v1 >= 2, "12 MiB should be several chunks");

    data[6_000_000] ^= 0xFF;
    fs::write(repo.join("asset.bin"), &data).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "v2"]);
    cdc(&repo, &["push"]);
    let count_v1_and_v2 = count_remote();
    assert!(count_v1_and_v2 > count_v1, "v2's edit adds new chunks");

    // Drop v2, orphaning its unique chunks — v1 is the only live manifest.
    git(&repo, &["reset", "-q", "--hard", "HEAD~1"]);

    // A grace period longer than "just now" must keep every chunk, even
    // on a real (non-dry-run) gc — this is the KeepGrace outcome.
    cdc(&repo, &["gc", "--grace-secs", "3600"]);
    assert_eq!(
        count_remote(),
        count_v1_and_v2,
        "chunks within the grace period must survive a real gc run"
    );

    // Zero grace reverts to immediate sweep (already covered by the main
    // serverless test) — confirms the grace window, not gc itself, is what
    // kept them above.
    cdc(&repo, &["gc", "--grace-secs", "0"]);
    assert_eq!(
        count_remote(),
        count_v1,
        "zero grace must sweep v2's now-unreferenced chunks"
    );
}

/// A malformed `cdc.opendal.option` (missing `=`) must fail loudly and name
/// the bad entry, not panic or silently drop it.
#[test]
fn serverless_malformed_opendal_option_errors_clearly() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    git(repo, &["init", "-q"]);
    common::base_setup_repo(repo);
    git(repo, &["config", "cdc.opendal.scheme", "fs"]);
    git(
        repo,
        &["config", "--add", "cdc.opendal.option", "no-equals-sign"],
    );
    cdc(repo, &["track", "*.bin"]);
    fs::write(repo.join("asset.bin"), test_data(1024, 1)).unwrap();
    git(repo, &["add", "."]);
    git(repo, &["commit", "-q", "-m", "v1"]);

    let out = Command::new(BIN)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .args(["push"])
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("no-equals-sign") && err.contains("KEY=VALUE"),
        "error must name the malformed entry: {err}"
    );
}
