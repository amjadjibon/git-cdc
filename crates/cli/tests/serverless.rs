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
    assert_eq!(
        git(&repo, &["rev-parse", ":asset.bin"]),
        git(&repo, &["rev-parse", "HEAD:asset.bin"]),
        "pull stages nothing — the blob is unchanged"
    );
    assert_eq!(
        git(&repo, &["status", "--porcelain"]).trim(),
        "",
        "pull settles the stat cache — a rehydrated file is not 'modified'"
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
    assert!(log.contains("would drop"), "dry run reports: {log}");
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

/// Git C-quotes non-ASCII paths ("caf\303\251.bin") unless told otherwise,
/// and the quoted form matches no file on disk — so a tracked file with an
/// accent in its name must still resolve.
#[test]
fn drop_handles_non_ascii_filenames() {
    let tmp = tempfile::tempdir().unwrap();
    let remote_root = tmp.path().join("remote-store");
    let repo = tmp.path().join("origin");
    let data = pushed_repo(&repo, &remote_root, "café.bin", 52);

    cdc(&repo, &["drop", "--chunks", "café.bin"]);

    assert!(
        fs::read(repo.join("café.bin"))
            .unwrap()
            .starts_with(b"version git-cdc/spec/v1\n")
    );
    assert!(local_chunks(&repo).is_empty());
    assert_eq!(git(&repo, &["status", "--porcelain"]).trim(), "");
    cdc(&repo, &["pull"]);
    assert_eq!(fs::read(repo.join("café.bin")).unwrap(), data);
}

/// A file skipped for uncommitted edits must keep its chunks, even when a
/// sibling file in the same `--all` run is dropped — otherwise `git checkout`
/// of the skipped file quietly degrades to manifest passthrough.
#[test]
fn drop_keeps_chunks_of_skipped_files() {
    let tmp = tempfile::tempdir().unwrap();
    let remote_root = tmp.path().join("remote-store");
    let repo = tmp.path().join("origin");
    pushed_repo(&repo, &remote_root, "clean.bin", 53);
    let kept = test_data(3 * 1024 * 1024, 54);
    fs::write(repo.join("edited.bin"), &kept).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "v2"]);
    cdc(&repo, &["push"]);

    // edited.bin now diverges from its manifest; clean.bin does not.
    let edits = b"uncommitted".to_vec();
    fs::write(repo.join("edited.bin"), &edits).unwrap();

    cdc(&repo, &["drop", "--chunks", "--all"]);

    assert_eq!(fs::read(repo.join("edited.bin")).unwrap(), edits);
    assert!(
        fs::read(repo.join("clean.bin"))
            .unwrap()
            .starts_with(b"version git-cdc/spec/v1\n")
    );
    // The skipped file's chunks are still local: checkout restores it.
    git(&repo, &["checkout", "-q", "--", "edited.bin"]);
    assert_eq!(
        fs::read(repo.join("edited.bin")).unwrap(),
        kept,
        "a skipped file's chunks must not be swept with its sibling's"
    );
}

/// `drop` then `drop --chunks` on the same file: the second run sees manifest
/// text and must still reclaim the chunks the first run left behind.
#[test]
fn drop_chunks_reaches_an_already_dropped_file() {
    let tmp = tempfile::tempdir().unwrap();
    let remote_root = tmp.path().join("remote-store");
    let repo = tmp.path().join("origin");
    let data = pushed_repo(&repo, &remote_root, "asset.bin", 55);

    cdc(&repo, &["drop", "asset.bin"]); // worktree half only
    assert!(
        !local_chunks(&repo).is_empty(),
        "chunks stay without --chunks"
    );

    cdc(&repo, &["drop", "--chunks", "asset.bin"]);
    assert!(local_chunks(&repo).is_empty(), "second run reclaims them");

    cdc(&repo, &["pull"]);
    assert_eq!(fs::read(repo.join("asset.bin")).unwrap(), data);
}

/// Dedup means one chunk can belong to both a dropped file and a skipped one.
/// Identical content makes every chunk shared, so nothing may be reclaimed
/// while the skipped file still needs it locally.
#[test]
fn drop_keeps_chunks_shared_with_a_skipped_file() {
    let tmp = tempfile::tempdir().unwrap();
    let remote_root = tmp.path().join("remote-store");
    let repo = tmp.path().join("origin");
    let data = pushed_repo(&repo, &remote_root, "clean.bin", 56);
    // Byte-identical copy: the two files share every chunk.
    fs::write(repo.join("copy.bin"), &data).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "copy"]);
    cdc(&repo, &["push"]);
    let before = local_chunks(&repo);

    fs::write(repo.join("copy.bin"), b"uncommitted").unwrap();
    cdc(&repo, &["drop", "--chunks", "--all"]);

    assert_eq!(
        local_chunks(&repo),
        before,
        "every chunk is shared with the skipped file, so none may be swept"
    );
    git(&repo, &["checkout", "-q", "--", "copy.bin"]);
    assert_eq!(fs::read(repo.join("copy.bin")).unwrap(), data);
}

/// `pull` settles git's stat cache by stat-only means, so it can never stage
/// a re-chunked manifest. If `cdc.chunk.*` changed since the commit the file
/// genuinely diverges — it must stay honestly unstaged-modified, never
/// silently staged.
#[test]
fn pull_never_stages_a_rechunked_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let remote_root = tmp.path().join("remote-store");
    let repo = tmp.path().join("origin");
    pushed_repo(&repo, &remote_root, "asset.bin", 57);
    let committed = git(&repo, &["rev-parse", "HEAD:asset.bin"]);

    cdc(&repo, &["drop", "--chunks", "asset.bin"]);
    // inherit=false so re-chunking really would diverge — otherwise `clean`
    // reuses the committed parameters and there is nothing to get wrong.
    git(&repo, &["config", "cdc.chunk.inherit", "false"]);
    git(&repo, &["config", "cdc.chunk.avg", "1048576"]);
    cdc(&repo, &["pull"]);

    assert_eq!(
        git(&repo, &["rev-parse", ":asset.bin"]),
        committed,
        "the index blob must still be the committed manifest"
    );
}

/// An executable tracked file must not come back as mode 100644 after pull
/// rewrites its index entry.
#[test]
fn pull_preserves_the_file_mode() {
    let tmp = tempfile::tempdir().unwrap();
    let remote_root = tmp.path().join("remote-store");
    let repo = tmp.path().join("origin");
    pushed_repo(&repo, &remote_root, "asset.bin", 58);
    // Both halves: the file on disk and the index entry. `--chmod=+x` alone
    // would leave the worktree at 0644 and the repo dirty before pull runs.
    let exe = repo.join("asset.bin");
    let mut perms = fs::metadata(&exe).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    fs::set_permissions(&exe, perms).unwrap();
    git(&repo, &["update-index", "--chmod=+x", "asset.bin"]);
    git(&repo, &["commit", "-q", "-m", "chmod"]);
    assert!(git(&repo, &["ls-files", "-s", "asset.bin"]).starts_with("100755"));
    assert_eq!(git(&repo, &["status", "--porcelain"]).trim(), "");

    cdc(&repo, &["drop", "--chunks", "asset.bin"]);
    cdc(&repo, &["pull"]);

    assert!(
        git(&repo, &["ls-files", "-s", "asset.bin"]).starts_with("100755"),
        "pull must not downgrade an executable file's mode"
    );
    assert_eq!(git(&repo, &["status", "--porcelain"]).trim(), "");
}

/// Chunk boundaries are reproducible only for a given parameter set, so a
/// clone configured differently re-chunks files nobody edited — silently
/// losing dedup. `pull` must say so and name the config that matches.
#[test]
fn pull_warns_when_chunk_config_diverges() {
    let tmp = tempfile::tempdir().unwrap();
    let remote_root = tmp.path().join("remote-store");
    let repo = tmp.path().join("origin");
    pushed_repo(&repo, &remote_root, "asset.bin", 59);
    let committed_avg = git(&repo, &["show", "HEAD:asset.bin"])
        .lines()
        .find_map(|l| l.strip_prefix("chunk-avg ").map(str::to_string))
        .expect("manifest records chunk-avg");

    // Matching config (the default): no warning.
    cdc(&repo, &["drop", "--chunks", "--all"]);
    let quiet = cdc(&repo, &["pull"]);
    assert!(
        !quiet.contains("chunk parameters"),
        "must not warn when the config matches: {quiet}"
    );

    // Diverging config: warn, and name the committed values to restore.
    cdc(&repo, &["drop", "--chunks", "--all"]);
    git(&repo, &["config", "cdc.chunk.avg", "1048576"]);
    let loud = cdc(&repo, &["pull"]);
    assert!(
        loud.contains("chunk parameters"),
        "diverging config must warn: {loud}"
    );
    assert!(
        loud.contains(&format!("git config cdc.chunk.avg {committed_avg}")),
        "warning must name the committed avg ({committed_avg}): {loud}"
    );

    // Content is never at risk either way — reassembly goes by chunk hash.
    assert_eq!(
        git(&repo, &["rev-parse", ":asset.bin"]),
        git(&repo, &["rev-parse", "HEAD:asset.bin"]),
        "a diverging config must not stage a re-chunked manifest"
    );
}

/// The point of inheriting: a clone configured differently must not re-chunk
/// a tracked file it did not edit. Without this, `git add` rewrites the
/// manifest and `push` uploads a second copy of identical content.
#[test]
fn clean_inherits_committed_chunk_params() {
    let tmp = tempfile::tempdir().unwrap();
    let remote_root = tmp.path().join("remote-store");
    let repo = tmp.path().join("origin");
    pushed_repo(&repo, &remote_root, "asset.bin", 60);
    let committed = git(&repo, &["show", "HEAD:asset.bin"]);
    let remote_before = count_remote_objects(&remote_root);

    // This clone wants 1 MiB chunks; the file was committed at the default.
    git(&repo, &["config", "cdc.chunk.avg", "1048576"]);
    fs::write(repo.join("notes.txt"), b"unrelated").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "unrelated change"]);

    assert_eq!(
        git(&repo, &["show", "HEAD:asset.bin"]),
        committed,
        "a file nobody edited must keep its committed manifest verbatim"
    );
    cdc(&repo, &["push"]);
    assert_eq!(
        count_remote_objects(&remote_root),
        remote_before,
        "no duplicate chunks for content the remote already has"
    );
}

/// `cdc.chunk.inherit=false` is the opt-out for deliberately re-chunking a
/// repo onto new settings.
#[test]
fn chunk_inherit_false_uses_the_local_config() {
    let tmp = tempfile::tempdir().unwrap();
    let remote_root = tmp.path().join("remote-store");
    let repo = tmp.path().join("origin");
    pushed_repo(&repo, &remote_root, "asset.bin", 61);
    let before = git(&repo, &["show", "HEAD:asset.bin"]);
    assert!(before.contains("chunk-avg 2097152"));

    git(&repo, &["config", "cdc.chunk.inherit", "false"]);
    git(&repo, &["config", "cdc.chunk.avg", "1048576"]);
    // Touch the file so git re-runs the clean filter on it.
    let data = fs::read(repo.join("asset.bin")).unwrap();
    fs::write(repo.join("asset.bin"), &data).unwrap();
    git(&repo, &["add", "--renormalize", "."]);
    git(&repo, &["commit", "-q", "-m", "rechunk"]);

    assert!(
        git(&repo, &["show", "HEAD:asset.bin"]).contains("chunk-avg 1048576"),
        "opting out must honour the local config"
    );
}

/// A brand-new file has no committed manifest to inherit from, so it uses
/// this repo's config — the one case where divergence is unavoidable.
#[test]
fn new_files_use_the_local_chunk_config() {
    let tmp = tempfile::tempdir().unwrap();
    let remote_root = tmp.path().join("remote-store");
    let repo = tmp.path().join("origin");
    pushed_repo(&repo, &remote_root, "asset.bin", 62);

    git(&repo, &["config", "cdc.chunk.avg", "1048576"]);
    fs::write(repo.join("fresh.bin"), test_data(3 * 1024 * 1024, 63)).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "fresh"]);

    assert!(git(&repo, &["show", "HEAD:fresh.bin"]).contains("chunk-avg 1048576"));
    assert!(git(&repo, &["show", "HEAD:asset.bin"]).contains("chunk-avg 2097152"));
}

fn count_remote_objects(remote_root: &Path) -> usize {
    fn walk(dir: &Path, n: &mut usize) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, n);
            } else {
                *n += 1;
            }
        }
    }
    let mut n = 0;
    walk(remote_root, &mut n);
    n
}
