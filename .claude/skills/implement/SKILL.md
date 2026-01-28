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
Analysis → TDD(RED→GREEN→REFACTOR) → /verify → /review → /wrap
```

## Execution

1. **Analysis**: Review plan, identify affected modules
2. **TDD Cycle**:
   - RED: Write failing test first
   - GREEN: Minimal code to pass
   - REFACTOR: Clean up (keep tests passing)
3. **Auto-call `/verify`**: On implementation complete
4. **Auto-call `/review`**: On verify pass
5. **Auto-call `/wrap`**: On review PASS

## Commands
```bash
cargo test
cargo clippy -- -D warnings
cargo fmt --check
```

## Rules
- No implementation without test
- On verify fail → fix and retry
- On review FAIL → fix and retry
- Complete full chain without stopping
