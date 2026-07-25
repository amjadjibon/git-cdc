//! Serverless e2e (PLAN 3.2): track → commit → push straight to a
//! remote store (no git-cdc server) → fresh clone → pull → gc. Exercises
//! the generic `cdc.store.*` remote against the `fs` scheme — the wire
//! shape is identical for every other OpenDAL-backed service (s3, azblob,
//! gcs, ...), which is OpenDAL's contract to verify, not ours.

use std::fs;
use std::path::Path;
use std::process::Command;

use git_cdc_core::chunker::test_util::test_data;
use git_cdc_core::store::{DiskStore, OpendalConfig, OpendalStore};

mod utils;

use utils::{BIN, cdc, cdc_cmd, git};

fn setup_repo(repo: &Path, remote_root: &Path) {
    utils::base_setup_repo(repo);
    git(repo, &["config", "cdc.store.scheme", "fs"]);
    git(
        repo,
        &[
            "config",
            "cdc.store.option",
            &format!("root={}", remote_root.display()),
        ],
    );
    git(repo, &["config", "cdc.store.prefix", "chunks/"]);
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

/// A malformed `cdc.store.option` (missing `=`) must fail loudly and name
/// the bad entry, not panic or silently drop it.
#[test]
fn serverless_malformed_store_option_errors_clearly() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    git(repo, &["init", "-q"]);
    utils::base_setup_repo(repo);
    git(repo, &["config", "cdc.store.scheme", "fs"]);
    git(
        repo,
        &["config", "--add", "cdc.store.option", "no-equals-sign"],
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

// ---------------------------------------------------------------------------
// `git cdc drop` — reclaiming local disk. A tracked file costs it twice (the
// hydrated worktree copy plus its chunks), and both halves must come back
// through `git cdc pull`.
// ---------------------------------------------------------------------------

/// Chunks currently in a repo's local store.
fn local_chunks(repo: &Path) -> Vec<blake3::Hash> {
    let mut out = DiskStore::new(repo.join(".git").join("cdc").join("objects"))
        .list()
        .unwrap();
    out.sort_by_key(|h| h.to_hex().to_string());
    out
}

fn opendal_config(remote_root: &Path) -> OpendalConfig {
    OpendalConfig {
        scheme: "fs".into(),
        options: vec![("root".into(), remote_root.to_str().unwrap().into())],
        prefix: "chunks/".into(),
    }
}

/// A repo with one tracked, committed, pushed file at `name`. Returns its
/// content so tests can assert byte-identical restores.
fn pushed_repo(repo: &Path, remote_root: &Path, name: &str, seed: u64) -> Vec<u8> {
    fs::create_dir_all(remote_root).unwrap();
    fs::create_dir_all(repo).unwrap();
    git(repo, &["init", "-q", "-b", "main"]);
    setup_repo(repo, remote_root);
    cdc(repo, &["track", "*.bin"]);
    let data = test_data(3 * 1024 * 1024, seed);
    let path = repo.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, &data).unwrap();
    git(repo, &["add", "."]);
    git(repo, &["commit", "-q", "-m", "v1"]);
    cdc(repo, &["push"]);
    data
}

/// The whole feature in one test: drop reclaims both halves of local disk,
/// leaves the worktree in a state git considers clean, and `pull` reverses it.
#[test]
fn drop_reclaims_worktree_and_chunks() {
    let tmp = tempfile::tempdir().unwrap();
    let remote_root = tmp.path().join("remote-store");
    let repo = tmp.path().join("origin");
    let data = pushed_repo(&repo, &remote_root, "asset.bin", 41);
    assert!(!local_chunks(&repo).is_empty(), "chunks land in the store");

    cdc(&repo, &["drop", "--chunks", "asset.bin"]);

    assert!(
        fs::read(repo.join("asset.bin"))
            .unwrap()
            .starts_with(b"version git-cdc/spec/v1\n"),
        "worktree file collapses to its manifest"
    );
    assert!(
        local_chunks(&repo).is_empty(),
        "local chunks are gone — the remote has them"
    );
    // The load-bearing assertion: git must not see a dropped file as
    // modified, or `drop` is unusable in a real working tree.
    assert_eq!(
        git(&repo, &["status", "--porcelain"]).trim(),
        "",
        "a dropped file must leave the worktree clean"
    );

    cdc(&repo, &["pull"]);
    assert_eq!(
        fs::read(repo.join("asset.bin")).unwrap(),
        data,
        "pull restores byte-identical content"
    );
    // Not asserting a clean status here: `pull` leaves git's stat cache
    // recording the manifest's size, so the file reads as modified until
    // something re-runs the clean filter. Pre-existing (a fresh clone's pull
    // does the same) and untouched by `drop` — see the note in cmd_pull.
    assert_eq!(
        git(&repo, &["rev-parse", ":asset.bin"]),
        git(&repo, &["rev-parse", "HEAD:asset.bin"]),
        "pull stages nothing — the blob is unchanged"
    );
}

/// Uncommitted edits were never chunked (the clean filter runs at `git add`),
/// so they exist in no store and overwriting them is unrecoverable. This is
/// the one mistake `drop` must never make.
#[test]
fn drop_skips_files_with_uncommitted_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let remote_root = tmp.path().join("remote-store");
    let repo = tmp.path().join("origin");
    pushed_repo(&repo, &remote_root, "asset.bin", 42);

    let edited = b"work in progress that lives nowhere else".to_vec();
    fs::write(repo.join("asset.bin"), &edited).unwrap();

    let out = cdc_cmd(&repo, &["drop", "--chunks", "asset.bin"]);
    assert!(
        !out.status.success(),
        "dropping only a modified file must fail loudly"
    );
    assert_eq!(
        fs::read(repo.join("asset.bin")).unwrap(),
        edited,
        "uncommitted work survives verbatim"
    );
    assert!(
        !local_chunks(&repo).is_empty(),
        "a refused drop removes no chunks either"
    );
}

/// `--chunks` without a remote must abort before *any* side effect, not just
/// before the chunk deletion — otherwise it reports failure with every file
/// already dehydrated.
#[test]
fn drop_chunks_without_a_remote_touches_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("origin");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    utils::base_setup_repo(&repo); // deliberately no cdc.store.* config
    cdc(&repo, &["track", "*.bin"]);
    let data = test_data(3 * 1024 * 1024, 43);
    fs::write(repo.join("asset.bin"), &data).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "v1"]);
    let before = local_chunks(&repo);

    let out = cdc_cmd(&repo, &["drop", "--chunks", "asset.bin"]);
    assert!(!out.status.success(), "no remote means no safe drop");
    assert_eq!(
        fs::read(repo.join("asset.bin")).unwrap(),
        data,
        "the worktree file must be untouched, not dehydrated-then-failed"
    );
    assert_eq!(local_chunks(&repo), before);
}

/// A chunk the remote lacks is the one chunk that is not safe to delete.
#[test]
fn drop_keeps_chunks_missing_from_the_remote() {
    let tmp = tempfile::tempdir().unwrap();
    let remote_root = tmp.path().join("remote-store");
    let repo = tmp.path().join("origin");
    pushed_repo(&repo, &remote_root, "asset.bin", 44);

    let config = opendal_config(&remote_root);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let orphan = rt.block_on(async {
        let store = OpendalStore::connect(&config).unwrap();
        let hash = store.list().await.unwrap()[0].0;
        store.remove(&hash).await.unwrap();
        hash
    });

    cdc(&repo, &["drop", "--chunks", "asset.bin"]);
    assert_eq!(
        local_chunks(&repo),
        vec![orphan],
        "only the chunk the remote lacks stays behind"
    );
}

#[test]
fn drop_dry_run_changes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let remote_root = tmp.path().join("remote-store");
    let repo = tmp.path().join("origin");
    let data = pushed_repo(&repo, &remote_root, "asset.bin", 45);
    let before = local_chunks(&repo);

    let log = cdc(&repo, &["drop", "--chunks", "--dry-run", "asset.bin"]);
    assert!(log.contains("would drop 1 files"), "dry run reports: {log}");
    assert_eq!(fs::read(repo.join("asset.bin")).unwrap(), data);
    assert_eq!(local_chunks(&repo), before);
}

#[test]
fn drop_requires_an_explicit_selection() {
    let tmp = tempfile::tempdir().unwrap();
    let remote_root = tmp.path().join("remote-store");
    let repo = tmp.path().join("origin");
    let data = pushed_repo(&repo, &remote_root, "asset.bin", 46);

    // Bare `drop` must not nuke every hydrated file in the repo.
    let bare = cdc_cmd(&repo, &["drop"]);
    assert!(!bare.status.success());
    assert!(String::from_utf8_lossy(&bare.stderr).contains("--all"));
    assert_eq!(fs::read(repo.join("asset.bin")).unwrap(), data);

    // A path that names no tracked file is an error, not a silent no-op.
    assert!(!cdc_cmd(&repo, &["drop", "nope.bin"]).status.success());
}

/// Paths are cwd-relative for the user but repo-relative in the index; `drop`
/// is the first command in the CLI that has to bridge the two.
#[test]
fn drop_resolves_paths_from_a_subdirectory() {
    let tmp = tempfile::tempdir().unwrap();
    let remote_root = tmp.path().join("remote-store");
    let repo = tmp.path().join("origin");
    let data = pushed_repo(&repo, &remote_root, "sub/asset.bin", 47);
    let sub = repo.join("sub");

    cdc(&sub, &["drop", "--chunks", "asset.bin"]);

    assert!(
        fs::read(repo.join("sub/asset.bin"))
            .unwrap()
            .starts_with(b"version git-cdc/spec/v1\n")
    );
    assert_eq!(git(&repo, &["status", "--porcelain"]).trim(), "");
    cdc(&repo, &["pull"]);
    assert_eq!(fs::read(repo.join("sub/asset.bin")).unwrap(), data);
}

/// `--all` covers every tracked file, so one command reclaims a whole repo.
#[test]
fn drop_all_covers_every_tracked_file() {
    let tmp = tempfile::tempdir().unwrap();
    let remote_root = tmp.path().join("remote-store");
    let repo = tmp.path().join("origin");
    let data = pushed_repo(&repo, &remote_root, "asset.bin", 48);
    let second = test_data(3 * 1024 * 1024, 49);
    fs::write(repo.join("other.bin"), &second).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "v2"]);
    cdc(&repo, &["push"]);

    cdc(&repo, &["drop", "--chunks", "--all"]);
    assert!(local_chunks(&repo).is_empty());
    assert_eq!(git(&repo, &["status", "--porcelain"]).trim(), "");

    cdc(&repo, &["pull"]);
    assert_eq!(fs::read(repo.join("asset.bin")).unwrap(), data);
    assert_eq!(fs::read(repo.join("other.bin")).unwrap(), second);
}
