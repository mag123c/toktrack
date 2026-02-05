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

## 2026-02-05: daily-column-priority
- **결정**: 좁은 화면에서 Usage 컬럼 유지, Input/Output/Cache 먼저 숨김
- **이유**: 시각적 바 차트(Usage)가 한눈에 패턴 파악에 유용
- **대안**: 기존 방식(Usage 먼저 숨김) - 숫자 데이터 우선이나 직관성 부족
- **숨김 순서**: Input → Output → Cache → Usage

## 2026-02-05: daily-model-display
- **결정**: 다중 모델 시 "주력모델 +N" 형식, 컬러 분리 (주력=accent, 카운트=muted)
- **이유**: "3 models"보다 정보량 높음, 시각적 계층으로 주력 모델 강조
- **기준**: 비용(cost_usd) 기준 최고 모델을 주력으로 선정
- **0토큰 필터**: 모든 모델 목록에서 토큰 0인 모델 제외 (Models탭, Daily팝업, 카운트)

## 2026-02-05: quit-default-yes
- **결정**: Quit 확인 팝업 기본 선택을 No → Yes로 변경
- **이유**: Ctrl+C 의도적 액션이므로 빠른 종료 UX 제공

## 2026-02-05: popup-styling
- **결정**: 팝업 통일 스타일 - 선택색상 (Quit=LightRed, Update=LightGreen), 2줄 힌트
- **이유**: 일관된 UX, 힌트 가독성 향상, 액션 특성에 맞는 컬러

