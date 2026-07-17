use std::io::Read;

use anyhow::Result;
use fastcdc::v2020::StreamCDC;

// DESIGN.md §4 chunking parameters. ponytail: constants, not config — plumb
// through git config when a repo actually needs different sizes.
pub const MIN_SIZE: u32 = 512 * 1024;
pub const AVG_SIZE: u32 = 2 * 1024 * 1024;
pub const MAX_SIZE: u32 = 8 * 1024 * 1024;

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
    mut sink: impl FnMut(&Chunk, &[u8]) -> Result<()>,
) -> Result<(Vec<Chunk>, blake3::Hash, u64)> {
    let mut chunks = Vec::new();
    let mut file_hasher = blake3::Hasher::new();
    let mut size: u64 = 0;

    for entry in StreamCDC::new(reader, MIN_SIZE as usize, AVG_SIZE as usize, MAX_SIZE as usize) {
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    // Deterministic pseudo-random bytes without a rand dependency.
    pub(crate) fn test_data(len: usize, seed: u64) -> Vec<u8> {
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

    fn chunk_all(data: &[u8]) -> (Vec<Chunk>, blake3::Hash, u64) {
        chunk_stream(data, |_, _| Ok(())).unwrap()
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
        let (chunks, _, _) = chunk_stream(&data[..], |c, bytes| {
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
    fn small_edit_changes_few_chunks() {
        let mut data = test_data(20 * 1024 * 1024, 1);
        let (before, _, _) = chunk_all(&data);
        data[10 * 1024 * 1024] ^= 0xFF; // 1-byte edit in the middle
        let (after, _, _) = chunk_all(&data);

        let before_set: std::collections::HashSet<_> =
            before.iter().map(|c| c.hash).collect();
        let changed = after.iter().filter(|c| !before_set.contains(&c.hash)).count();
        assert!(changed <= 2, "1-byte edit changed {changed} chunks");
    }
}
