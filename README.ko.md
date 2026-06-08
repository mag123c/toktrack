# toktrack

[![CI](https://github.com/mag123c/toktrack/actions/workflows/ci.yml/badge.svg)](https://github.com/mag123c/toktrack/actions/workflows/ci.yml)
[![npm](https://img.shields.io/npm/v/toktrack)](https://www.npmjs.com/package/toktrack)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

[English](README.md) | **한국어**

> **⚠️ 알고 계셨나요?** Claude Code는 **기본적으로 30일 후 세션 데이터를 삭제**합니다. 삭제되면 토큰 사용량과 비용 기록은 영원히 사라집니다 — 보존하지 않는 한.

**비용 기록을 절대 잃지 않는 토큰 & 비용 트래커.** 대부분의 도구는 매 실행마다 CLI 세션 파일을 다시 읽습니다 — 그래서 Claude Code가 30일 후 파일을 삭제하면 비용 기록도 함께 사라집니다. toktrack은 **영구 캐시**를 유지해 기록이 살아남습니다.

**모든 AI 코딩 CLI**의 사용량을 한 곳에서 — Claude Code, Codex CLI, Gemini CLI, Qwen Code, OpenCode, PI Agent 통합 대시보드. Rust 기반이라 대용량 기록에서도 빠릅니다 (simd-json + rayon).

![toktrack overview](assets/demo.gif)

## 왜 toktrack인가?

| 문제 | 해결책 |
|------|--------|
| 🗑️ **Claude Code 30일 후 데이터 삭제** — 비용 기록 사라짐 | 💾 **영구 캐시** — CLI가 파일을 삭제해도 기록 유지 |
| 📊 **통합 뷰 없음** — CLI별로 데이터 분산 | 🎯 **원 대시보드** — Claude Code, Codex, Gemini, Qwen, OpenCode, PI Agent 통합 |
| 🐌 **대용량 기록 재스캔이 느림** | ⚡ **캐시 시 ~0.04초** — 매 실행 즉시 |

## 주요 기능

- **데이터 보존** — 영구 캐시로 CLI가 세션 파일을 삭제한 뒤에도 비용 기록 유지
- **멀티 CLI 지원** — Claude Code, Codex CLI, Gemini CLI, Qwen Code, OpenCode, PI Agent 한 곳에서
- **TUI 대시보드** — 3개 탭 (Overview, Stats, Models), 일별/주별/월별 뷰
- **CLI 명령어** — `daily`, `weekly`, `monthly`, `stats` (JSON 출력 지원)
- **사용량 리포트** — 공유 가능한 텍스트 & SVG 영수증 (`toktrack report`)
- **대용량에서도 빠름** — simd-json + rayon 병렬 파싱 (~3 GiB/s), 캐시 시 ~0.04초

## 설치

### npx (권장)

Rust 툴체인 불필요. 플랫폼에 맞는 바이너리를 자동으로 다운로드합니다.

```bash
npx toktrack
# 또는
bunx toktrack
```

### Homebrew (macOS / Linux)

```bash
brew tap mag123c/toktrack
brew install toktrack
```

### 소스에서 설치

```bash
cargo install --git https://github.com/mag123c/toktrack
```

### 미리 빌드된 바이너리

[GitHub Releases](https://github.com/mag123c/toktrack/releases)에서 다운로드하세요.

| 플랫폼 | 아키텍처 |
|---------|----------|
| macOS | x64, ARM64 |
| Linux | x64, ARM64 |
| Windows | x64 |

## 빠른 시작

```bash
# TUI 대시보드 실행
npx toktrack

# 오늘의 비용을 JSON으로 확인
npx toktrack daily --json

# 월별 요약
npx toktrack monthly --json
```

## 사용법

### TUI 모드 (기본)

```bash
toktrack
```

### CLI 명령어

```bash
# 특정 탭으로 TUI 열기
toktrack daily     # Overview (일별 보기)
toktrack weekly    # Overview (주별 보기)
toktrack monthly   # Overview (월별 보기)
toktrack stats     # Stats 탭

# JSON 출력 (스크립팅용)
toktrack daily --json
toktrack weekly --json
toktrack monthly --json
toktrack stats --json

# 사용량 리포트 (공유용 영수증)
toktrack report              # 최근 7일 (텍스트)
toktrack report --month      # 최근 30일
toktrack report --days 14    # 최근 N일
toktrack report --svg        # 텍스트 + SVG 파일

# 데이터 보존 감사 (디스크에 살아있음 vs 캐시에만 있음)
toktrack audit               # 소스별 커버리지 리포트
toktrack audit --json        # 머신 리더블
```

### 셸 통합

오늘의 AI 지출을 프롬프트에 바로 표시 (명령마다 갱신):

```
~/projects/api  main  $12.40
❯
```

zsh/bash/fish 몇 줄이면 됩니다 — **[셸 통합 가이드](docs/shell-integration.md)** 참고.

### 키보드 단축키

| 키 | 동작 |
|-----|--------|
| `1-4` | 탭 직접 전환 (Audit 포함) |
| `Tab` / `Shift+Tab` | 다음 / 이전 탭 |
| `j` / `k` 또는 `↑` / `↓` | 위 / 아래 스크롤 |
| `Enter` | 모델 상세 팝업 열기 (Daily 탭) |
| `d` / `w` / `m` | 일별 / 주별 / 월별 보기 (Daily 탭) |
| `?` | 도움말 토글 |
| `Ctrl+C` | 종료 |

## 지원하는 AI CLI

| CLI | 상태 | 데이터 위치 |
|-----|--------|---------------|
| Claude Code | ✅ | `~/.claude/projects/` |
| Codex CLI | ✅ | `~/.codex/sessions/` |
| Gemini CLI | ✅ | `~/.gemini/tmp/*/chats/` |
| Qwen Code | ✅ | `~/.qwen/tmp/*/chats/` |
| OpenCode | ✅ | `~/.local/share/opencode/storage/message/` |
| PI Agent | ✅ | `~/.pi/agent/sessions/` |
| Antigravity | ⚠️ 감지됨, 미지원 | `~/.gemini/antigravity-cli/` (로컬 파일에 토큰 사용량 없음) |

> 자체 비용을 기록하지 않는 소스(Gemini, Qwen, Codex, 최신 Claude 로그)의 비용은
> [LiteLLM](https://github.com/BerriAI/litellm) 가격으로 계산되며 `~` 마커(추정치)로 표시됩니다.
> 네트워크가 없을 때도 번들된 스냅샷으로 가격 계산이 동작합니다.

### 환경 변수

각 소스의 데이터 디렉터리는 상위 CLI가 쓰는 변수명으로 재지정할 수 있습니다:

| 변수 | 소스 | 기본값 |
|------|------|--------|
| `CLAUDE_CONFIG_DIR` | Claude Code (루트) | `~/.claude` (+ `/projects`) |
| `CODEX_HOME` | Codex CLI (루트) | `~/.codex` (+ `/sessions`) |
| `GEMINI_CLI_HOME` | Gemini CLI (홈 루트) | `~` (+ `/.gemini/tmp`) |
| `QWEN_HOME` | Qwen Code | `~/.qwen` (+ `/tmp`) |
| `OPENCODE_DATA_DIR` / `XDG_DATA_HOME` | OpenCode | `~/.local/share/opencode` |
| `PI_CODING_AGENT_SESSION_DIR` (이후 `PI_AGENT_DIR`) | PI Agent | `~/.pi/agent/sessions` |

```bash
export CLAUDE_CONFIG_DIR="/path/to/.claude"
```

커스텀 가격(`web_search`/`web_fetch` 요청당 단가 포함)은 `~/.toktrack/pricing.toml`에 설정할 수 있습니다.

## 성능

| 실행 | 시간 |
|------|------|
| 첫 실행 (콜드) | **~1.0초** |
| 일상 사용 (캐시) | **~0.04초** |

> Apple Silicon 기준, 2,000+ JSONL 파일 (3.4 GB). 영구 캐시 덕분에 매 실행 시 현재 날짜만 재계산하고 지난 날짜는 캐시에서 바로 읽으므로, 기록이 아무리 커져도 일상 사용은 즉시 끝납니다.
>
> **왜 빠른가?** SIMD JSON 파싱 ([simd-json](https://github.com/simd-lite/simd-json)) + 병렬 처리 ([rayon](https://github.com/rayon-rs/rayon)) = 콜드 경로에서 ~3 GiB/s 처리량.

## 데이터 보존

> **문제 상황:** Claude Code를 3개월간 사용하며 수십만 원을 썼습니다. 어느 날 총 지출을 확인하려는데 — 2개월 전 세션 파일이 이미 삭제되었습니다. 그 비용 데이터는 영원히 사라졌습니다.

**toktrack이 이를 해결합니다.** 일별 비용 요약을 독립적으로 캐시하므로, CLI가 원본 파일을 삭제한 후에도 사용 기록이 보존됩니다.

### CLI별 데이터 보존 정책 (숨겨진 위험)

| CLI | 기본 보존 기간 | 정책 |
|-----|----------------|------|
| Claude Code | **30일** | `cleanupPeriodDays` (기본값: 30) |
| Gemini CLI | 무제한 | opt-in `sessionRetention` |
| Codex CLI | 무제한 | 용량 제한만 (`max_bytes`) |

### toktrack 캐시 구조

```
~/.toktrack/
├── cache/
│   ├── claude-code_daily.json   # 일별 비용 요약
│   ├── codex_daily.json
│   ├── codex@devbox_daily.json
│   ├── gemini_daily.json
│   ├── opencode_daily.json
│   └── pi-agent_daily.json
├── remotes/
│   └── devbox/codex/sessions/   # 동기화된 원격 Codex JSONL 스냅샷
├── config.toml                  # 선택: 원격 소스 설정
└── pricing.json                 # LiteLLM 가격 정보 (1시간 TTL)
```

각 `*_daily.json`의 지난 날짜 데이터는 **불변**입니다 — 한번 집계된 날의 결과는 수정되지 않습니다. 현재 날짜만 매 실행마다 재계산됩니다. 따라서 Claude Code가 30일 후 세션 파일을 삭제하더라도, 캐시에 비용 기록이 그대로 남습니다.

### 무엇이 보존됐는지 직접 확인

`toktrack audit`는 소스별·날짜별로 데이터가 아직 **live**(raw 세션 파일이 디스크에 존재)인지, **cache-only**(CLI가 raw를 삭제 — toktrack만 보유)인지, **missing**(데이터 없음 — 안 쓴 날이거나 toktrack이 보기 전에 사라진 날; 손실로 단정하지 않음)인지 보여줍니다.

```
$ toktrack audit

Data preservation audit (2026-06-08)

  claude-code   2025-12-22 → 2026-06-08
    live (raw on disk):         79 days
    preserved (CLI deleted):    83 days
    no data (unused or lost):    7 days
  ...

  ► 100 days of cost history preserved that your CLIs already deleted.
```

전체 일별 내역은 `toktrack audit --json`, 또는 TUI에서 **Audit** 탭(`4` 키)으로 시각적 커버리지 맵을 확인하세요.

### Claude Code 자동 삭제 비활성화

```json
// ~/.claude/settings.json
{
  "cleanupPeriodDays": 9999999999
}
```

### 캐시 초기화

```bash
rm -rf ~/.toktrack/cache/
```

다음 실행 시 사용 가능한 세션 데이터로부터 캐시를 재구축합니다.

## 동작 방식

![architecture](assets/architecture.png)

**콜드 경로** (첫 실행): 전체 glob 스캔 → 병렬 SIMD 파싱 → 캐시 구축 → 집계.

**웜 경로** (캐시 있음): 캐시된 요약 로드 → 최근 파일만 파싱 (어제 자정 mtime 필터) → 병합 → 집계.

> **Deep Dive:** [Node.js CLI를 Rust로 재작성 — 43초에서 1초로](https://mag1c.tistory.com/601) | [English](https://medium.com/@diehreo/i-rewrote-a-node-js-cli-in-rust-it-went-from-43s-to-1s-c13e38e7fe88)

## 개발

```bash
make check    # fmt + clippy + test (커밋 전 실행)
cargo test    # 테스트 실행
cargo bench   # 벤치마크 실행
```

## 로드맵

현재 6개 CLI 지원 (Claude Code, Codex, Gemini, Qwen Code, OpenCode, PI Agent) — [지원하는 AI CLI](#지원하는-ai-cli) 참조. 예정: 실시간/번레이트 모니터링, MCP 서버 / statusline, 추가 CLI(Goose, Amp, Kimi, Copilot).

## 기여하기

이슈와 PR 환영합니다!

```bash
make check  # PR 전 실행
```

## Star History

<a href="https://www.star-history.com/#mag123c/toktrack&Date">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=mag123c/toktrack&type=Date&theme=dark" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=mag123c/toktrack&type=Date" />
   <img alt="Star History Chart" src="https://api.star-history.com/svg?repos=mag123c/toktrack&type=Date" />
 </picture>
</a>

## 라이선스

MIT
