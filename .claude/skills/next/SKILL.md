---
name: next
description: Session start - check progress, suggest next task
required_context:
  - .claude/ai-context/architecture.md
---

# Next

## Flow
```
Read Planning → Git Log → Analyze → Present → Suggest /clarify
```

## Execution

1. **Read Planning**
   ```bash
   # Read docs/planning/*.md files
   # Check checkbox status: [ ] incomplete, [x] complete
   ```

2. **Git Log**
   ```bash
   git log --oneline -5
   git status --short
   ```

3. **Analyze**
   - Identify current phase
   - Count completed/total tasks
   - Identify next priority task

4. **Present** (table format)
   | Phase | Status | Progress |
   |-------|--------|----------|
   | Phase 0 | ✅ | 5/5 |
   | Phase 1 | 🔄 | 3/4 |

5. **Suggest**
   - Summarize next task
   - Suggest running `/clarify`

## Output Format
```markdown
## Current Status
- Phase: {current_phase}
- Progress: {completed}/{total} tasks

## Next Task
**{task_id}: {task_name}**
{brief_description}

## Action
Run `/clarify` to start: {task_summary}
```

## Rules
- If no planning files → infer from git log + code state
- Keep output concise (5-10 lines)
- Always suggest /clarify connection
