//! Criterion benchmarks for ClaudeCodeParser

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;
use std::path::{Path, PathBuf};
use toktrack::parsers::{CLIParser, ClaudeCodeParser};

/// Find all JSONL files in a directory recursively
fn find_all_jsonl(dir: &Path) -> Vec<PathBuf> {
    let pattern = dir.join("**/*.jsonl");
    let pattern_str = pattern.to_string_lossy();

    glob::glob(&pattern_str)
        .map(|paths| {
            paths
                .filter_map(|entry| entry.ok())
                .filter(|path| path.is_file())
                .collect()
        })
        .unwrap_or_default()
}

/// Find the largest JSONL file in a directory recursively
fn find_largest_jsonl(dir: &Path) -> Option<PathBuf> {
    find_all_jsonl(dir)
        .into_iter()
        .max_by_key(|path| std::fs::metadata(path).map(|m| m.len()).unwrap_or(0))
}

/// Get test file: prefer real Claude data, fallback to fixture
fn get_bench_file(parser: &ClaudeCodeParser) -> PathBuf {
    let real_data_dir = parser.data_dir();

    if real_data_dir.exists() {
        if let Some(largest) = find_largest_jsonl(real_data_dir) {
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

fn bench_parse_all_files(c: &mut Criterion) {
    let parser = ClaudeCodeParser::new();
    let data_dir = parser.data_dir();

    if !data_dir.exists() {
        eprintln!("Skipping parse_all_files: no real Claude data found");
        return;
    }

    let files = find_all_jsonl(data_dir);
    if files.is_empty() {
        eprintln!("Skipping parse_all_files: no JSONL files found");
        return;
    }

    let total_size: u64 = files
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .sum();

    eprintln!(
        "Benchmarking {} files, total {} bytes ({:.2} GB)",
        files.len(),
        total_size,
        total_size as f64 / 1_073_741_824.0
    );

    let mut group = c.benchmark_group("parser");
    group.throughput(Throughput::Bytes(total_size));
    group.sample_size(10); // 3GB는 시간이 오래 걸리므로 샘플 수 줄임

    group.bench_function("parse_all_files_sequential", |b| {
        b.iter(|| {
            for file in &files {
                let _ = parser.parse_file(black_box(file));
            }
        });
    });

    group.bench_function("parse_all_files_parallel", |b| {
        b.iter(|| {
            let _ = parser.parse_all();
        });
    });

    group.finish();
}

fn bench_parse_recent_files(c: &mut Criterion) {
    let parser = ClaudeCodeParser::new();
    let data_dir = parser.data_dir();

    if !data_dir.exists() {
        eprintln!("Skipping parse_recent_files: no real Claude data found");
        return;
    }

    let all_files = find_all_jsonl(data_dir);
    if all_files.is_empty() {
        eprintln!("Skipping parse_recent_files: no JSONL files found");
        return;
    }

    let since_24h = std::time::SystemTime::now() - std::time::Duration::from_secs(24 * 3600);

    // Count recent files and their sizes for reporting
    let recent_files: Vec<&PathBuf> = all_files
        .iter()
        .filter(|f| {
            f.metadata()
                .and_then(|m| m.modified())
                .map(|mtime| mtime >= since_24h)
                .unwrap_or(true)
        })
        .collect();

    let recent_size: u64 = recent_files
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .sum();

    let total_size: u64 = all_files
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .sum();

    eprintln!(
        "parse_recent_files: {}/{} files, {:.2} MB / {:.2} GB total",
        recent_files.len(),
        all_files.len(),
        recent_size as f64 / 1_048_576.0,
        total_size as f64 / 1_073_741_824.0
    );

    let mut group = c.benchmark_group("parser");
    group.sample_size(10);

    group.bench_function("parse_all (cold path)", |b| {
        b.iter(|| {
            let _ = parser.parse_all();
        });
    });

    group.bench_function("parse_recent_files_24h (warm path)", |b| {
        b.iter(|| {
            let _ = parser.parse_recent_files(black_box(since_24h));
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_parse_file,
    bench_parse_line,
    bench_parse_all_files,
    bench_parse_recent_files
);
criterion_main!(benches);
