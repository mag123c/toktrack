---
name: wrap
description: Session end - document updates, commit
required_context:
  - .claude/ai-context/architecture.md
---

# Wrap

## Flow
```
Git Status → Doc Check → User Selection → Execute → Complete
```

## Execution

1. **Git Status**
   ```bash
   git status --short
   git diff --stat HEAD~3
   ```

2. **Doc Check** (ai-context DSL rules)
   | Change | Target | Format |
   |--------|--------|--------|
   | trait/type | architecture.md | table/codeblock |
   | convention | conventions.md | table DSL |
   | module | CLAUDE.md | brief description |
   | task complete | docs/planning/*.md | checkbox [x] |

3. **User Selection**: AskUserQuestion
4. **Execute**: Run selected items

## DSL Rules (ai-context)
- Table > prose
- Codeblock > description
- Core only, minimize lines

## Commit
```
{type}({scope}): {summary}
```

## Completion
wrap complete = **skill chain finished**
Next task starts with new `/clarify` or `/next`
