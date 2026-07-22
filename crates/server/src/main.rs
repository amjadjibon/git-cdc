use std::time::Duration;

use anyhow::bail;
use clap::{Parser, ValueEnum};
use git_cdc_core::store::{DiskStore, OpendalConfig, OpendalStore};
use git_cdc_server::{AppState, Backend, app};

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum BackendKind {
    Disk,
    /// Any OpenDAL service: s3, azblob, azfile, b2, dropbox, gcs, sftp,
    /// ftp, gdrive, swift, webdav, onedrive
    Opendal,
}

/// Split a `KEY=VALUE` --opendal-option argument.
fn parse_key_value(s: &str) -> Result<(String, String), String> {
    s.split_once('=')
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .ok_or_else(|| format!("expected KEY=VALUE, got '{s}'"))
}

#[derive(Parser)]
#[command(
    name = "git-cdc-server",
    about = "git-cdc chunk CAS + batch API server"
)]
struct Args {
    /// Chunk storage backend
    #[arg(long, value_enum, default_value = "disk")]
    backend: BackendKind,
    /// Chunk store root directory (disk backend)
    #[arg(long, env = "GIT_CDC_ROOT", required_if_eq("backend", "disk"))]
    root: Option<std::path::PathBuf>,
    /// OpenDAL service scheme (opendal backend), e.g. s3, azblob, azfile,
    /// b2, dropbox, gcs, sftp, ftp, gdrive, swift, webdav, onedrive
    #[arg(
        long,
        env = "GIT_CDC_OPENDAL_SCHEME",
        required_if_eq("backend", "opendal")
    )]
    opendal_scheme: Option<String>,
    /// Service option as KEY=VALUE (repeatable), passed to OpenDAL verbatim,
    /// e.g. --opendal-option container=chunks --opendal-option account_name=me
    #[arg(long = "opendal-option", value_parser = parse_key_value)]
    opendal_options: Vec<(String, String)>,
    /// Directory chunks live under (opendal backend)
    #[arg(long, default_value = "chunks/")]
    opendal_prefix: String,
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
            let Some(root) = args.root else {
                bail!("--root is required for the disk backend")
            };
            Backend::Disk(DiskStore::new(root))
        }
        BackendKind::Opendal => {
            let Some(scheme) = args.opendal_scheme else {
                bail!("--opendal-scheme is required for the opendal backend")
            };
            Backend::Opendal(OpendalStore::connect(&OpendalConfig {
                scheme,
                options: args.opendal_options,
                prefix: args.opendal_prefix,
            })?)
        }
    };
    let state = AppState {
        backend,
        token: args.token,
        grace: Duration::from_secs(args.grace_secs),
        upload_times: Default::default(),
    };
    let listener = tokio::net::TcpListener::bind(&args.listen).await?;
    eprintln!("git-cdc-server listening on {}", listener.local_addr()?);
    axum::serve(listener, app(state)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // PLAN 1.3 done-when: backend/flag pairing is enforced at startup.
    #[test]
    fn disk_backend_requires_root() {
        assert!(Args::try_parse_from(["s", "--backend", "disk", "--token", "t"]).is_err());
        assert!(Args::try_parse_from(["s", "--root", "/tmp/x", "--token", "t"]).is_ok());
        // Defaulted backend skips clap's required_if_eq — main()'s runtime
        // bail is the guard for the bare `--token t` invocation.
        let bare = Args::try_parse_from(["s", "--token", "t"]).unwrap();
        assert!(bare.root.is_none());
    }

    #[test]
    fn opendal_backend_requires_scheme() {
        assert!(Args::try_parse_from(["s", "--backend", "opendal", "--token", "t"]).is_err());
        let ok = Args::try_parse_from([
            "s",
            "--backend",
            "opendal",
            "--opendal-scheme",
            "fs",
            "--opendal-option",
            "root=/tmp/x",
            "--token",
            "t",
        ])
        .unwrap();
        assert_eq!(
            ok.opendal_options,
            vec![("root".to_string(), "/tmp/x".to_string())]
        );
        // Malformed option is rejected at parse time.
        assert!(
            Args::try_parse_from([
                "s",
                "--backend",
                "opendal",
                "--opendal-scheme",
                "fs",
                "--opendal-option",
                "no-equals",
                "--token",
                "t"
            ])
            .is_err()
        );
    }
}
