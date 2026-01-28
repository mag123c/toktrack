---
name: clarify
description: Clarify requirements → auto Plan Mode
required_context:
  - .claude/ai-context/architecture.md
  - .claude/ai-context/conventions.md
---

# Clarify

## Flow
```
Record Original → AskUserQuestion → Summary → EnterPlanMode()
```

## Execution

1. **Record**: Note original request, identify ambiguous parts
2. **Question**: Use AskUserQuestion with specific options
3. **Summary**: Before/After comparison (Goal, Scope, Constraints, Success Criteria)
4. **Auto Plan**: Call `EnterPlanMode()` without user confirmation

## Rules
- No assumptions → ask questions
- Clarify to TDD-ready level
- Always enter Plan Mode after clarify

## Next Step
On plan approval → immediately call `/implement`
