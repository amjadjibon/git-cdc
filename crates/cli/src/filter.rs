//! The clean/smudge filters: one-shot commands and the long-running
//! filter-process protocol.

use std::io::{Read, Write};

use anyhow::{Result, bail};
use git_cdc_core::chunker::{ChunkParams, chunk_stream};
use git_cdc_core::manifest::{Manifest, is_manifest};
use git_cdc_core::store::{ChunkStore, DiskStore};

use crate::git::{chunk_params, local_store};

/// Clean: file content in, manifest out. Shared by the one-shot filter and
/// the filter-process loop.
///
/// Peeks far enough to recognize a manifest: re-cleaning passed-through
/// manifest text (fresh clone worktree state) must not chunk the manifest
/// itself — it passes through unchanged, same as git-lfs does for pointers.
fn clean_stream(
    mut reader: impl Read,
    mut writer: impl Write,
    store: &DiskStore,
    params: ChunkParams,
) -> Result<()> {
    let mut head = Vec::with_capacity(64);
    (&mut reader).take(64).read_to_end(&mut head)?;
    if is_manifest(&head) {
        writer.write_all(&head)?;
        std::io::copy(&mut reader, &mut writer)?;
        return Ok(());
    }
    let input = head.as_slice().chain(reader);
    let (chunks, oid, size) = chunk_stream(input, params, |c, bytes| store.put(&c.hash, bytes))?;
    writer.write_all(Manifest::new(oid, size, chunks, params).encode().as_bytes())?;
    Ok(())
}

/// Smudge: manifest in, file content out (shared like `clean_stream`).
/// The passthrough case (file committed before tracking) can be arbitrarily
/// large and must stream, not buffer — hence the same 64-byte peek.
fn smudge_stream(mut reader: impl Read, mut writer: impl Write, store: &DiskStore) -> Result<()> {
    let mut input = Vec::with_capacity(64);
    (&mut reader).take(64).read_to_end(&mut input)?;
    if !is_manifest(&input) {
        writer.write_all(&input)?;
        std::io::copy(&mut reader, &mut writer)?;
        return Ok(());
    }
    reader.read_to_end(&mut input)?; // manifests are small
    let m = Manifest::parse(&input)?;

    if m.chunks.iter().any(|c| !store.has(&c.hash)) {
        // Fresh clone / chunks not fetched yet: write the manifest through
        // so checkout succeeds, and tell the user how to materialize.
        writer.write_all(&input)?;
        eprintln!("git-cdc: chunks not in local store; run `git cdc pull` to fetch file content");
        return Ok(());
    }

    let mut hasher = blake3::Hasher::new();
    for c in &m.chunks {
        let data = store.get(&c.hash)?; // hard-errors on corrupt chunk
        hasher.update(&data);
        writer.write_all(&data)?;
    }
    if hasher.finalize() != m.oid {
        bail!("reassembled file does not match manifest oid — refusing to emit corrupt data");
    }
    Ok(())
}

pub fn cmd_clean() -> Result<()> {
    let stdin = std::io::stdin().lock();
    let stdout = std::io::stdout().lock();
    clean_stream(stdin, stdout, &local_store()?, chunk_params()?)
}

pub fn cmd_smudge() -> Result<()> {
    let stdin = std::io::stdin().lock();
    let stdout = std::io::stdout().lock();
    smudge_stream(stdin, stdout, &local_store()?)
}

/// Long-running filter (gitattributes(5) filter-process protocol, v2): one
/// process per git operation instead of one per file; store and chunk
/// params are opened once.
pub fn cmd_filter_process() -> Result<()> {
    use git_cdc_core::pktline::{PktReader, PktWriter, read_text, write_flush, write_text};

    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();

    // Handshake: welcome + version, then capability negotiation.
    match read_text(&mut input)?.as_deref() {
        Some("git-filter-client") => {}
        other => bail!("not a git filter client (got {other:?})"),
    }
    let mut versions = Vec::new();
    while let Some(line) = read_text(&mut input)? {
        versions.push(line);
    }
    if !versions.iter().any(|v| v == "version=2") {
        bail!("no common filter protocol version (client sent {versions:?})");
    }
    write_text(&mut output, "git-filter-server")?;
    write_text(&mut output, "version=2")?;
    write_flush(&mut output)?;

    while read_text(&mut input)?.is_some() {} // client capability list
    write_text(&mut output, "capability=clean")?;
    write_text(&mut output, "capability=smudge")?;
    write_flush(&mut output)?;

    let store = local_store()?;
    let params = chunk_params()?;

    // Per-file loop: keys, flush, content packets, flush.
    loop {
        let mut command = String::new();
        let mut pathname = String::new();
        // Git signals it's done by closing stdin between files.
        let first = match read_text(&mut input) {
            Ok(line) => line,
            Err(e)
                if e.downcast_ref::<std::io::Error>()
                    .is_some_and(|io| io.kind() == std::io::ErrorKind::UnexpectedEof) =>
            {
                return Ok(());
            }
            Err(e) => return Err(e),
        };
        let mut next = first;
        while let Some(line) = next {
            match line.split_once('=') {
                Some(("command", v)) => command = v.to_string(),
                Some(("pathname", v)) => pathname = v.to_string(),
                _ => {} // unknown keys are fine per protocol
            }
            next = read_text(&mut input)?;
        }
        if command.is_empty() {
            bail!("filter request without a command");
        }

        // Buffer the result so a mid-stream failure can still become a
        // clean per-file status=error instead of a truncated success.
        let mut content = PktReader::new(&mut input);
        let mut result = Vec::new();
        let outcome = match command.as_str() {
            "clean" => clean_stream(&mut content, &mut result, &store, params),
            "smudge" => smudge_stream(&mut content, &mut result, &store),
            other => Err(anyhow::anyhow!("unsupported filter command {other:?}")),
        };
        content.drain()?;

        match outcome {
            Ok(()) => {
                write_text(&mut output, "status=success")?;
                write_flush(&mut output)?;
                PktWriter::new(&mut output).write_all(&result)?;
                write_flush(&mut output)?;
                write_flush(&mut output)?; // empty list: status unchanged
            }
            Err(e) => {
                eprintln!("git-cdc: {pathname}: {e:#}");
                write_text(&mut output, "status=error")?;
                write_flush(&mut output)?;
            }
        }
    }
}
