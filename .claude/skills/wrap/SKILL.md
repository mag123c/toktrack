---
name: wrap
description: 세션 종료 - 문서 업데이트, 커밋
required_context:
  - .claude/ai-context/architecture.md
---

# Wrap

## Flow
```
Git Status → Doc Check → User Selection → Execute
```

## Execution

1. **Git Status**
   ```bash
   git status --short
   git diff --stat HEAD~3
   ```

2. **Doc Check** (ai-context DSL 규칙 준수)
   | 변경 | 대상 | 형식 |
   |------|------|------|
   | trait/타입 | architecture.md | 테이블/코드블록 |
   | 컨벤션 | conventions.md | 테이블 DSL |
   | 모듈 | CLAUDE.md | 간결한 설명 |
   | 태스크 완료 | docs/planning/*.md | 체크박스 [x] |

3. **User Selection**: AskUserQuestion
4. **Execute**: 선택 항목 실행

## DSL Rules (ai-context)
- 테이블 > 산문
- 코드블록 > 설명
- 핵심만, 라인수 최소화

## Commit
```
{type}({scope}): {summary}

Co-Authored-By: Claude <noreply@anthropic.com>
```
