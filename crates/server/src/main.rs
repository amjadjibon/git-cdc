use std::time::Duration;

use clap::Parser;
use git_cdc_server::{app, AppState};
use git_cdc_core::store::DiskStore;

#[derive(Parser)]
#[command(name = "git-cdc-server", about = "git-cdc chunk CAS + batch API server")]
struct Args {
    /// Chunk store root directory
    #[arg(long, env = "GIT_CDC_ROOT")]
    root: std::path::PathBuf,
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
    let state = AppState {
        store: DiskStore::new(&args.root),
        token: args.token,
        grace: Duration::from_secs(args.grace_secs),
    };
    let listener = tokio::net::TcpListener::bind(&args.listen).await?;
    eprintln!("git-cdc-server listening on {}", listener.local_addr()?);
    axum::serve(listener, app(state)).await?;
    Ok(())
}
