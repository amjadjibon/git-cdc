//! Serverless S3 e2e (PLAN 3.2), env-gated: track → commit → push straight
//! to a bucket (no server) → fresh clone → pull → gc. See s3_backend.rs for
//! the MinIO invocation.

use std::fs;
use std::path::Path;
use std::process::Command;

use git_cdc_core::store::s3::{make_client, S3Config, S3Store};

const BIN: &str = env!("CARGO_BIN_EXE_git-cdc");
const BUCKET: &str = "git-cdc-test-serverless";

fn git(repo: &Path, args: &[&str]) -> String {
    // Hooks invoke `git cdc push` via $PATH — put the freshly built binary first.
    let bin_dir = Path::new(BIN).parent().unwrap();
    let path = format!("{}:{}", bin_dir.display(), std::env::var("PATH").unwrap());
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .env("PATH", path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn cdc(repo: &Path, args: &[&str]) -> String {
    let out = Command::new(BIN).args(args).current_dir(repo).output().unwrap();
    assert!(
        out.status.success(),
        "git-cdc {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn setup_repo(repo: &Path, endpoint: &str) {
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Test"]);
    cdc(repo, &["install"]);
    git(repo, &["config", "filter.cdc.clean", &format!("{BIN} clean")]);
    git(repo, &["config", "filter.cdc.smudge", &format!("{BIN} smudge")]);
    git(repo, &["config", "cdc.s3.bucket", BUCKET]);
    git(repo, &["config", "cdc.s3.prefix", "chunks/"]);
    git(repo, &["config", "cdc.s3.endpoint", endpoint]);
    git(repo, &["config", "cdc.s3.force-path-style", "true"]);
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
fn serverless_push_clone_pull_gc() {
    let Ok(endpoint) = std::env::var("GIT_CDC_TEST_S3_ENDPOINT") else {
        eprintln!("skipped: set GIT_CDC_TEST_S3_ENDPOINT (+ AWS env creds) to run");
        return;
    };
    let config = S3Config {
        bucket: BUCKET.into(),
        prefix: "chunks/".into(),
        endpoint: Some(endpoint.clone()),
        force_path_style: true,
    };
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let client = make_client(&config).await;
        let _ = client.create_bucket().bucket(BUCKET).send().await;
        // Empty the prefix so counts are deterministic across runs.
        let store = S3Store::connect(&config).await;
        for (hash, _) in store.list().await.unwrap() {
            store.remove(&hash).await.unwrap();
        }
    });
    let count_bucket = || {
        rt.block_on(async { S3Store::connect(&config).await.list().await.unwrap().len() })
    };

    let tmp = tempfile::tempdir().unwrap();

    // origin: v1 + v2, push straight to the bucket.
    let repo = tmp.path().join("origin");
    fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    setup_repo(&repo, &endpoint);
    cdc(&repo, &["track", "*.bin"]);

    let mut data = test_data(12 * 1024 * 1024, 77);
    fs::write(repo.join("asset.bin"), &data).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "v1"]);
    cdc(&repo, &["push"]);
    let count_v1 = count_bucket();
    assert!(count_v1 >= 2, "12 MiB should be several chunks");

    data[6_000_000] ^= 0xFF;
    fs::write(repo.join("asset.bin"), &data).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "v2"]);
    let log = cdc(&repo, &["push"]);
    assert!(
        count_bucket() - count_v1 <= 2,
        "1-byte edit should upload few chunks ({log})"
    );

    // fresh clone via bare remote: passthrough, then pull from the bucket.
    let remote = tmp.path().join("remote.git");
    git(tmp.path(), &["init", "-q", "--bare", &remote.to_string_lossy()]);
    git(&repo, &["remote", "add", "origin", &remote.to_string_lossy()]);
    git(&repo, &["push", "-q", "origin", "main"]);

    let clone = tmp.path().join("clone");
    git(tmp.path(), &["clone", "-q", &remote.to_string_lossy(), &clone.to_string_lossy()]);
    setup_repo(&clone, &endpoint);
    assert!(fs::read(clone.join("asset.bin"))
        .unwrap()
        .starts_with(b"version git-cdc/spec/v1\n"));
    cdc(&clone, &["pull"]);
    assert_eq!(fs::read(clone.join("asset.bin")).unwrap(), data, "v2 materialized from bucket");

    // gc: drop v2 everywhere (incl. the remote-tracking ref rev-list sees),
    // bucket sweep removes its unique chunks.
    git(&repo, &["reset", "-q", "--hard", "HEAD~1"]);
    git(&repo, &["push", "-q", "--force", "origin", "main"]);
    let before = count_bucket();
    cdc(&repo, &["gc", "--dry-run", "--grace-secs", "0"]);
    assert_eq!(count_bucket(), before, "dry run deletes nothing");
    cdc(&repo, &["gc", "--grace-secs", "0"]);
    assert_eq!(count_bucket(), count_v1, "bucket back to the v1 chunk set");
}
