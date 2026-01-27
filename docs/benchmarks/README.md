# Benchmark Results

Performance benchmarks for toktrack's JSONL parser.

## Latest (Phase 1 - simd-json baseline)

| Metric | toktrack | ccusage | Speedup |
|--------|----------|---------|---------|
| Time (2.9GB) | 4.7s | 40.3s | **8.5x** |
| Throughput | 628 MiB/s | ~73 MiB/s | - |

> Rust + simd-json vs TypeScript/Node.js. 병렬 처리 등 추가 최적화 미적용.

## Dataset

- **Size**: 2.9 GB
- **Files**: 2,046 JSONL files
- **Location**: `~/.claude/projects/`

## Phase History

- [Phase 1: Baseline](./phase-1-baseline.md) - simd-json sequential parsing
