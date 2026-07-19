//! OpendalStore integration (storage-backends PLAN TASK-005). Runs against
//! the `fs` scheme in a tempdir — exercises the store logic; service-specific
//! transport (azblob, gcs, sftp, ...) is OpenDAL's contract, not ours.

use git_cdc_core::store::envelope;
use git_cdc_core::store::opendal::{OpendalConfig, OpendalStore};

fn fs_store(root: &std::path::Path) -> OpendalStore {
    OpendalStore::connect(&OpendalConfig {
        scheme: "fs".into(),
        options: vec![("root".into(), root.to_str().unwrap().into())],
        // Deliberately without trailing slash — connect must normalize.
        prefix: "chunks".into(),
    })
    .unwrap()
}

#[tokio::test]
async fn round_trip_remove_and_gc_listing() {
    let dir = tempfile::tempdir().unwrap();
    let store = fs_store(dir.path());

    let data = b"opendal chunk".to_vec();
    let hash = blake3::hash(&data);

    // Empty store: has() false, list() empty (prefix dir doesn't exist yet).
    assert!(!store.has(&hash).await.unwrap());
    assert!(store.list().await.unwrap().is_empty());

    store.put(&hash, &data).await.unwrap();
    assert!(store.has(&hash).await.unwrap());
    assert_eq!(store.get(&hash).await.unwrap(), data);

    // get_encoded returns the envelope, decodable to the chunk.
    let encoded = store.get_encoded(&hash).await.unwrap();
    assert_eq!(envelope::decode(&encoded, &hash).unwrap(), data);

    // list: our chunk with an mtime; a planted foreign object is skipped.
    std::fs::write(dir.path().join("chunks/not-a-hash"), b"foreign").unwrap();
    let listed = store.list().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].0, hash);
    assert!(listed[0].1.is_some(), "fs listing must carry an mtime for GC");

    store.remove(&hash).await.unwrap();
    assert!(!store.has(&hash).await.unwrap());
}

#[tokio::test]
async fn upload_poisoning_guards() {
    let dir = tempfile::tempdir().unwrap();
    let store = fs_store(dir.path());

    let data = b"real data".to_vec();
    let hash = blake3::hash(&data);

    // put with a wrong hash is rejected.
    let wrong = blake3::hash(b"other");
    assert!(store.put(&wrong, &data).await.is_err());

    // put_encoded with a corrupt envelope is rejected.
    let mut encoded = envelope::encode(&data);
    let last = encoded.len() - 1;
    encoded[last] ^= 0xff;
    assert!(store.put_encoded(&hash, encoded).await.is_err());

    // Nothing was admitted.
    assert!(!store.has(&hash).await.unwrap());
    assert!(store.list().await.unwrap().is_empty());
}
