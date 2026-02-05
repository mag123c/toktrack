# Project Decisions

설계 결정 누적 기록. `/clarify` 완료 시 자동 추가.

## Format

```markdown
## YYYY-MM-DD: {feature-name}
- **결정**: 무엇을 결정했는가
- **이유**: 왜 이 선택을 했는가
- **대안**: 고려했으나 선택하지 않은 옵션 (있으면)
- **참조**: .dev/specs/{feature-name}/PLAN.md
```

---

## 2026-01-26: project-init
- **결정**: Rust + simd-json + ratatui 스택
- **이유**: Node.js 대비 40-100x 성능 목표, 제로 오버헤드, SIMD 파싱
- **대안**: Node.js 캐싱 최적화 (7s까지 개선했으나 불충분)

## 2026-01-26: architecture
- **결정**: Parser → Services → Cache → TUI 레이어 분리
- **이유**: 관심사 분리, 테스트 용이성, 확장성

## 2026-01-28: cli-parsers
- **결정**: trait 기반 다형성 (`Box<dyn CLIParser>`)
- **이유**: 멀티 CLI 지원 확장성 (Claude, Codex, Gemini 등)

## 2026-02-05: model-normalizer
- **결정**: `normalize_model_name()` + `display_name()` 분리
- **이유**: 집계용 정규화 vs UI용 축약 표시 목적 분리

## 2026-02-05: quit-behavior
- **결정**: Ctrl+C만 종료, q/Esc 트리거 제거
- **이유**: 터미널 표준 동작 준수, crossterm은 OS 무관 CONTROL 사용

