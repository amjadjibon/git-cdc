//! S3 backend integration (PLAN 3.2). Runs against an in-process s3s-fs
//! server by default — no external S3 needed. Set GIT_CDC_TEST_S3_ENDPOINT
//! (+ AWS env creds) to point it at a real MinIO/RustFS/S3 instead.

use std::time::Duration;

use git_cdc_core::store::{OpendalConfig, OpendalStore};

mod s3_fixture;

fn test_config(bucket: &str, endpoint: String) -> OpendalConfig {
    OpendalConfig::s3(bucket.into(), "chunks/".into(), Some(endpoint), true)
}

/// s3s-fs buckets are directories under the fixture root; a real endpoint
/// (GIT_CDC_TEST_S3_ENDPOINT) must have the bucket pre-created.
fn ensure_bucket(bucket: &str, s3_dir: &Option<tempfile::TempDir>) {
    if let Some(dir) = s3_dir {
        std::fs::create_dir_all(dir.path().join(bucket)).unwrap();
    }
}

#[tokio::test]
async fn s3_store_round_trip_and_gc_metadata() {
    let (endpoint, s3_dir) = s3_fixture::endpoint();
    let config = test_config("git-cdc-test-backend", endpoint);
    ensure_bucket("git-cdc-test-backend", &s3_dir);
    let store = OpendalStore::connect(&config).unwrap();

    let data = format!("chunk-{}", std::process::id()).into_bytes();
    let hash = blake3::hash(&data);

    // put/has/get round trip with verification.
    assert!(!store.has(&hash).await.unwrap());
    store.put(&hash, &data).await.unwrap();
    assert!(store.has(&hash).await.unwrap());
    assert_eq!(store.get(&hash).await.unwrap(), data);

    // put rejects corrupt data.
    let wrong = blake3::hash(b"something else");
    assert!(store.put(&wrong, &data).await.is_err());

    // list sees the chunk with a LastModified age.
    let listed = store.list().await.unwrap();
    let entry = listed
        .iter()
        .find(|(h, _)| *h == hash)
        .expect("chunk listed");
    let age = entry
        .1
        .and_then(|m| std::time::SystemTime::now().duration_since(m).ok())
        .expect("LastModified present");
    assert!(age < Duration::from_secs(300), "fresh chunk, sane clock");

    // remove.
    store.remove(&hash).await.unwrap();
    assert!(!store.has(&hash).await.unwrap());
}
