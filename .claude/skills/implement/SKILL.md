---
name: implement
description: TDD implementation (RED→GREEN→REFACTOR) → verify → review
required_context:
  - .claude/ai-context/architecture.md
  - .claude/ai-context/conventions.md
---

# Implement

## Chain (MUST - 체인 컨트롤러)
```
/clarify → Plan Mode → /implement → /verify → /review → /wrap
                       ^^^^^^^^^^
                       현재 단계 (체인 컨트롤러)
```
| 이전 | 현재 | 다음 (자동 호출) |
|------|------|------------------|
| Plan 승인 | `/implement` | `/verify` → `/review` → `/wrap` |

**CRITICAL**: 구현 완료 후 "verify 할까요?" 묻지 말고 **즉시 실행**

## Flow
```
Analysis → TDD(RED→GREEN→REFACTOR) → /verify → /review → /wrap
```

## Execution

1. **Analysis**: Plan 확인, 영향 모듈 파악
2. **TDD Cycle**:
   - RED: 실패 테스트 먼저 작성
   - GREEN: 최소 코드로 통과
   - REFACTOR: 정리 (테스트 유지)
3. **Auto-call `/verify`**: 구현 완료 즉시 (확인 불필요)
4. **Auto-call `/review`**: verify 통과 즉시 (확인 불필요)
5. **Auto-call `/wrap`**: review PASS 즉시 (확인 불필요)

## Commands
```bash
cargo test              # TDD
cargo clippy -- -D warnings
cargo fmt --check
```

## Rules
- 테스트 없는 구현 금지
- verify 실패 시 수정 후 재검증
- review FAIL 시 수정 후 재검증
- **전체 체인 완료까지 중단 금지**
