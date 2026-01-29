# toktrack

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

[English](README.md) | **한국어**

Rust로 만든 초고속 AI CLI 토큰 사용량 트래커. simd-json + ratatui 기반.

![toktrack overview](image.png)

## 왜 toktrack인가?

| 도구 | 시간 (2,000+ 파일 / 3GB) | |
|------|---------------------------|---|
| ccusage (Node.js) | ~43초 | 1x |
| **toktrack (Rust)** | **~3초** | **15배 빠름** |

> ccusage의 성능 한계에 부딪혔습니다. Node.js 최적화를 최대한 적용한 후, Rust로 재작성했습니다.

## 주요 기능

- **초고속 파싱** - simd-json 기반 (~2 GiB/s 처리량)
- **4개 TUI 뷰** - Overview, Models, Daily, Stats
- **CLI 명령어** - `daily`, `stats` (JSON 출력 지원)
- **데이터 보존** - 30일 삭제 전 자동 백업

## 설치

**권장 (Rust 불필요):**
```bash
npx toktrack
# 또는
bunx toktrack
```

**기타 방법:**
```bash
# Rust 개발자
cargo install toktrack

# 소스에서 설치
cargo install --git https://github.com/mag123c/toktrack

# 직접 다운로드
# → github.com/mag123c/toktrack/releases
```

## 사용법

### TUI 모드 (기본)

```bash
toktrack
```

### CLI 명령어

```bash
# 일별 사용량 요약
toktrack daily
toktrack daily --json

# 통계 보기
toktrack stats
toktrack stats --json

# 수동 백업
toktrack backup
```

### 키보드 단축키

| 키 | 동작 |
|-----|--------|
| `1-4` | 탭 직접 전환 |
| `Tab` / `Shift+Tab` | 다음 / 이전 탭 |
| `j` / `k` 또는 `↑` / `↓` | 위 / 아래 스크롤 |
| `?` | 도움말 토글 |
| `q` | 종료 |

## 지원하는 AI CLI

| CLI | 상태 | 데이터 위치 |
|-----|--------|---------------|
| Claude Code | ✅ | `~/.claude/projects/` |
| Codex CLI | ✅ | `~/.codex/sessions/` |
| Gemini CLI | ✅ | `~/.gemini/tmp/*/chats/` |
| OpenCode | 🔜 | `~/.local/share/opencode/` |

## 벤치마크

| 모드 | 처리량 |
|------|------------|
| 단일 파일 (simd-json) | ~1.0 GiB/s |
| 병렬 처리 (rayon) | ~2.0 GiB/s |

**실제 성능** (2,000+ 파일 / 3GB 데이터):

| 도구 | 시간 | |
|------|------|---|
| ccusage (Node.js) | ~43초 | 1x |
| **toktrack** | **~3초** | **15배 빠름** |

## 데이터 보존

Claude Code와 Gemini CLI는 기본적으로 30일 후 세션 데이터를 삭제합니다.

toktrack은 첫 실행 시 자동으로 `~/.toktrack/cache/`에 데이터를 백업합니다.

Claude Code의 자동 삭제 비활성화:
```json
// ~/.claude/settings.json
{
  "cleanupPeriodDays": 9999999999
}
```

## 개발

```bash
make check    # fmt + clippy + test (커밋 전 실행)
cargo test    # 테스트 실행
cargo bench   # 벤치마크 실행
```

## 로드맵

- [ ] **성능 개선** - 3GB+ 데이터셋 1초 이내 목표
- [ ] **OpenCode 지원**

## 기여하기

이슈와 PR 환영합니다!

```bash
make check  # PR 전 실행
```

## 라이선스

MIT
