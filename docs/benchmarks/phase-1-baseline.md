# Phase 1: Baseline Benchmark

**Date**: 2025-01-27
**Commit**: bff6c56

## Dataset

| Metric | Value |
|--------|-------|
| Total Size | 2.9 GB |
| File Count | 2,046 |
| Largest File | 63 MB |

## Results

### Single File (63MB)

| Metric | Value |
|--------|-------|
| Time | 63 ms |
| Throughput | 966 MiB/s |

### All Files (2.9GB, sequential)

| Metric | Value |
|--------|-------|
| Time | 4.7 s |
| Throughput | 628 MiB/s |

### vs ccusage

| Tool | Time | Note |
|------|------|------|
| toktrack | 4.7s | Rust + simd-json, pure parsing |
| ccusage | 40.3s | TypeScript/Node.js, includes LiteLLM fetch |
| **Speedup** | **8.5x** | |

> **Note**: 이 차이는 주로 언어(Rust vs TS/Node.js)와 simd-json 라이브러리에서 기인.
> toktrack은 아직 병렬 처리, mmap 등 추가 최적화 미적용 상태.

## Environment

- **CPU**: Apple M3
- **OS**: macOS Darwin 23.6.0
- **Rust**: 1.93.0
