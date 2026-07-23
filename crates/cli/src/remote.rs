//! Remote chunk stores (HTTP server, any OpenDAL service, ssh) and the
//! stdio protocol that backs the ssh transport.

use std::fs;
use std::io::{Read, Write};
use std::process::Command as Git;

use anyhow::{Context, Result, bail};
use git_cdc_core::store::{ChunkStore, DiskStore};

use crate::git::git_out;

/// Serve the chunk store over stdin/stdout (RESEARCH protocol v1) — the
/// far end of `ssh <host> git-cdc stdio --root <path>`, the same model as
/// git's own upload-pack.
pub fn cmd_stdio(root: &std::path::Path) -> Result<()> {
    use git_cdc_core::pktline::{PktReader, PktWriter, read_text, write_flush, write_text};

    let store = DiskStore::new(root);
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();

    match read_text(&mut input)?.as_deref() {
        Some("git-cdc-stdio version=1") => {}
        other => bail!("unexpected stdio client greeting: {other:?}"),
    }
    write_text(&mut output, "ok")?;
    output.flush()?;

    loop {
        let line = match read_text(&mut input) {
            Ok(Some(line)) => line,
            Ok(None) => continue, // stray flush
            Err(e)
                if e.downcast_ref::<std::io::Error>()
                    .is_some_and(|io| io.kind() == std::io::ErrorKind::UnexpectedEof) =>
            {
                return Ok(()); // client hung up: session over
            }
            Err(e) => return Err(e),
        };
        let (cmd, arg) = line.split_once(' ').unwrap_or((line.as_str(), ""));
        match cmd {
            "has" => {
                let hash = blake3::Hash::from_hex(arg)?;
                write_text(&mut output, if store.has(&hash) { "yes" } else { "no" })?;
            }
            "put" => {
                let hash = blake3::Hash::from_hex(arg)?;
                let mut encoded = Vec::new();
                PktReader::new(&mut input).read_to_end(&mut encoded)?;
                match store.put_encoded(&hash, &encoded) {
                    Ok(()) => write_text(&mut output, "ok")?,
                    Err(e) => write_text(&mut output, &format!("err {e:#}"))?,
                }
            }
            "get" => {
                let hash = blake3::Hash::from_hex(arg)?;
                match store.get_encoded(&hash) {
                    Ok(encoded) => {
                        write_text(&mut output, "ok")?;
                        PktWriter::new(&mut output).write_all(&encoded)?;
                        write_flush(&mut output)?;
                    }
                    Err(_) => write_text(&mut output, "err not-found")?,
                }
            }
            "list" => {
                for hash in store.list()? {
                    let mtime = fs::metadata(store.path_for(&hash))
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs().to_string())
                        .unwrap_or_else(|| "-".into());
                    write_text(&mut output, &format!("chunk {} {mtime}", hash.to_hex()))?;
                }
                write_flush(&mut output)?;
            }
            "remove" => {
                let hash = blake3::Hash::from_hex(arg)?;
                store.remove(&hash)?;
                write_text(&mut output, "ok")?;
            }
            other => bail!("unknown stdio command {other:?}"),
        }
        output.flush()?;
    }
}

/// Client half of the stdio protocol: a spawned transport process
/// (normally ssh) with pkt-line request/response over its pipes.
pub struct SshRemote {
    child: std::process::Child,
    to: std::io::BufWriter<std::process::ChildStdin>,
    from: std::io::BufReader<std::process::ChildStdout>,
}

impl SshRemote {
    fn connect(argv: &[String]) -> Result<SshRemote> {
        use git_cdc_core::pktline::{read_text, write_text};
        let mut child = Git::new(&argv[0])
            .args(&argv[1..])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .with_context(|| format!("spawning ssh transport {argv:?}"))?;
        let mut to = std::io::BufWriter::new(child.stdin.take().unwrap());
        let mut from = std::io::BufReader::new(child.stdout.take().unwrap());
        write_text(&mut to, "git-cdc-stdio version=1")?;
        to.flush()?;
        match read_text(&mut from)?.as_deref() {
            Some("ok") => Ok(SshRemote { child, to, from }),
            other => bail!(
                "ssh remote did not answer the git-cdc handshake (got {other:?}) — \
                 is git-cdc installed on the remote host?"
            ),
        }
    }

    fn expect_ok(&mut self) -> Result<()> {
        use git_cdc_core::pktline::read_text;
        match read_text(&mut self.from)?.as_deref() {
            Some("ok") => Ok(()),
            Some(err) => bail!("ssh remote: {err}"),
            None => bail!("ssh remote closed the stream"),
        }
    }

    pub fn put_encoded(&mut self, hash: &blake3::Hash, encoded: &[u8]) -> Result<()> {
        use git_cdc_core::pktline::{PktWriter, write_flush, write_text};
        write_text(&mut self.to, &format!("put {}", hash.to_hex()))?;
        PktWriter::new(&mut self.to).write_all(encoded)?;
        write_flush(&mut self.to)?;
        self.expect_ok()
    }

    pub fn get_encoded(&mut self, hash: &blake3::Hash) -> Result<Vec<u8>> {
        use git_cdc_core::pktline::{PktReader, write_text};
        write_text(&mut self.to, &format!("get {}", hash.to_hex()))?;
        self.to.flush()?;
        self.expect_ok()
            .with_context(|| format!("fetching {}", hash.to_hex()))?;
        let mut encoded = Vec::new();
        PktReader::new(&mut self.from).read_to_end(&mut encoded)?;
        Ok(encoded)
    }

    pub fn list(&mut self) -> Result<Vec<(blake3::Hash, Option<std::time::SystemTime>)>> {
        use git_cdc_core::pktline::{read_text, write_text};
        write_text(&mut self.to, "list")?;
        self.to.flush()?;
        let mut out = Vec::new();
        while let Some(line) = read_text(&mut self.from)? {
            let mut parts = line.split(' ');
            let (Some("chunk"), Some(hex), Some(mtime)) =
                (parts.next(), parts.next(), parts.next())
            else {
                bail!("bad list line from ssh remote: {line:?}");
            };
            let modified = mtime
                .parse::<u64>()
                .ok()
                .map(|s| std::time::UNIX_EPOCH + std::time::Duration::from_secs(s));
            out.push((blake3::Hash::from_hex(hex)?, modified));
        }
        Ok(out)
    }

    pub fn remove(&mut self, hash: &blake3::Hash) -> Result<()> {
        use git_cdc_core::pktline::write_text;
        write_text(&mut self.to, &format!("remove {}", hash.to_hex()))?;
        self.to.flush()?;
        self.expect_ok()
    }
}

impl Drop for SshRemote {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Where chunks live remotely: a git-cdc-server (batch API), any OpenDAL
/// service directly (s3, azblob, gcs, ... — IAM/service credentials), or a
/// host reachable over ssh with git-cdc installed.
pub enum Remote {
    Http(git_cdc_core::client::Client),
    Opendal {
        store: git_cdc_core::store::OpendalStore,
        rt: tokio::runtime::Runtime,
    },
    Ssh(SshRemote),
}

/// `cdc.store.option` may be set multiple times (`git config --add`),
/// one `KEY=VALUE` pair each — the same convention as the server's
/// repeatable `--store-option` flag. The underlying mechanism (currently
/// OpenDAL) is an implementation detail, so neither the config keys nor
/// this option shape name it.
fn store_options() -> Result<Vec<(String, String)>> {
    git_out(&["config", "--get-all", "cdc.store.option"])
        .unwrap_or_default()
        .lines()
        .map(|line| {
            line.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .with_context(|| format!("cdc.store.option {line:?} is not KEY=VALUE"))
        })
        .collect()
}

pub fn remote() -> Result<Remote> {
    if let Ok(scheme) = git_out(&["config", "--get", "cdc.store.scheme"]) {
        let config = git_cdc_core::store::OpendalConfig {
            scheme,
            options: store_options()?,
            prefix: git_out(&["config", "--get", "cdc.store.prefix"])
                .unwrap_or_else(|_| "chunks/".into()),
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let store = git_cdc_core::store::OpendalStore::connect(&config)?;
        return Ok(Remote::Opendal { store, rt });
    }
    // cdc.ssh.command (advanced/testing) overrides the ssh invocation with
    // an arbitrary argv, whitespace-split.
    if let Ok(command) = git_out(&["config", "--get", "cdc.ssh.command"]) {
        let argv: Vec<String> = command.split_whitespace().map(String::from).collect();
        if argv.is_empty() {
            bail!("cdc.ssh.command is set but empty");
        }
        return Ok(Remote::Ssh(SshRemote::connect(&argv)?));
    }
    if let Ok(host) = git_out(&["config", "--get", "cdc.ssh.remote"]) {
        let path = git_out(&["config", "--get", "cdc.ssh.path"])
            .context("cdc.ssh.path is not configured (chunk root on the remote host)")?;
        let argv: Vec<String> = ["ssh", &host, "git-cdc", "stdio", "--root", &path]
            .into_iter()
            .map(String::from)
            .collect();
        return Ok(Remote::Ssh(SshRemote::connect(&argv)?));
    }
    let url = git_out(&["config", "--get", "cdc.url"]).context(
        "no remote configured; set cdc.url + cdc.token (server), \
         cdc.store.scheme (serverless — s3, azblob, gcs, ...), \
         or cdc.ssh.remote + cdc.ssh.path (ssh)",
    )?;
    let token = git_out(&["config", "--get", "cdc.token"])
        .context("cdc.token is not configured; set it with `git config cdc.token <token>`")?;
    Ok(Remote::Http(git_cdc_core::client::Client::new(
        &url, &token,
    )))
}
