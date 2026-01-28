# toktrack 태스크 분해

## Phase 0: 프로젝트 셋업 ✅

### Task 0.1: 기본 구조 생성 ✅
- [x] Cargo.toml 초기화
- [x] 디렉토리 구조 생성
- [x] .gitignore 설정
- [x] CLAUDE.md 작성
- [x] ai-context JSON 파일 작성

### Task 0.2: 개발 환경 설정 ✅
- [x] GitHub 저장소 생성
- [x] CI/CD 워크플로우 (lint, test, build)
- [x] release-please 자동 릴리즈 설정
- [x] 크로스 컴파일 설정 (release.yml)

### Task 0.3: 의존성 추가 ✅
```toml
[dependencies]
simd-json = "0.14"
ratatui = "0.29"
crossterm = "0.28"
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
chrono = "0.4"
directories = "5"
rayon = "1.10"

[dev-dependencies]
insta = "1.41"
criterion = "0.5"
```

---

## Phase 1: 코어 파싱 (TDD) - In Progress

### Task 1.1: 타입 정의 ✅
```rust
// src/types/mod.rs

pub struct UsageEntry {
    pub timestamp: DateTime<Utc>,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cost_usd: Option<f64>,
}

pub struct DailySummary {
    pub date: NaiveDate,
    pub total_input: u64,
    pub total_output: u64,
    pub total_cost: f64,
    pub models: HashMap<String, ModelUsage>,
}
```

**테스트 먼저:**
```rust
#[test]
fn test_usage_entry_from_json() {
    let json = r#"{"timestamp": "2025-01-10T10:00:00Z", ...}"#;
    let entry: UsageEntry = parse_entry(json).unwrap();
    assert_eq!(entry.input_tokens, 100);
}
```

### Task 1.2: CLIParser trait 정의 ✅
```rust
// src/parsers/mod.rs

pub trait CLIParser: Send + Sync {
    fn name(&self) -> &str;
    fn data_dir(&self) -> PathBuf;
    fn file_pattern(&self) -> &str;
    fn parse_file(&self, path: &Path) -> Result<Vec<UsageEntry>>;
}
```

**테스트 먼저:**
```rust
#[test]
fn test_cli_parser_trait_object() {
    let parser: Box<dyn CLIParser> = Box::new(ClaudeCodeParser::new());
    assert_eq!(parser.name(), "claude-code");
}
```

### Task 1.3: Claude Code 파서 구현 ✅
```rust
// src/parsers/claude.rs

pub struct ClaudeCodeParser {
    data_dir: PathBuf,
}

impl CLIParser for ClaudeCodeParser {
    fn name(&self) -> &str { "claude-code" }
    fn data_dir(&self) -> PathBuf { ... }
    fn file_pattern(&self) -> &str { "**/*.jsonl" }
    fn parse_file(&self, path: &Path) -> Result<Vec<UsageEntry>> {
        // simd-json으로 JSONL 파싱
    }
}
```

**스냅샷 테스트:**
```rust
#[test]
fn test_parse_claude_jsonl() {
    let entries = parse_test_file("fixtures/claude-sample.jsonl");
    insta::assert_debug_snapshot!(entries);
}
```

### Task 1.4: Criterion 벤치마크 ✅
- [x] criterion 벤치마크 구현 (benches/parser_bench.rs)
- [x] lib.rs 생성 (public exports)
- [x] 실제 Claude 데이터 자동 탐지 + fixture fallback
- [x] Throughput 측정 (MB/s)
- **Baseline**: ~950 MiB/s (63MB 파일, simd-json 기반)

---

## Phase 2: 서비스 레이어 (TDD) - In Progress

### Task 2.1: 집계 서비스 ✅
- [x] `src/services/mod.rs` 생성
- [x] `src/services/aggregator.rs` 구현
- [x] `Aggregator::daily()` - 일별 집계 (날짜 오름차순)
- [x] `Aggregator::by_model()` - 모델별 집계 (None → "unknown")
- [x] `Aggregator::total()` - 전체 합계
- [x] TDD 테스트 12개 통과

### Task 2.2: 비용 계산 서비스 ✅
- [x] `src/services/pricing.rs` 생성
- [x] `ModelPricing`, `PricingCache` 타입 정의
- [x] `PricingService::get_or_calculate_cost()` - auto 모드 (cost_usd 우선)
- [x] `PricingService::calculate_cost()` - 토큰 기반 계산
- [x] LiteLLM 가격 데이터 캐싱 (1h TTL, ~/.toktrack/pricing.json)
- [x] TDD 테스트 13개 통과

### Task 2.3: DailySummary 캐싱 서비스 ✅
- [x] `src/services/cache.rs` 생성
- [x] `DailySummaryCache` 타입 정의 (cli, updated_at, summaries)
- [x] `DailySummaryCacheService::load_or_compute()` - 캐시 로드 + 미싱 날짜 계산 + 병합
- [x] `DailySummaryCacheService::clear()` - 캐시 삭제
- [x] 캐시 경로: `~/.toktrack/cache/{cli}_daily.json`
- [x] 핵심 로직: 오늘 이전 = 캐시, 오늘 = 항상 재계산
- [x] TDD 테스트 10개 통과

### Task 2.4: PricingService 통합 ✅
- [x] `app.rs`에 `PricingService` import
- [x] `load_data()`에서 pricing 계산 로직 추가
- [x] 네트워크 실패 시 graceful fallback (`PricingService::new().ok()`)

---

## Phase 3: SIMD 최적화 ✅

### Task 3.1: Zero-copy serde 최적화 ✅
- [x] serde 구조체를 borrowed (`&'a str`) 타입으로 변경
- [x] `parse_file`에서 in-place mutable buffer 사용
- [x] `line.to_vec()` 할당 제거

### Task 3.2: 벤치마크 #2 (SIMD) ✅
- [x] Baseline: ~950 MiB/s → 최적화 후: ~1.0 GiB/s
- [x] **+17-25% 성능 개선** 달성

---

## Phase 4: 병렬 처리 ✅

### Task 4.1: rayon 적용 ✅
- [x] `CLIParser::parse_all()` 기본 구현 추가
- [x] `rayon::par_iter()` + `flat_map()` 병렬 처리
- [x] glob 의존성 main dependencies로 이동

### Task 4.2: 벤치마크 #3 (병렬) ✅
- [x] `parse_all_files_parallel` 벤치마크 추가
- [x] Sequential: ~4.7s (630 MiB/s)
- [x] **Parallel: ~1.5s (2.0 GiB/s)** - 3.2x 성능 향상

---

## Phase 5: TUI - In Progress

### Task 5.1: 기본 TUI 구조 ✅
- [x] `src/tui/mod.rs` - 모듈 export
- [x] `src/tui/app.rs` - AppState enum, App struct, run()
- [x] `src/tui/widgets/mod.rs` - 위젯 모듈
- [x] `src/tui/widgets/spinner.rs` - 로딩 스피너
- [x] `src/tui/widgets/heatmap.rs` - 52주 히트맵
- [x] `src/tui/widgets/overview.rs` - Overview 레이아웃

### Task 5.2: Overview 뷰 ✅
- [x] 총 토큰/비용 표시
- [x] 52주 히트맵 (percentile 기반 강도)
- [x] q 키로 종료

### Task 5.2.1: Overview 가독성 개선 ✅
- [x] 히트맵 2x2 블록 셀 (상하좌우 반블록 조합)
- [x] 히트맵 7행 (Mon~Sun) + 빈 셀 표시
- [x] 월 라벨 렌더링
- [x] Legend 위젯
- [x] TabBar 위젯 + Tab 전환 (Tab/Shift+Tab)
- [x] Hero stat (중앙 정렬 큰 숫자)
- [x] Sub-stats: Cost only (Today/Week/Month 제거 - 단순화)
- [x] 반응형 주 수 (52/26/13)
- [x] 키바인딩 힌트
- [x] All Tokens 계산 (input+output+cache_read+cache_creation)

### Task 5.3: Models 뷰 ✅
- [x] 모델별 사용량 breakdown
- [x] 퍼센티지 바 차트
- [x] 비용 내림차순 정렬
- [x] TDD 테스트 9개 통과

### Task 5.4: Daily 뷰 ✅
- [x] 일별 테이블 (Date, Model, Input, Output, Cache, Total, Cost)
- [x] Unicode 스파크라인 차트 (▓/░ 기반)
- [x] 최신순 정렬 (desc)
- [x] ↑↓/j/k 스크롤 네비게이션
- [x] TDD 테스트 11개 통과

### Task 5.5: Stats 뷰 ✅
- [x] StatsData 구조체 (6개 통계)
- [x] StatsView 반응형 카드 그리드 (2x3)
- [x] TDD 테스트 8개 통과

### Task 5.6: 키보드/마우스 네비게이션 ✅
- [x] 1-4 키로 뷰 전환
- [x] 방향키 스크롤 (↑↓/j/k - Task 5.4에서 구현)
- [x] q 종료 (Task 5.1에서 구현)

### Task 5.7: TUI 가독성/반응형 개선 ✅
- [x] Models/Daily 테이블 중앙정렬 (calculate_table_offset)
- [x] Daily 컬럼 폭 확장 (14→18자, 100조까지 지원)
- [x] Stats 카드 색상 차별화 (테두리/제목 각 카드별 고유 색상)
- [x] Stats 카드 3열 고정 (2x3 균형 그리드)
- [x] Legend 반응형 배치 (heatmap 우측 끝에 맞춤)
- [x] Models/Daily "Usage" 헤더 추가 및 bar 크기 확장 (8→14)
- [x] Usage 컬럼 우측 정렬

---

## Phase 6: CLI 명령어

### Task 6.1: clap 설정 ✅
- [x] `toktrack` → TUI (기본값)
- [x] `toktrack daily` → 텍스트 테이블
- [x] `toktrack stats` → 통계 텍스트
- [x] `Backup` 서브커맨드 제거

### Task 6.2: JSON 출력 모드 ✅
- [x] `toktrack daily --json` → JSON 배열
- [x] `toktrack stats --json` → JSON 객체
- [x] `StatsData`를 `types/usage.rs`로 이동 (CLI/TUI 공유)

---

## Phase 7: 릴리즈 준비

### Task 7.1: README 작성 ✅
- [x] 설치 방법
- [x] 사용법
- [x] 스크린샷 placeholder
- [x] 벤치마크 결과
- [x] README.ko.md 한국어 번역
- [x] 언어 스위처 (영어/한국어)

### Task 7.2: 크로스 컴파일 빌드
- Linux (x86_64, aarch64)
- macOS (x86_64, aarch64)
- Windows (x86_64)

### Task 7.3: npm 패키지 배포 설정 ✅

**목표**: `npx toktrack` / `bunx toktrack`으로 실행 가능

```
npm/
├── package.json
├── bin/
│   ├── toktrack-darwin-arm64
│   ├── toktrack-darwin-x64
│   ├── toktrack-linux-x64
│   └── toktrack-win32-x64.exe
└── bin/run.js
```

**bin/run.js 구현:**
```javascript
#!/usr/bin/env node
const { execFileSync } = require('child_process');
const path = require('path');
const os = require('os');

const platform = os.platform();
const arch = os.arch();
const ext = platform === 'win32' ? '.exe' : '';
const binary = path.join(__dirname, `toktrack-${platform}-${arch}${ext}`);

try {
  execFileSync(binary, process.argv.slice(2), { stdio: 'inherit' });
} catch (e) {
  if (e.status) process.exit(e.status);
  throw e;
}
```

**package.json:**
```json
{
  "name": "toktrack",
  "version": "0.1.0",
  "description": "Ultra-fast AI CLI token usage tracker",
  "bin": {
    "toktrack": "./bin/run.js"
  },
  "files": ["bin/"],
  "repository": "https://github.com/jaehojang/toktrack",
  "license": "MIT"
}
```

### Task 7.4: GitHub Actions CI/CD

```yaml
name: Release

on:
  push:
    tags: ['v*']

jobs:
  build:
    strategy:
      matrix:
        include:
          - os: macos-latest
            target: aarch64-apple-darwin
            name: darwin-arm64
          - os: macos-latest
            target: x86_64-apple-darwin
            name: darwin-x64
          - os: ubuntu-latest
            target: x86_64-unknown-linux-gnu
            name: linux-x64
          - os: windows-latest
            target: x86_64-pc-windows-msvc
            name: win32-x64

    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}
      - run: cargo build --release --target ${{ matrix.target }}
      - uses: actions/upload-artifact@v4
        with:
          name: toktrack-${{ matrix.name }}
          path: target/${{ matrix.target }}/release/toktrack*

  publish-npm:
    needs: build
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/download-artifact@v4
      - run: |
          mkdir -p npm/bin
          cp toktrack-*/toktrack* npm/bin/
          chmod +x npm/bin/*
      - uses: actions/setup-node@v4
      - run: cd npm && npm publish
        env:
          NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}

  github-release:
    needs: build
    runs-on: ubuntu-latest
    steps:
      - uses: actions/download-artifact@v4
      - uses: softprops/action-gh-release@v2
        with:
          files: toktrack-*/*
```

### Task 7.5: GitHub Release ✅
- [x] v0.1.0 태그 생성 및 푸시
- [x] release.yml로 자동 바이너리 빌드/첨부
- [x] npm publish 자동화

---

## 타임라인 예상

| Phase | 내용 | 예상 시간 |
|-------|------|----------|
| Phase 0 | 프로젝트 셋업 | 1h |
| Phase 1 | 코어 파싱 (TDD) | 2-3h |
| Phase 2 | 서비스 레이어 | 2h |
| Phase 3 | SIMD 최적화 | 1h |
| Phase 4 | 병렬 처리 | 1h |
| Phase 5 | TUI | 3-4h |
| Phase 6 | CLI 명령어 | 1h |
| Phase 7 | 릴리즈 + npm 배포 | 2-3h |
| **Total** | | **13-17h** |

---

**작성일**: 2026-01-26
