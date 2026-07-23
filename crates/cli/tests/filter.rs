//! End-to-end filter test (PLAN 4.4): real `git add` / `git checkout`
//! against a scratch repo, byte-identical restore, manifest in the blob.

use std::fs;
use std::process::Command;

use git_cdc_core::chunker::test_util::test_data;

mod utils;

use utils::{BIN, cdc, git};

fn scratch_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    git(repo, &["init", "-q"]);
    utils::base_setup_repo(repo);
    cdc(repo, &["track", "*.bin"]);
    dir
}

#[test]
fn add_checkout_round_trip_is_byte_identical() {
    let dir = scratch_repo();
    let repo = dir.path();
    let data = test_data(3 * 1024 * 1024, 11);
    fs::write(repo.join("asset.bin"), &data).unwrap();

    git(repo, &["add", ".gitattributes", "asset.bin"]);
    git(repo, &["commit", "-q", "-m", "add asset"]);

    // The committed blob is a manifest, not the file content.
    let blob = git(repo, &["show", "HEAD:asset.bin"]);
    assert!(
        blob.starts_with("version git-cdc/spec/v1\n"),
        "committed blob is not a manifest: {:.80}",
        blob
    );
    assert!(blob.len() < 4096, "manifest should be small");

    // Delete and restore through smudge: byte-identical.
    fs::remove_file(repo.join("asset.bin")).unwrap();
    git(repo, &["checkout", "--", "asset.bin"]);
    assert_eq!(fs::read(repo.join("asset.bin")).unwrap(), data);
}

#[test]
fn smudge_with_empty_store_passes_manifest_through() {
    let dir = scratch_repo();
    let repo = dir.path();
    let data = test_data(2 * 1024 * 1024, 5);
    fs::write(repo.join("asset.bin"), &data).unwrap();
    git(repo, &["add", ".gitattributes", "asset.bin"]);
    git(repo, &["commit", "-q", "-m", "add asset"]);

    // Simulate a fresh clone: wipe the local chunk store, force re-smudge.
    fs::remove_dir_all(repo.join(".git/cdc")).unwrap();
    fs::remove_file(repo.join("asset.bin")).unwrap();
    git(repo, &["checkout", "--", "asset.bin"]);

    // Checkout succeeded (no hard error) and the worktree holds manifest text.
    let content = fs::read(repo.join("asset.bin")).unwrap();
    assert!(content.starts_with(b"version git-cdc/spec/v1\n"));

    // Re-adding the passed-through manifest must not re-chunk it (clean
    // passthrough): the staged blob stays byte-identical to the manifest.
    git(repo, &["add", "asset.bin"]);
    let staged = git(repo, &["show", ":asset.bin"]);
    assert_eq!(staged.as_bytes(), &content[..]);
}

#[test]
fn diff_reports_changed_chunks() {
    let dir = scratch_repo();
    let repo = dir.path();

    let mut data = test_data(20 * 1024 * 1024, 33);
    fs::write(repo.join("asset.bin"), &data).unwrap();
    git(repo, &["add", "."]);
    git(repo, &["commit", "-q", "-m", "v1"]);
    fs::write(
        repo.join("a.manifest"),
        git(repo, &["show", "HEAD:asset.bin"]),
    )
    .unwrap();

    data[10_000_000] ^= 0xFF;
    fs::write(repo.join("asset.bin"), &data).unwrap();
    git(repo, &["add", "asset.bin"]);
    git(repo, &["commit", "-q", "-m", "v2"]);
    fs::write(
        repo.join("b.manifest"),
        git(repo, &["show", "HEAD:asset.bin"]),
    )
    .unwrap();

    let out = Command::new(BIN)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .args(["diff", "a.manifest", "b.manifest"])
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("added: 1 chunks") || text.contains("added: 2 chunks"),
        "1-byte edit should change 1-2 chunks: {text}"
    );
}

#[test]
fn chunk_sizes_configurable_via_gitconfig() {
    let dir = scratch_repo();
    let repo = dir.path();
    // git's --type=int suffixes: 64k/256k/1m.
    git(repo, &["config", "cdc.chunk.min", "64k"]);
    git(repo, &["config", "cdc.chunk.avg", "256k"]);
    git(repo, &["config", "cdc.chunk.max", "1m"]);

    let data = test_data(4 * 1024 * 1024, 3);
    fs::write(repo.join("asset.bin"), &data).unwrap();
    git(repo, &["add", ".gitattributes", "asset.bin"]);
    git(repo, &["commit", "-q", "-m", "add asset"]);

    // Manifest headers echo the configured values; chunks respect the max.
    let blob = git(repo, &["show", "HEAD:asset.bin"]);
    assert!(blob.contains("\nchunk-min 65536\n"), "{blob:.200}");
    assert!(blob.contains("\nchunk-avg 262144\n"));
    assert!(blob.contains("\nchunk-max 1048576\n"));
    let chunk_lines = blob.lines().filter(|l| l.starts_with("chunk ")).count();
    assert!(
        chunk_lines >= 4,
        "4 MiB at ≤1 MiB per chunk needs ≥4, got {chunk_lines}"
    );

    // Restore stays byte-identical with custom params.
    fs::remove_file(repo.join("asset.bin")).unwrap();
    git(repo, &["checkout", "--", "asset.bin"]);
    assert_eq!(fs::read(repo.join("asset.bin")).unwrap(), data);
}

#[test]
fn invalid_chunk_config_fails_the_add() {
    let dir = scratch_repo();
    let repo = dir.path();
    git(repo, &["config", "cdc.chunk.min", "63"]); // below fastcdc's 64-byte floor

    fs::write(repo.join("asset.bin"), test_data(1024, 1)).unwrap();
    let out = Command::new("git")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .args(["add", "asset.bin"])
        .current_dir(repo)
        .output()
        .unwrap();
    // Clean must hard-error (never chunk with unvalidated params); with the
    // filter not `required`, git may still stage the raw file — the essential
    // assertion is the loud complaint naming the key.
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("cdc.chunk.min"),
        "add must surface the invalid config: {err}"
    );
}

#[test]
fn filter_process_alone_round_trips_many_files() {
    // Only filter.cdc.process configured — no clean/smudge fallback keys —
    // so every byte here provably moved through the pkt-line protocol, and
    // one process serviced all files in each git operation.
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    git(repo, &["init", "-q"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Test"]);
    git(
        repo,
        &[
            "config",
            "filter.cdc.process",
            &format!("{BIN} filter-process"),
        ],
    );
    fs::write(repo.join(".gitattributes"), "*.bin filter=cdc -text\n").unwrap();

    // Several files, all larger than one 64 KiB packet; one crosses chunks.
    let files: Vec<(String, Vec<u8>)> = (0..4u64)
        .map(|i| {
            (
                format!("asset{i}.bin"),
                test_data(700 * 1024 + i as usize, 100 + i),
            )
        })
        .chain([("big.bin".to_string(), test_data(3 * 1024 * 1024, 55))])
        .collect();
    for (name, data) in &files {
        fs::write(repo.join(name), data).unwrap();
    }
    git(repo, &["add", "."]);
    git(repo, &["commit", "-q", "-m", "assets"]);

    for (name, _) in &files {
        let blob = git(repo, &["show", &format!("HEAD:{name}")]);
        assert!(
            blob.starts_with("version git-cdc/spec/v1\n"),
            "{name} not a manifest"
        );
        fs::remove_file(repo.join(name)).unwrap();
    }
    git(repo, &["checkout", "--", "."]);
    for (name, data) in &files {
        assert_eq!(&fs::read(repo.join(name)).unwrap(), data, "{name} restored");
    }
}

#[test]
fn one_shot_filters_still_work_without_process() {
    // git < 2.11 fallback: unset filter.cdc.process, forcing the one-shot
    // clean/smudge paths.
    let dir = scratch_repo();
    let repo = dir.path();
    git(repo, &["config", "--unset", "filter.cdc.process"]);

    let data = test_data(2 * 1024 * 1024, 21);
    fs::write(repo.join("asset.bin"), &data).unwrap();
    git(repo, &["add", ".gitattributes", "asset.bin"]);
    git(repo, &["commit", "-q", "-m", "add asset"]);
    assert!(git(repo, &["show", "HEAD:asset.bin"]).starts_with("version git-cdc/spec/v1\n"));
    fs::remove_file(repo.join("asset.bin")).unwrap();
    git(repo, &["checkout", "--", "asset.bin"]);
    assert_eq!(fs::read(repo.join("asset.bin")).unwrap(), data);
}

#[test]
fn smudge_never_emits_corrupt_data() {
    let dir = scratch_repo();
    let repo = dir.path();
    let data = test_data(3 * 1024 * 1024, 77);
    fs::write(repo.join("asset.bin"), &data).unwrap();
    git(repo, &["add", ".gitattributes", "asset.bin"]);
    git(repo, &["commit", "-q", "-m", "add asset"]);

    // Corrupt every stored chunk in place.
    for shard1 in fs::read_dir(repo.join(".git/cdc/objects")).unwrap() {
        for shard2 in fs::read_dir(shard1.unwrap().path()).unwrap() {
            for chunk in fs::read_dir(shard2.unwrap().path()).unwrap() {
                let path = chunk.unwrap().path();
                let mut bytes = fs::read(&path).unwrap();
                bytes[0] ^= 0xFF;
                fs::write(&path, bytes).unwrap();
            }
        }
    }

    fs::remove_file(repo.join("asset.bin")).unwrap();
    let out = Command::new("git")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .args(["checkout", "--", "asset.bin"])
        .current_dir(repo)
        .output()
        .unwrap();

    // The one outcome that must never happen: corrupt bytes materialized as
    // if they were the real file. Either checkout fails, or (filter not
    // `required`) git falls back to the manifest blob.
    if let Ok(content) = fs::read(repo.join("asset.bin")) {
        assert_ne!(content, data, "corrupt store cannot reproduce the original");
        assert!(
            content.starts_with(b"version git-cdc/spec/v1\n") || !out.status.success(),
            "smudge emitted {} bytes of non-manifest data from a corrupt store",
            content.len()
        );
    }
}

#[test]
fn track_without_patterns_errors() {
    let dir = scratch_repo();
    let out = Command::new(BIN)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .args(["track"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("usage"));
}

#[test]
fn install_leaves_foreign_pre_push_hook_alone() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    git(repo, &["init", "-q"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Test"]);

    let hook = repo.join(".git/hooks/pre-push");
    fs::create_dir_all(hook.parent().unwrap()).unwrap();
    let foreign = "#!/bin/sh\necho my own hook\n";
    fs::write(&hook, foreign).unwrap();

    let out = Command::new(BIN)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .args(["install"])
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("already exists"),
        "user must be told to wire the hook manually"
    );
    assert_eq!(
        fs::read_to_string(&hook).unwrap(),
        foreign,
        "foreign hook must not be clobbered"
    );
}

#[test]
fn install_and_track_are_idempotent() {
    let dir = scratch_repo();
    let repo = dir.path();

    let hook = fs::read_to_string(repo.join(".git/hooks/pre-push")).unwrap();
    assert!(hook.contains("git cdc push"));

    cdc(repo, &["install"]);
    cdc(repo, &["track", "*.bin"]);
    let attrs = fs::read_to_string(repo.join(".gitattributes")).unwrap();
    assert_eq!(
        attrs.matches("*.bin filter=cdc -text").count(),
        1,
        "track must not duplicate lines"
    );

    // Untracked files (no filter match) commit as-is.
    fs::write(repo.join("readme.txt"), "plain text").unwrap();
    git(repo, &["add", "readme.txt"]);
    git(repo, &["commit", "-q", "-m", "plain"]);
    assert_eq!(git(repo, &["show", "HEAD:readme.txt"]), "plain text");
}
