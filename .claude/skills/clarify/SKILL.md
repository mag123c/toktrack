---
name: clarify
description: Clarify requirements → auto Plan Mode
required_context:
  - .claude/ai-context/architecture.md
---

# Clarify

## Flow
```
Record Original → AskUserQuestion → Summary → EnterPlanMode()
```

## Execution

1. **Record**: 원본 요청 기록 + 모호한 부분 식별
2. **Question**: AskUserQuestion으로 명확화 (구체적 옵션 제시)
3. **Summary**: Before/After 비교 (Goal, Scope, Constraints, Success Criteria)
4. **Auto Plan**: `EnterPlanMode()` 호출 (사용자 확인 없이)

## Rules
- 가정 금지 → 질문
- TDD 가능 수준까지 구체화
- clarify 후 반드시 Plan Mode 진입

## After Plan Approval
Plan이 승인되면 즉시 `/implement` 스킬 호출하여 구현 시작
