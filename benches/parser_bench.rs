//! Criterion benchmarks for ClaudeCodeParser

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::path::{Path, PathBuf};
use toktrack::parsers::{CLIParser, ClaudeCodeParser};

/// Find the largest JSONL file in a directory recursively
fn find_largest_jsonl(dir: &Path) -> Option<PathBuf> {
    let pattern = dir.join("**/*.jsonl");
    let pattern_str = pattern.to_string_lossy();

    glob::glob(&pattern_str)
        .ok()?
        .filter_map(|entry| entry.ok())
        .filter(|path| path.is_file())
        .max_by_key(|path| std::fs::metadata(path).map(|m| m.len()).unwrap_or(0))
}

/// Get test file: prefer real Claude data, fallback to fixture
fn get_bench_file(parser: &ClaudeCodeParser) -> PathBuf {
    let real_data_dir = parser.data_dir();

    if real_data_dir.exists() {
        if let Some(largest) = find_largest_jsonl(&real_data_dir) {
            let size = std::fs::metadata(&largest).map(|m| m.len()).unwrap_or(0);
            if size > 0 {
                eprintln!(
                    "Using real Claude data: {} ({} bytes)",
                    largest.display(),
                    size
                );
                return largest;
            }
        }
    }

    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("claude-sample.jsonl");

    eprintln!("Using fixture: {} (real data not found)", fixture.display());
    fixture
}

fn bench_parse_file(c: &mut Criterion) {
    let parser = ClaudeCodeParser::new();
    let test_file = get_bench_file(&parser);

    let file_size = std::fs::metadata(&test_file).map(|m| m.len()).unwrap_or(0);

    if file_size == 0 {
        eprintln!("Warning: benchmark file is empty or not found");
        return;
    }

    let mut group = c.benchmark_group("parser");
    group.throughput(Throughput::Bytes(file_size));

    group.bench_with_input(
        BenchmarkId::new("parse_file", format!("{} bytes", file_size)),
        &test_file,
        |b, path| {
            b.iter(|| parser.parse_file(black_box(path)));
        },
    );

    group.finish();
}

fn bench_parse_line(c: &mut Criterion) {
    // Single line parsing benchmark
    let sample_line = br#"{"timestamp":"2025-01-26T10:00:00Z","requestId":"req-001","message":{"model":"claude-sonnet-4-20250514","id":"msg-001","usage":{"input_tokens":100,"output_tokens":50,"cache_creation_input_tokens":10,"cache_read_input_tokens":20}}}"#;

    let mut group = c.benchmark_group("parser");
    group.throughput(Throughput::Bytes(sample_line.len() as u64));

    group.bench_function("parse_line", |b| {
        b.iter(|| {
            let mut line_copy = sample_line.to_vec();
            let _: Result<serde_json::Value, _> = simd_json::from_slice(black_box(&mut line_copy));
        });
    });

    group.finish();
}

criterion_group!(benches, bench_parse_file, bench_parse_line);
criterion_main!(benches);
