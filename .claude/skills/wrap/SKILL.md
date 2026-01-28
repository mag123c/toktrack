---
name: wrap
description: 세션 종료 - 문서 업데이트, 커밋
required_context:
  - .claude/ai-context/architecture.md
---

# Wrap

## Chain (MUST - 종료 단계)
```
/clarify → Plan Mode → /implement → /verify → /review → /wrap
                                                        ^^^^^
                                                        현재 단계 (종료)
```
| 이전 | 현재 | 다음 |
|------|------|------|
| `/review` PASS | `/wrap` | 세션 완료 |

## Flow
```
Git Status → Doc Check → User Selection → Execute → 완료
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
```

## COMPLETION
wrap 완료 = **스킬 체인 정상 종료**
다음 작업은 새 `/clarify` 또는 `/next`로 시작
