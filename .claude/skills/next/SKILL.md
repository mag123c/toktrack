---
name: next
description: 세션 시작 - 진행 상태 파악, 다음 작업 제시
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
   # docs/planning/*.md 파일들 읽기
   # 체크박스 상태 파악: [ ] 미완료, [x] 완료
   ```

2. **Git Log**
   ```bash
   git log --oneline -5
   git status --short
   ```

3. **Analyze**
   - 현재 Phase 식별
   - 완료된 태스크 수 / 전체 태스크 수
   - 다음 우선순위 태스크 식별

4. **Present** (테이블 형식)
   | Phase | Status | Progress |
   |-------|--------|----------|
   | Phase 0 | ✅ | 5/5 |
   | Phase 1 | 🔄 | 3/4 |

5. **Suggest**
   - 다음 태스크 요약
   - `/clarify` 실행 제안

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
- planning 파일 없으면 git log + 코드 상태로 추론
- 간결하게 출력 (5-10줄)
- 항상 /clarify 연결 제안
