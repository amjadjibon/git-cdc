//! S3 backend integration (PLAN 3.2), env-gated: no S3 in the default test
//! environment. Run with MinIO:
//!
//! ```sh
//! docker run -d -p 9000:9000 minio/minio server /data
//! GIT_CDC_TEST_S3_ENDPOINT=http://127.0.0.1:9000 \
//! AWS_ACCESS_KEY_ID=minioadmin AWS_SECRET_ACCESS_KEY=minioadmin \
//! cargo test -p git-cdc-server --test s3_backend
//! ```

use std::time::Duration;

use git_cdc_core::s3::{make_client, S3Config, S3Store};

fn test_config(bucket: &str) -> Option<S3Config> {
    let endpoint = std::env::var("GIT_CDC_TEST_S3_ENDPOINT").ok()?;
    Some(S3Config {
        bucket: bucket.into(),
        prefix: "chunks/".into(),
        endpoint: Some(endpoint),
        force_path_style: true,
    })
}

async fn ensure_bucket(config: &S3Config) {
    let client = make_client(config).await;
    // Idempotent: BucketAlreadyOwnedByYou is fine.
    let _ = client.create_bucket().bucket(&config.bucket).send().await;
}

#[tokio::test]
async fn s3_store_round_trip_and_gc_metadata() {
    let Some(config) = test_config("git-cdc-test-backend") else {
        eprintln!("skipped: set GIT_CDC_TEST_S3_ENDPOINT (+ AWS env creds) to run");
        return;
    };
    ensure_bucket(&config).await;
    let store = S3Store::connect(&config).await;

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
    let entry = listed.iter().find(|(h, _)| *h == hash).expect("chunk listed");
    let age = entry
        .1
        .and_then(|m| std::time::SystemTime::now().duration_since(m).ok())
        .expect("LastModified present");
    assert!(age < Duration::from_secs(300), "fresh chunk, sane clock");

    // remove.
    store.remove(&hash).await.unwrap();
    assert!(!store.has(&hash).await.unwrap());
}
