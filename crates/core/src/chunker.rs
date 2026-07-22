use std::io::Read;

use anyhow::{Result, bail};
use fastcdc::v2020::StreamCDC;

// DESIGN.md §4 default chunking parameters; override per repo with
// `cdc.chunk.{min,avg,max}` git config.
pub const MIN_SIZE: u32 = 512 * 1024;
pub const AVG_SIZE: u32 = 2 * 1024 * 1024;
pub const MAX_SIZE: u32 = 8 * 1024 * 1024;

/// The largest chunk any client may produce (fastcdc's MAXIMUM_MAX) — the
/// server sizes its request-body limit off this, not the client default.
pub const CEILING: u32 = fastcdc::v2020::MAXIMUM_MAX as u32;

/// FastCDC bounds for one chunking run. Manifests record the values used;
/// readers never need them (reassembly is by chunk hash alone).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkParams {
    pub min: u32,
    pub avg: u32,
    pub max: u32,
}

impl Default for ChunkParams {
    fn default() -> Self {
        ChunkParams {
            min: MIN_SIZE,
            avg: AVG_SIZE,
            max: MAX_SIZE,
        }
    }
}

impl ChunkParams {
    /// fastcdc only `debug_assert`s its bounds — a release build silently
    /// chunks wrong on out-of-range values, so enforce them here.
    pub fn validate(self) -> Result<Self> {
        use fastcdc::v2020::{
            AVERAGE_MAX, AVERAGE_MIN, MAXIMUM_MAX, MAXIMUM_MIN, MINIMUM_MAX, MINIMUM_MIN,
        };
        let ranges = [
            ("cdc.chunk.min", self.min as usize, MINIMUM_MIN, MINIMUM_MAX),
            ("cdc.chunk.avg", self.avg as usize, AVERAGE_MIN, AVERAGE_MAX),
            ("cdc.chunk.max", self.max as usize, MAXIMUM_MIN, MAXIMUM_MAX),
        ];
        for (key, value, lo, hi) in ranges {
            if value < lo || value > hi {
                bail!("{key} = {value} is out of range ({lo}..={hi} bytes)");
            }
        }
        if !(self.min <= self.avg && self.avg <= self.max) {
            bail!(
                "chunk sizes must satisfy min <= avg <= max, got {} / {} / {}",
                self.min,
                self.avg,
                self.max
            );
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chunk {
    pub hash: blake3::Hash,
    pub offset: u64,
    pub length: u32,
}

/// Chunk a stream with FastCDC. `sink` is called once per chunk with its raw
/// bytes (store them, upload them — caller's choice), keeping memory bounded
/// by MAX_SIZE instead of file size. Returns the chunk list, the whole-file
/// BLAKE3 oid (computed in the same pass), and the total size.
pub fn chunk_stream<R: Read>(
    reader: R,
    params: ChunkParams,
    mut sink: impl FnMut(&Chunk, &[u8]) -> Result<()>,
) -> Result<(Vec<Chunk>, blake3::Hash, u64)> {
    let params = params.validate()?;
    let mut chunks = Vec::new();
    let mut file_hasher = blake3::Hasher::new();
    let mut size: u64 = 0;

    for entry in StreamCDC::new(
        reader,
        params.min as usize,
        params.avg as usize,
        params.max as usize,
    ) {
        let entry = entry?;
        let chunk = Chunk {
            hash: blake3::hash(&entry.data),
            offset: entry.offset,
            length: entry.length as u32,
        };
        file_hasher.update(&entry.data);
        size += entry.length as u64;
        sink(&chunk, &entry.data)?;
        chunks.push(chunk);
    }

    Ok((chunks, file_hasher.finalize(), size))
}

/// Test/bench helpers. Compiled unconditionally (not cfg(test)) so other
/// crates' test binaries and benches can share them.
pub mod test_util {
    /// Deterministic pseudo-random bytes without a rand dependency.
    pub fn test_data(len: usize, seed: u64) -> Vec<u8> {
        let mut state = seed | 1;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::test_util::test_data;
    use super::*;

    fn chunk_all(data: &[u8]) -> (Vec<Chunk>, blake3::Hash, u64) {
        chunk_stream(data, ChunkParams::default(), |_, _| Ok(())).unwrap()
    }

    #[test]
    fn bounds_and_sizes_respected() {
        let data = test_data(20 * 1024 * 1024, 42);
        let (chunks, oid, size) = chunk_all(&data);

        assert_eq!(size, data.len() as u64);
        assert_eq!(oid, blake3::hash(&data));
        assert_eq!(
            chunks.iter().map(|c| c.length as u64).sum::<u64>(),
            data.len() as u64
        );
        // All but the last chunk respect the min bound; all respect max.
        for c in &chunks[..chunks.len() - 1] {
            assert!(c.length >= MIN_SIZE, "chunk below min: {}", c.length);
        }
        for c in &chunks {
            assert!(c.length <= MAX_SIZE, "chunk above max: {}", c.length);
        }
        // Offsets are contiguous.
        let mut expect = 0u64;
        for c in &chunks {
            assert_eq!(c.offset, expect);
            expect += c.length as u64;
        }
    }

    #[test]
    fn empty_file_yields_no_chunks() {
        let (chunks, oid, size) = chunk_all(&[]);
        assert!(chunks.is_empty());
        assert_eq!(size, 0);
        assert_eq!(oid, blake3::hash(&[]));
    }

    #[test]
    fn file_exactly_max_chunk_size_round_trips() {
        let data = test_data(MAX_SIZE as usize, 21);
        let (chunks, oid, size) = chunk_all(&data);
        assert_eq!(size, MAX_SIZE as u64);
        assert_eq!(oid, blake3::hash(&data));
        assert_eq!(
            chunks.iter().map(|c| c.length as u64).sum::<u64>(),
            MAX_SIZE as u64
        );
    }

    #[test]
    fn file_smaller_than_min_is_one_chunk() {
        let data = test_data(1000, 7);
        let (chunks, _, size) = chunk_all(&data);
        assert_eq!(chunks.len(), 1);
        assert_eq!(size, 1000);
        assert_eq!(chunks[0].hash, blake3::hash(&data));
    }

    #[test]
    fn sink_sees_exact_chunk_bytes() {
        let data = test_data(3 * 1024 * 1024, 9);
        let mut rebuilt = Vec::new();
        let (chunks, _, _) = chunk_stream(&data[..], ChunkParams::default(), |c, bytes| {
            assert_eq!(blake3::hash(bytes), c.hash);
            assert_eq!(bytes.len() as u32, c.length);
            rebuilt.extend_from_slice(bytes);
            Ok(())
        })
        .unwrap();
        assert!(!chunks.is_empty());
        assert_eq!(rebuilt, data);
    }

    #[test]
    fn params_validation() {
        assert!(ChunkParams::default().validate().is_ok());
        // fastcdc hard-bound edges are accepted…
        assert!(
            ChunkParams {
                min: 64,
                avg: 256,
                max: 1024
            }
            .validate()
            .is_ok()
        );
        assert!(
            ChunkParams {
                min: 1 << 20,
                avg: 4 << 20,
                max: 16 << 20
            }
            .validate()
            .is_ok()
        );
        // …out-of-range values are rejected, naming the config key:
        let err = ChunkParams {
            min: 63,
            ..Default::default()
        }
        .validate()
        .unwrap_err();
        assert!(err.to_string().contains("cdc.chunk.min"), "{err}");
        let err = ChunkParams {
            max: (16 << 20) + 1,
            ..Default::default()
        }
        .validate()
        .unwrap_err();
        assert!(err.to_string().contains("cdc.chunk.max"), "{err}");
        // …and misordered sizes too:
        let err = ChunkParams {
            min: 1 << 20,
            avg: 512 * 1024,
            max: 8 << 20,
        }
        .validate()
        .unwrap_err();
        assert!(err.to_string().contains("min <= avg <= max"), "{err}");
    }

    #[test]
    fn custom_params_change_chunking() {
        let data = test_data(4 * 1024 * 1024, 5);
        let small = ChunkParams {
            min: 64 * 1024,
            avg: 256 * 1024,
            max: 1024 * 1024,
        };
        let (chunks, oid, _) = chunk_stream(&data[..], small, |_, _| Ok(())).unwrap();
        let (default_chunks, default_oid, _) = chunk_all(&data);
        assert!(
            chunks.len() > default_chunks.len(),
            "smaller bounds → more chunks"
        );
        for c in &chunks {
            assert!(
                c.length <= small.max,
                "chunk above configured max: {}",
                c.length
            );
        }
        assert_eq!(oid, default_oid, "oid is chunking-independent");
    }

    #[test]
    fn small_edit_changes_few_chunks() {
        let mut data = test_data(20 * 1024 * 1024, 1);
        let (before, _, _) = chunk_all(&data);
        data[10 * 1024 * 1024] ^= 0xFF; // 1-byte edit in the middle
        let (after, _, _) = chunk_all(&data);

        let before_set: std::collections::HashSet<_> = before.iter().map(|c| c.hash).collect();
        let changed = after
            .iter()
            .filter(|c| !before_set.contains(&c.hash))
            .count();
        assert!(changed <= 2, "1-byte edit changed {changed} chunks");
    }
}
