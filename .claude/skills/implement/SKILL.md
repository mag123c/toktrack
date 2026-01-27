---
name: implement
description: TDD implementation (RED→GREEN→REFACTOR) → verify → review
required_context:
  - .claude/ai-context/architecture.md
  - .claude/ai-context/conventions.md
---

# Implement

## Flow
```
Analysis → TDD(RED→GREEN→REFACTOR) → /verify → /review → Commit
```

## Execution

1. **Analysis**: Plan 확인, 영향 모듈 파악, 브랜치 생성 (`feat/`, `fix/`, `refactor/`)
2. **TDD Cycle**:
   - RED: 실패 테스트 먼저 작성
   - GREEN: 최소 코드로 통과
   - REFACTOR: 정리 (테스트 유지)
3. **Verify**: `/verify` 호출 (자동)
4. **Review**: `/review` 호출 (자동)
5. **Commit**: Conventional Commits + Co-Authored-By

## Commands
```bash
cargo test              # TDD
cargo clippy -- -D warnings
cargo fmt --check
```

## Rules
- 테스트 없는 구현 금지
- verify 실패 시 수정 후 재검증
- review 통과 후 커밋
