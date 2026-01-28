# toktrack

Ultra-fast AI CLI token usage tracker. Rust + simd-json + ratatui.

## Quick Start

```bash
cargo build --release
./target/release/toktrack
```

## Context

| File | Content |
|------|---------|
| [architecture.md](.claude/ai-context/architecture.md) | Layers, paths, traits, data flow |
| [conventions.md](.claude/ai-context/conventions.md) | Naming, TDD, error handling, commits |

## Dev Workflow (MUST FOLLOW)

```
/next → /clarify → Plan Mode → /implement → /verify → /review → /wrap
```

**Session start**: Run `/next` to see current progress and next task.

## Skill Chain (ENFORCED - 위반 금지)

| 완료 단계 | 다음 호출 | 조건 | 확인 필요 |
|-----------|----------|------|----------|
| Plan 승인 | `/implement` | 즉시 | **NO** |
| `/implement` | `/verify` | 구현 완료 시 | **NO** |
| `/verify` | `/review` | 통과 시 | **NO** |
| `/review` | `/wrap` | PASS 시 | **NO** |

**CRITICAL**:
- 각 단계 완료 후 "다음 단계 진행할까요?" **묻지 말 것**
- 사용자 확인 없이 **즉시** 다음 스킬 호출
- 체인 중단 = 세션 미완료 (버그로 간주)

**예외 상황**:
- `/verify` 실패 → 수정 후 재실행
- `/review` FAIL → `/implement`로 복귀
- 사용자가 명시적으로 중단 요청

**Plan provided directly** → skip clarify/plan, start with `/implement` skill immediately

See `.claude/skills/` for skill details.

## Commands

```bash
make check      # fmt + clippy + test (pre-commit)
make setup      # Configure git hooks
cargo test      # Run tests
cargo bench     # Benchmarks
```

## Commit Rules

```
{type}({scope}): {description}
```

types: `feat|fix|refactor|docs|test|chore|perf`
scopes: `parser|tui|services|cache|cli`
