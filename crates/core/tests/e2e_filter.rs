//! End-to-end filter test (PLAN 4.4): real `git add` / `git checkout`
//! against a scratch repo, byte-identical restore, manifest in the blob.

use std::fs;
use std::path::Path;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_git-cdc");

fn git(repo: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn cdc(repo: &Path, args: &[&str]) {
    let out = Command::new(BIN).args(args).current_dir(repo).output().unwrap();
    assert!(
        out.status.success(),
        "git-cdc {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn scratch_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    git(repo, &["init", "-q"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Test"]);
    cdc(repo, &["install"]);
    // install writes `git-cdc clean` expecting the binary on $PATH; tests
    // must run the freshly built binary, so point config at it directly.
    git(repo, &["config", "filter.cdc.clean", &format!("{BIN} clean")]);
    git(repo, &["config", "filter.cdc.smudge", &format!("{BIN} smudge")]);
    cdc(repo, &["track", "*.bin"]);
    dir
}

fn test_data(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed | 1;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect()
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
        blob.starts_with("version https://git-cdc.dev/spec/v1\n"),
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
    assert!(content.starts_with(b"version https://git-cdc.dev/spec/v1\n"));

    // Re-adding the passed-through manifest must not re-chunk it (clean
    // passthrough): the staged blob stays byte-identical to the manifest.
    git(repo, &["add", "asset.bin"]);
    let staged = git(repo, &["show", ":asset.bin"]);
    assert_eq!(staged.as_bytes(), &content[..]);
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
