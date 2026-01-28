---
name: review
description: Negative perspective code review (convention, quality, bugs, performance)
required_context:
  - .claude/ai-context/architecture.md
  - .claude/ai-context/conventions.md
---

# Review

## Purpose
Review implementation from a **negative perspective**. Goal is to find issues.

## Checklist

### 1. Convention Violations
- [ ] CLAUDE.md rule violations
- [ ] Naming convention violations
- [ ] Architecture layer violations

### 2. Code Quality
- [ ] Unnecessary complexity
- [ ] Duplicate code
- [ ] Magic numbers/strings
- [ ] Missing error handling

### 3. Potential Bugs
- [ ] Unhandled edge cases
- [ ] Off-by-one errors
- [ ] Missing null/None checks
- [ ] Race conditions

### 4. Performance
- [ ] Unnecessary allocations/copies
- [ ] O(n²)+ complexity
- [ ] Cache not utilized

### 5. Test Coverage
- [ ] Missing test cases
- [ ] Boundary value tests
- [ ] Error path tests

## Output
```markdown
## Review Result

### Issues Found
1. [CRITICAL/WARNING/INFO] Description

### Recommendations
- Suggested fixes

### Verdict
- [ ] PASS: Ready to commit → auto-call /wrap
- [ ] FAIL: Needs fixes → return to /implement
```

## Rules
- Goal is to find issues (no praise)
- Fix discovered issues before proceeding
- On FAIL → return to /implement

## Next Step
- **PASS**: immediately call `/wrap`
- **FAIL**: return to `/implement`, fix, restart chain
