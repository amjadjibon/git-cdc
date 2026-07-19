//! Hot-path benchmarks. Run: `cargo bench -p git-cdc-core`.
//! Baselines live in docs/benchmarks/RESULTS.md — update them when a
//! change moves a number on purpose.

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;

use git_cdc_core::chunker::{ChunkParams, chunk_stream};
use git_cdc_core::manifest::Manifest;
use git_cdc_core::pktline::{PktReader, PktWriter};
use git_cdc_core::store::envelope;
use git_cdc_core::store::{ChunkStore, DiskStore};

/// Deterministic pseudo-random bytes (same xorshift as the test suites).
fn test_data(len: usize, seed: u64) -> Vec<u8> {
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

fn compressible_data(len: usize) -> Vec<u8> {
    b"the same line of text over and over again and again\n"
        .iter()
        .cycle()
        .take(len)
        .copied()
        .collect()
}

fn bench_chunker(c: &mut Criterion) {
    const SIZE: usize = 32 * 1024 * 1024;
    let data = test_data(SIZE, 42);
    let mut g = c.benchmark_group("chunker");
    g.throughput(Throughput::Bytes(SIZE as u64));
    g.sample_size(20);

    g.bench_function("default_params_32MiB", |b| {
        b.iter(|| {
            let (chunks, oid, _) =
                chunk_stream(black_box(&data[..]), ChunkParams::default(), |_, _| Ok(())).unwrap();
            black_box((chunks, oid))
        })
    });
    let small = ChunkParams {
        min: 64 * 1024,
        avg: 256 * 1024,
        max: 1024 * 1024,
    };
    g.bench_function("small_params_32MiB", |b| {
        b.iter(|| {
            let (chunks, oid, _) =
                chunk_stream(black_box(&data[..]), small, |_, _| Ok(())).unwrap();
            black_box((chunks, oid))
        })
    });
    g.finish();
}

fn bench_envelope(c: &mut Criterion) {
    const SIZE: usize = 2 * 1024 * 1024;
    let noise = test_data(SIZE, 7);
    let text = compressible_data(SIZE);
    let noise_hash = blake3::hash(&noise);
    let text_hash = blake3::hash(&text);
    let noise_enc = envelope::encode(&noise);
    let text_enc = envelope::encode(&text);

    let mut g = c.benchmark_group("envelope");
    g.throughput(Throughput::Bytes(SIZE as u64));

    g.bench_function("encode_incompressible_2MiB", |b| {
        b.iter(|| black_box(envelope::encode(black_box(&noise))))
    });
    g.bench_function("encode_compressible_2MiB", |b| {
        b.iter(|| black_box(envelope::encode(black_box(&text))))
    });
    g.bench_function("decode_raw_2MiB", |b| {
        b.iter(|| black_box(envelope::decode(black_box(&noise_enc), &noise_hash).unwrap()))
    });
    g.bench_function("decode_zstd_2MiB", |b| {
        b.iter(|| black_box(envelope::decode(black_box(&text_enc), &text_hash).unwrap()))
    });
    g.finish();
}

fn bench_manifest(c: &mut Criterion) {
    // A 1000-chunk manifest ≈ a 2 GB file at default params.
    let chunks: Vec<git_cdc_core::chunker::Chunk> = (0..1000u64)
        .map(|i| git_cdc_core::chunker::Chunk {
            hash: blake3::hash(&i.to_le_bytes()),
            offset: i * 2_097_152,
            length: 2_097_152,
        })
        .collect();
    let m = Manifest::new(
        blake3::hash(b"whole file"),
        1000 * 2_097_152,
        chunks,
        ChunkParams::default(),
    );
    let text = m.encode();

    let mut g = c.benchmark_group("manifest");
    g.bench_function("encode_1000_chunks", |b| {
        b.iter(|| black_box(black_box(&m).encode()))
    });
    g.bench_function("parse_1000_chunks", |b| {
        b.iter(|| black_box(Manifest::parse(black_box(text.as_bytes())).unwrap()))
    });
    g.finish();
}

fn bench_pktline(c: &mut Criterion) {
    const SIZE: usize = 8 * 1024 * 1024;
    let data = test_data(SIZE, 3);
    let mut wire = Vec::with_capacity(SIZE + SIZE / 4096);
    {
        use std::io::Write;
        PktWriter::new(&mut wire).write_all(&data).unwrap();
        git_cdc_core::pktline::write_flush(&mut wire).unwrap();
    }

    let mut g = c.benchmark_group("pktline");
    g.throughput(Throughput::Bytes(SIZE as u64));
    g.bench_function("write_8MiB", |b| {
        b.iter(|| {
            use std::io::Write;
            let mut out = Vec::with_capacity(wire.len());
            PktWriter::new(&mut out)
                .write_all(black_box(&data))
                .unwrap();
            black_box(out)
        })
    });
    g.bench_function("read_8MiB", |b| {
        b.iter(|| {
            use std::io::Read;
            let mut r = black_box(&wire[..]);
            let mut out = Vec::with_capacity(SIZE);
            PktReader::new(&mut r).read_to_end(&mut out).unwrap();
            black_box(out)
        })
    });
    g.finish();
}

fn bench_disk_store(c: &mut Criterion) {
    const SIZE: usize = 2 * 1024 * 1024;
    let data = test_data(SIZE, 11);
    let hash = blake3::hash(&data);
    let dir = tempfile::tempdir().unwrap();
    let store = DiskStore::new(dir.path().join("objects"));
    store.put(&hash, &data).unwrap();

    let mut g = c.benchmark_group("disk_store");
    g.throughput(Throughput::Bytes(SIZE as u64));
    g.sample_size(30);
    g.bench_function("put_2MiB_new", |b| {
        // A fresh hash each iteration so put always takes the write path.
        let mut i = 0u64;
        b.iter(|| {
            i += 1;
            let mut d = data.clone();
            d[..8].copy_from_slice(&i.to_le_bytes());
            store.put(&blake3::hash(&d), &d).unwrap();
        })
    });
    g.bench_function("get_2MiB", |b| {
        b.iter(|| black_box(store.get(black_box(&hash)).unwrap()))
    });
    g.finish();
}

criterion_group!(
    benches,
    bench_chunker,
    bench_envelope,
    bench_manifest,
    bench_pktline,
    bench_disk_store
);
criterion_main!(benches);
