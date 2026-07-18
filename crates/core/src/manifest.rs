use std::collections::BTreeMap;
use std::fmt::Write as _;

use anyhow::{Context, Result, bail};

use crate::chunker::{Chunk, ChunkParams};

/// Normative manifest spec: docs/spec/manifest.md. LFS-pointer-style discipline:
/// UTF-8, LF only, `{key} {value}` lines, `version` first, remaining header
/// keys sorted, unknown keys preserved. Chunk lines follow the header.
pub const VERSION: &str = "git-cdc/spec/v1";
const VERSION_LINE: &str = "version git-cdc/spec/v1";
const HASH_PREFIX: &str = "blake3:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub oid: blake3::Hash,
    pub size: u64,
    pub chunk_min: u32,
    pub chunk_avg: u32,
    pub chunk_max: u32,
    /// Unknown header keys, preserved verbatim across parse→encode.
    pub extra: BTreeMap<String, String>,
    pub chunks: Vec<Chunk>,
}

/// Cheap sniff used by smudge/push to tell manifests from arbitrary file
/// content (passthrough safety, same idea as git-lfs pointer detection).
pub fn is_manifest(data: &[u8]) -> bool {
    match data.strip_prefix(VERSION_LINE.as_bytes()) {
        Some(rest) => rest.first() == Some(&b'\n'),
        None => false,
    }
}

impl Manifest {
    pub fn new(oid: blake3::Hash, size: u64, chunks: Vec<Chunk>, params: ChunkParams) -> Self {
        Manifest {
            oid,
            size,
            chunk_min: params.min,
            chunk_avg: params.avg,
            chunk_max: params.max,
            extra: BTreeMap::new(),
            chunks,
        }
    }

    /// Byte-stable encoding: version first, header keys sorted, then chunk
    /// lines in file byte order.
    pub fn encode(&self) -> String {
        let mut header: BTreeMap<&str, String> = BTreeMap::new();
        header.insert("chunk-avg", self.chunk_avg.to_string());
        header.insert("chunk-max", self.chunk_max.to_string());
        header.insert("chunk-min", self.chunk_min.to_string());
        header.insert("oid", format!("{HASH_PREFIX}{}", self.oid.to_hex()));
        header.insert("size", self.size.to_string());
        for (k, v) in &self.extra {
            header.insert(k, v.clone());
        }

        let mut out = String::new();
        out.push_str(VERSION_LINE);
        out.push('\n');
        for (k, v) in &header {
            let _ = writeln!(out, "{k} {v}");
        }
        for c in &self.chunks {
            let _ = writeln!(
                out,
                "chunk {HASH_PREFIX}{} {} {}",
                c.hash.to_hex(),
                c.offset,
                c.length
            );
        }
        out
    }

    pub fn parse(data: &[u8]) -> Result<Manifest> {
        if !is_manifest(data) && !data.eq(VERSION_LINE.as_bytes()) {
            bail!("not a git-cdc manifest (missing version line)");
        }
        let text = std::str::from_utf8(data).context("manifest is not UTF-8")?;
        // spec: LF only — lines() would silently strip a CR before each LF.
        if text.contains('\r') {
            bail!("manifest contains a carriage return (LF-only format)");
        }

        let mut header: BTreeMap<String, String> = BTreeMap::new();
        let mut chunks: Vec<Chunk> = Vec::new();

        for line in text.lines().skip(1) {
            if line.is_empty() {
                bail!("empty line in manifest");
            }
            let (key, value) = line
                .split_once(' ')
                .with_context(|| format!("malformed manifest line: {line:?}"))?;
            if key == "chunk" {
                chunks.push(parse_chunk(value)?);
            } else {
                if !chunks.is_empty() {
                    bail!("header key {key:?} after chunk lines");
                }
                if !key
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
                {
                    bail!("invalid manifest key: {key:?}");
                }
                header.insert(key.to_string(), value.to_string());
            }
        }

        let mut take = |k: &str| -> Result<String> {
            header
                .remove(k)
                .with_context(|| format!("manifest missing {k:?}"))
        };
        let oid = parse_hash(&take("oid")?)?;
        let size: u64 = take("size")?.parse().context("bad size")?;
        let chunk_min: u32 = take("chunk-min")?.parse().context("bad chunk-min")?;
        let chunk_avg: u32 = take("chunk-avg")?.parse().context("bad chunk-avg")?;
        let chunk_max: u32 = take("chunk-max")?.parse().context("bad chunk-max")?;

        let chunk_total: u64 = chunks.iter().map(|c| c.length as u64).sum();
        if chunk_total != size {
            bail!("chunk lengths sum to {chunk_total}, size says {size}");
        }

        Ok(Manifest {
            oid,
            size,
            chunk_min,
            chunk_avg,
            chunk_max,
            extra: header,
            chunks,
        })
    }
}

pub fn parse_hash(s: &str) -> Result<blake3::Hash> {
    let hex = s
        .strip_prefix(HASH_PREFIX)
        .with_context(|| format!("oid missing {HASH_PREFIX:?} prefix: {s:?}"))?;
    blake3::Hash::from_hex(hex).with_context(|| format!("bad blake3 hex: {hex:?}"))
}

fn parse_chunk(value: &str) -> Result<Chunk> {
    let mut parts = value.split(' ');
    let (Some(hash), Some(offset), Some(length), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        bail!("malformed chunk line: {value:?}");
    };
    Ok(Chunk {
        hash: parse_hash(hash)?,
        offset: offset.parse().context("bad chunk offset")?,
        length: length.parse().context("bad chunk length")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Manifest {
        let data = b"hello world, this is chunk content";
        let chunk = Chunk {
            hash: blake3::hash(data),
            offset: 0,
            length: data.len() as u32,
        };
        Manifest::new(
            blake3::hash(data),
            data.len() as u64,
            vec![chunk],
            ChunkParams::default(),
        )
    }

    #[test]
    fn round_trip_is_identity_and_byte_stable() {
        let m = sample();
        let text = m.encode();
        let parsed = Manifest::parse(text.as_bytes()).unwrap();
        assert_eq!(parsed, m);
        assert_eq!(parsed.encode(), text);
    }

    #[test]
    fn header_layout_is_strict() {
        let text = sample().encode();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], VERSION_LINE);
        let header: Vec<&str> = lines[1..]
            .iter()
            .take_while(|l| !l.starts_with("chunk "))
            .map(|l| l.split(' ').next().unwrap())
            .collect();
        let mut sorted = header.clone();
        sorted.sort_unstable();
        assert_eq!(header, sorted, "header keys must be sorted");
        assert!(text.ends_with('\n'));
        assert!(!text.contains('\r'));
    }

    #[test]
    fn unknown_keys_survive_rewrite() {
        let mut m = sample();
        m.extra.insert("x-future-field".into(), "some value".into());
        let reparsed = Manifest::parse(m.encode().as_bytes()).unwrap();
        assert_eq!(reparsed.extra.get("x-future-field").unwrap(), "some value");
        assert_eq!(reparsed.encode(), m.encode());
    }

    #[test]
    fn empty_file_manifest_round_trips() {
        let m = Manifest::new(blake3::hash(&[]), 0, vec![], ChunkParams::default());
        assert_eq!(Manifest::parse(m.encode().as_bytes()).unwrap(), m);
    }

    #[test]
    fn rejects_non_manifest_input() {
        assert!(!is_manifest(b"just a regular binary file \x00\x01"));
        assert!(!is_manifest(b"version git-cdc/spec/v1x\n"));
        assert!(Manifest::parse(b"random bytes").is_err());
        // Right version line but header damage:
        let bad = format!("{VERSION_LINE}\nsize notanumber\n");
        assert!(Manifest::parse(bad.as_bytes()).is_err());
        // Size/chunk mismatch:
        let m = sample();
        let tampered = m
            .encode()
            .replace(&format!("size {}", m.size), "size 999999");
        assert!(Manifest::parse(tampered.as_bytes()).is_err());
    }

    #[test]
    fn rejects_carriage_returns_anywhere() {
        let crlf = sample().encode().replace('\n', "\r\n");
        assert!(Manifest::parse(crlf.as_bytes()).is_err());
        // LF version line but one CRLF chunk line — lines() would hide this.
        let mixed = sample().encode().replacen("\nchunk ", "\r\nchunk ", 1);
        assert!(Manifest::parse(mixed.as_bytes()).is_err());
    }

    #[test]
    fn rejects_header_key_after_chunk_lines() {
        let mut text = sample().encode();
        text.push_str("zzz-late-key value\n");
        assert!(Manifest::parse(text.as_bytes()).is_err());
    }

    #[test]
    fn rejects_uppercase_and_underscore_keys() {
        for bad in ["Size 5", "chunk_min 1"] {
            let text = format!("{VERSION_LINE}\n{bad}\n");
            assert!(
                Manifest::parse(text.as_bytes()).is_err(),
                "{bad:?} accepted"
            );
        }
    }

    #[test]
    fn chunker_to_manifest_round_trip_reassembles() {
        let data = crate::chunker::tests::test_data(5 * 1024 * 1024, 3);
        let mut store: std::collections::HashMap<blake3::Hash, Vec<u8>> =
            std::collections::HashMap::new();
        let (chunks, oid, size) =
            crate::chunker::chunk_stream(&data[..], ChunkParams::default(), |c, bytes| {
                store.insert(c.hash, bytes.to_vec());
                Ok(())
            })
            .unwrap();

        let m = Manifest::parse(
            Manifest::new(oid, size, chunks, ChunkParams::default())
                .encode()
                .as_bytes(),
        )
        .unwrap();
        let mut rebuilt = Vec::with_capacity(m.size as usize);
        for c in &m.chunks {
            rebuilt.extend_from_slice(&store[&c.hash]);
        }
        assert_eq!(rebuilt, data);
        assert_eq!(blake3::hash(&rebuilt), m.oid);
    }
}
