//! In-process S3 server for tests (s3s-fs over a temp dir), so the S3 suites
//! run in a plain `cargo test` with no docker/MinIO. Set
//! GIT_CDC_TEST_S3_ENDPOINT (+ AWS env creds) to target a real S3 instead.

use std::path::PathBuf;

use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as ConnBuilder;

const ACCESS_KEY: &str = "git-cdc-test";
const SECRET_KEY: &str = "git-cdc-test-secret";

/// Returns the endpoint URL and, for the in-process server, the TempDir
/// backing it (keep it alive for the duration of the test).
pub fn endpoint() -> (String, Option<tempfile::TempDir>) {
    if let Ok(ep) = std::env::var("GIT_CDC_TEST_S3_ENDPOINT") {
        return (ep, None);
    }
    // The SDK (and CLI subprocesses) read credentials from the environment.
    // SAFETY: called once at test start, before any credential lookup.
    unsafe {
        std::env::set_var("AWS_ACCESS_KEY_ID", ACCESS_KEY);
        std::env::set_var("AWS_SECRET_ACCESS_KEY", SECRET_KEY);
    }
    let dir = tempfile::tempdir().unwrap();
    let ep = spawn(dir.path().to_path_buf());
    (ep, Some(dir))
}

fn spawn(root: PathBuf) -> String {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async move {
            let fs = s3s_fs::FileSystem::new(root).unwrap();
            let service = {
                let mut b = s3s::service::S3ServiceBuilder::new(fs);
                b.set_auth(s3s::auth::SimpleAuth::from_single(ACCESS_KEY, SECRET_KEY));
                b.build()
            };
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            tx.send(format!("http://{}", listener.local_addr().unwrap())).unwrap();
            let http = ConnBuilder::new(TokioExecutor::new());
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                let conn = http.serve_connection(TokioIo::new(socket), service.clone()).into_owned();
                tokio::spawn(async move {
                    let _ = conn.await;
                });
            }
        });
    });
    rx.recv().unwrap()
}
