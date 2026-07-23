//! Full network e2e (PLAN 5.4): scratch repo + in-process server; dedup on
//! second push, fresh clone via smudge passthrough, pull materialization,
//! pre-push hook guard, and GC.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use git_cdc_core::chunker::test_util::test_data;
use git_cdc_core::store::DiskStore;
use git_cdc_server::{AppState, Backend, app};

mod utils;

use utils::{BIN, cdc, git, git_cmd};

const TOKEN: &str = "e2e-token";

fn spawn_server(root: PathBuf, grace: Duration) -> String {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            tx.send(format!("http://{}", listener.local_addr().unwrap()))
                .unwrap();
            let state = AppState {
                backend: Backend::Disk(DiskStore::new(root)),
                token: TOKEN.into(),
                grace,
                upload_times: Default::default(),
            };
            axum::serve(listener, app(state)).await.unwrap();
        });
    });
    rx.recv().unwrap()
}

fn setup_repo(repo: &Path, server_url: &str) {
    utils::base_setup_repo(repo);
    git(repo, &["config", "cdc.url", server_url]);
    git(repo, &["config", "cdc.token", TOKEN]);
}

fn server_chunk_count(server_root: &Path) -> usize {
    DiskStore::new(server_root.join("objects"))
        .list()
        .unwrap()
        .len()
}

#[test]
fn push_with_missing_local_chunk_says_how_to_recover() {
    let tmp = tempfile::tempdir().unwrap();
    let url = spawn_server(tmp.path().join("server/objects"), Duration::ZERO);

    let repo = tmp.path().join("repo");
    fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    setup_repo(&repo, &url);
    cdc(&repo, &["track", "*.bin"]);
    fs::write(repo.join("asset.bin"), test_data(2 * 1024 * 1024, 42)).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "v1"]);

    // History references chunks, but the local store is gone (e.g. a clone
    // that never pulled) and the server never got them: push cannot invent
    // the bytes — it must fail and name the fix.
    fs::remove_dir_all(repo.join(".git/cdc")).unwrap();
    let out = Command::new(BIN)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .args(["push"])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "push cannot succeed without the chunk bytes"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("git cdc pull"),
        "error must tell the user how to recover: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn sync_without_remote_config_names_both_options() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["config", "user.name", "Test"]);
    cdc(&repo, &["track", "*.bin"]);
    git(
        &repo,
        &["config", "filter.cdc.clean", &format!("{BIN} clean")],
    );
    git(
        &repo,
        &["config", "filter.cdc.smudge", &format!("{BIN} smudge")],
    );
    git(
        &repo,
        &[
            "config",
            "filter.cdc.process",
            &format!("{BIN} filter-process"),
        ],
    );
    fs::write(repo.join("asset.bin"), test_data(1024 * 1024, 7)).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "v1"]);

    let out = Command::new(BIN)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .args(["push"])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("cdc.url") && err.contains("cdc.store.scheme"),
        "error must name both remote options: {err}"
    );
}

#[test]
fn full_push_clone_pull_gc_cycle() {
    let tmp = tempfile::tempdir().unwrap();
    let server_root = tmp.path().join("server");
    let url = spawn_server(server_root.join("objects"), Duration::ZERO);

    // origin repo with v1 and v2 of a 20 MiB file
    let repo = tmp.path().join("origin");
    fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    setup_repo(&repo, &url);
    cdc(&repo, &["track", "*.bin"]);

    let mut data = test_data(20 * 1024 * 1024, 99);
    fs::write(repo.join("asset.bin"), &data).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "v1"]);

    cdc(&repo, &["push"]);
    let count_v1 = server_chunk_count(&server_root);
    assert!(
        count_v1 >= 3,
        "20 MiB should be several chunks, got {count_v1}"
    );

    // 1 KiB edit in the middle → v2.
    for (i, b) in data[10_000_000..10_001_024].iter_mut().enumerate() {
        *b ^= (i as u8) | 1;
    }
    let data_v2 = data;
    fs::write(repo.join("asset.bin"), &data_v2).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "v2"]);

    let log = cdc(&repo, &["push"]);
    let count_v2 = server_chunk_count(&server_root);
    let new_chunks = count_v2 - count_v1;
    assert!(
        new_chunks <= 3,
        "1 KiB edit should upload only a few chunks, uploaded {new_chunks} ({log})"
    );

    // pre-push hook guard
    let remote = tmp.path().join("remote.git");
    git(
        tmp.path(),
        &[
            "init",
            "-q",
            "-b",
            "main",
            "--bare",
            &remote.to_string_lossy(),
        ],
    );
    git(
        &repo,
        &["remote", "add", "origin", &remote.to_string_lossy()],
    );

    // Unreachable server → hook's `git cdc push` fails → git push is blocked.
    git(&repo, &["config", "cdc.url", "http://127.0.0.1:1"]);
    let blocked = git_cmd(&repo, &["push", "origin", "main"]);
    assert!(
        !blocked.status.success(),
        "push should be blocked by the pre-push hook"
    );

    // Reachable again → hook succeeds → push goes through.
    git(&repo, &["config", "cdc.url", &url]);
    git(&repo, &["push", "origin", "main"]);

    // fresh clone: succeeds, passthrough, pull materializes
    let clone = tmp.path().join("clone");
    git(
        tmp.path(),
        &[
            "clone",
            "-q",
            &remote.to_string_lossy(),
            &clone.to_string_lossy(),
        ],
    );
    setup_repo(&clone, &url);

    let worktree = fs::read(clone.join("asset.bin")).unwrap();
    assert!(
        worktree.starts_with(b"version git-cdc/spec/v1\n"),
        "fresh clone should hold manifest text before pull"
    );

    cdc(&clone, &["pull"]);
    assert_eq!(
        fs::read(clone.join("asset.bin")).unwrap(),
        data_v2,
        "v2 materialized"
    );

    // v1 restores through smudge now that chunks are local (pull fetched only
    // v2's chunks; v1 shares all but the edited ones — fetch the rest).
    git(&clone, &["checkout", "-q", "HEAD~1", "--", "asset.bin"]);
    let v1_state = fs::read(clone.join("asset.bin")).unwrap();
    if v1_state.starts_with(b"version ") {
        cdc(&clone, &["pull"]); // v1's unique chunks weren't local yet
    }
    let restored_v1 = fs::read(clone.join("asset.bin")).unwrap();
    let mut expected_v1 = test_data(20 * 1024 * 1024, 99);
    assert_eq!(restored_v1, expected_v1, "v1 restores byte-identically");
    expected_v1.clear();

    // gc: drop v2, its unique chunks become garbage
    git(&repo, &["reset", "-q", "--hard", "HEAD~1"]);
    git(&repo, &["push", "-q", "--force", "origin", "main"]);
    let before = server_chunk_count(&server_root);
    let dry = cdc(&repo, &["gc", "--dry-run", "--grace-secs", "0"]);
    assert_eq!(
        server_chunk_count(&server_root),
        before,
        "dry run deletes nothing ({dry})"
    );
    cdc(&repo, &["gc", "--grace-secs", "0"]);
    let after = server_chunk_count(&server_root);
    assert!(
        after < before,
        "gc should remove v2-only chunks ({before} -> {after})"
    );
    assert_eq!(after, count_v1, "server back to exactly the v1 chunk set");

    // Everything still fetchable after gc.
    let clone2 = tmp.path().join("clone2");
    git(
        tmp.path(),
        &[
            "clone",
            "-q",
            &remote.to_string_lossy(),
            &clone2.to_string_lossy(),
        ],
    );
    setup_repo(&clone2, &url);
    cdc(&clone2, &["pull"]);
    assert_eq!(
        fs::read(clone2.join("asset.bin")).unwrap(),
        test_data(20 * 1024 * 1024, 99),
        "v1 fetchable from a second clone after gc"
    );
}
