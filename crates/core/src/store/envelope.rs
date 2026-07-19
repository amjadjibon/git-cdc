//! Stored-object envelope (docs/spec/chunk-storage.md): every chunk is
//! written as a 1-byte tag + payload — `0x00` raw, `0x01` zstd frame —
//! keyed by the *uncompressed* BLAKE3, so identity/dedup/manifests are
//! untouched. The envelope is also the wire format (upload/download).
//!
//! Legacy stores (pre-envelope) hold bare chunk bytes; `decode` detects
//! them by hash: if the whole buffer hashes to the expected oid it IS the
//! chunk. Both interpretations end in verification, so misdetection is
//! impossible.

use anyhow::{Context, Result, bail};

const TAG_RAW: u8 = 0x00;
const TAG_ZSTD: u8 = 0x01;
const LEVEL: i32 = 3;

pub fn encode(raw: &[u8]) -> Vec<u8> {
    let compressed = zstd::bulk::compress(raw, LEVEL).unwrap_or_default();
    // Under ~5% savings the decompress cost forever isn't worth it
    // (already-compressed media lands here).
    if !compressed.is_empty() && (compressed.len() as u128) * 100 < (raw.len() as u128) * 95 {
        let mut out = Vec::with_capacity(compressed.len() + 1);
        out.push(TAG_ZSTD);
        out.extend_from_slice(&compressed);
        out
    } else {
        let mut out = Vec::with_capacity(raw.len() + 1);
        out.push(TAG_RAW);
        out.extend_from_slice(raw);
        out
    }
}

/// Decode + verify: returns the raw chunk bytes or fails loudly. Never
/// returns unverified data.
pub fn decode(bytes: &[u8], expected: &blake3::Hash) -> Result<Vec<u8>> {
    // Legacy (pre-envelope) object: the buffer is the chunk.
    if blake3::hash(bytes) == *expected {
        return Ok(bytes.to_vec());
    }
    let raw = match bytes.split_first() {
        Some((&TAG_RAW, rest)) => rest.to_vec(),
        Some((&TAG_ZSTD, rest)) => {
            // Capacity bound: chunks never exceed the protocol ceiling.
            zstd::bulk::decompress(rest, crate::chunker::CEILING as usize + 1)
                .context("zstd decompress")?
        }
        _ => bail!("chunk {} is corrupt (bad envelope)", expected.to_hex()),
    };
    if blake3::hash(&raw) != *expected {
        bail!("chunk {} is corrupt (hash mismatch)", expected.to_hex());
    }
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compressible_round_trips_smaller() {
        let raw = vec![42u8; 1024 * 1024];
        let hash = blake3::hash(&raw);
        let enc = encode(&raw);
        assert_eq!(enc[0], TAG_ZSTD);
        assert!(
            enc.len() < raw.len() / 10,
            "1 MiB of a repeated byte should crush"
        );
        assert_eq!(decode(&enc, &hash).unwrap(), raw);
    }

    #[test]
    fn incompressible_stays_raw() {
        // xorshift noise doesn't compress.
        let raw = crate::chunker::tests::test_data(256 * 1024, 9);
        let hash = blake3::hash(&raw);
        let enc = encode(&raw);
        assert_eq!(enc[0], TAG_RAW);
        assert_eq!(enc.len(), raw.len() + 1);
        assert_eq!(decode(&enc, &hash).unwrap(), raw);
    }

    #[test]
    fn legacy_bare_bytes_still_decode() {
        let raw = b"a chunk stored before the envelope existed".to_vec();
        assert_eq!(decode(&raw, &blake3::hash(&raw)).unwrap(), raw);
        // Even legacy bytes that LOOK like an envelope: hash wins first.
        let tricky = [&[TAG_ZSTD][..], b"not actually zstd"].concat();
        assert_eq!(decode(&tricky, &blake3::hash(&tricky)).unwrap(), tricky);
    }

    #[test]
    fn corrupt_envelopes_fail_loudly() {
        let raw = vec![7u8; 4096];
        let hash = blake3::hash(&raw);
        let mut enc = encode(&raw);
        enc[10] ^= 0xFF;
        assert!(decode(&enc, &hash).is_err());
        assert!(decode(&[0x02, 1, 2, 3], &hash).is_err(), "unknown tag");
        assert!(decode(&[], &blake3::hash(b"x")).is_err(), "empty buffer");
    }

    #[test]
    fn empty_chunk_round_trips() {
        let hash = blake3::hash(&[]);
        assert_eq!(decode(&encode(&[]), &hash).unwrap(), Vec::<u8>::new());
    }
}
