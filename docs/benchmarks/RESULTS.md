# Benchmark baselines

`cargo bench -p git-cdc-core` (criterion 0.8), release profile.
Recorded 2026-07-19 on Apple Silicon (macOS, arm64); treat as
order-of-magnitude baselines, not cross-machine promises. Update when a
change moves a number on purpose.

| Benchmark | Median | Throughput |
| --------- | ------ | ---------- |
| chunker/default_params_32MiB | 42.1 ms | 760 MiB/s |
| chunker/small_params_32MiB | 42.1 ms | 761 MiB/s |
| envelope/encode_incompressible_2MiB | 404 µs | 4.8 GiB/s |
| envelope/encode_compressible_2MiB | 192 µs | 10.2 GiB/s |
| envelope/decode_raw_2MiB | 1.83 ms | 1.07 GiB/s |
| envelope/decode_zstd_2MiB | 1.16 ms | 1.69 GiB/s |
| manifest/encode_1000_chunks | 78 µs | — |
| manifest/parse_1000_chunks | 116 µs | — |
| pktline/write_8MiB | 115 µs | 68 GiB/s |
| pktline/read_8MiB | 665 µs | 11.7 GiB/s |
| disk_store/put_2MiB_new | 2.51 ms | 796 MiB/s |
| disk_store/get_2MiB | 1.98 ms | 1.01 GiB/s |

## Reading the numbers

- **Chunking runs at ~760 MiB/s and is parameter-insensitive** — the gear
  hash dominates, not boundary bookkeeping. A 1 GB `git add` spends ~1.3 s
  chunking; BLAKE3 and FastCDC share the same streaming pass.
- **The envelope is nowhere near a bottleneck**: encode of incompressible
  data (the worst realistic case — one wasted zstd attempt) still moves
  ~4.8 GiB/s; decode costs are dominated by the BLAKE3 verification, which
  is the safety property, not overhead to remove. Decoding compressed text
  is *faster* than raw because hashing 2 MiB dominates and the zstd frame
  is small.
- **pkt-line framing is effectively free** (68 GiB/s write, 11.7 GiB/s
  read) — the filter-process protocol adds no measurable cost over the
  one-shot filters.
- **Manifest codec is µs-scale** even for 1000-chunk (≈2 GB) files.
- **Disk store round-trips at ~0.8–1 GiB/s** for 2 MiB chunks including
  hash verification and atomic-rename hygiene — i.e. local operations are
  disk/hash-bound, exactly where they should be.

Conclusion: end-to-end performance is network-bound in practice; nothing
in the core pipeline is worth optimizing ahead of transfer parallelism.
