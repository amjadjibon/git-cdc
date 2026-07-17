use std::time::Duration;

use anyhow::bail;
use clap::{Parser, ValueEnum};
use git_cdc_core::s3::{S3Config, S3Store};
use git_cdc_core::store::DiskStore;
use git_cdc_server::{app, AppState, Backend};

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum BackendKind {
    Disk,
    S3,
}

#[derive(Parser)]
#[command(name = "git-cdc-server", about = "git-cdc chunk CAS + batch API server")]
struct Args {
    /// Chunk storage backend
    #[arg(long, value_enum, default_value = "disk")]
    backend: BackendKind,
    /// Chunk store root directory (disk backend)
    #[arg(long, env = "GIT_CDC_ROOT", required_if_eq("backend", "disk"))]
    root: Option<std::path::PathBuf>,
    /// S3 bucket (s3 backend)
    #[arg(long, env = "GIT_CDC_S3_BUCKET", required_if_eq("backend", "s3"))]
    s3_bucket: Option<String>,
    /// Key prefix inside the bucket, e.g. "chunks/"
    #[arg(long, default_value = "")]
    s3_prefix: String,
    /// Custom endpoint for MinIO/R2, e.g. http://127.0.0.1:9000
    #[arg(long, env = "GIT_CDC_S3_ENDPOINT")]
    s3_endpoint: Option<String>,
    /// Path-style addressing (required by MinIO)
    #[arg(long)]
    s3_force_path_style: bool,
    /// Static bearer token clients must present
    #[arg(long, env = "GIT_CDC_TOKEN")]
    token: String,
    /// Listen address
    #[arg(long, env = "GIT_CDC_LISTEN", default_value = "127.0.0.1:8077")]
    listen: String,
    /// GC grace period in seconds (unreferenced chunks younger than this survive)
    #[arg(long, default_value_t = 24 * 3600)]
    grace_secs: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let backend = match args.backend {
        BackendKind::Disk => {
            let Some(root) = args.root else { bail!("--root is required for the disk backend") };
            Backend::Disk(DiskStore::new(root))
        }
        BackendKind::S3 => {
            let Some(bucket) = args.s3_bucket else {
                bail!("--s3-bucket is required for the s3 backend")
            };
            Backend::S3(
                S3Store::connect(&S3Config {
                    bucket,
                    prefix: args.s3_prefix,
                    endpoint: args.s3_endpoint,
                    force_path_style: args.s3_force_path_style,
                })
                .await,
            )
        }
    };
    let state = AppState {
        backend,
        token: args.token,
        grace: Duration::from_secs(args.grace_secs),
    };
    let listener = tokio::net::TcpListener::bind(&args.listen).await?;
    eprintln!("git-cdc-server listening on {}", listener.local_addr()?);
    axum::serve(listener, app(state)).await?;
    Ok(())
}
