---
name: clarify
description: |
  Adaptive requirements clarification with auto-depth routing.
  Shallow (Q&A) for simple tasks, Deep (exploration + DRAFT + PLAN) for complex ones.
  Escalates automatically when ambiguity persists.
required_context:
  - .claude/ai-context/architecture.md
  - .claude/ai-context/conventions.md
allowed-tools:
  - Read
  - Grep
  - Glob
  - Task
  - Write
  - AskUserQuestion
---

# /clarify — Adaptive Requirements Clarification

## Chain (MUST)
| 이전 | 현재 | 다음 |
|------|------|------|
| 세션 시작 | /clarify | EnterPlanMode() → /implement |

## Core Model

```
/clarify → 복잡도 측정 → 충분히 명확?
                          ├─ Yes → EnterPlanMode()
                          └─ No  → 더 깊은 clarify (탐색, 분석, DRAFT...)
                                   → 재측정 → 반복
```

종료 조건: **"이 정보로 구현 가능한가?"**

---

## Step 0: Route — Complexity Assessment

요청 수신 즉시 내부적으로 복잡도를 측정한다 (사용자에게 노출하지 않음).

### Complexity Signals

| Signal | LOW | HIGH |
|--------|-----|------|
| 요청 길이 | 짧고 구체적 | 길거나 모호 |
| 키워드 | "추가", "수정", "고쳐줘" | "설계", "마이그레이션", "처음부터" |
| 불확실성 표현 | 없음 | "잘 모르겠는데", "어떻게 해야 할지" |
| 영향 범위 | 단일 파일/모듈 | 크로스커팅, 여러 서비스 |
| 리스크 | 낮음 (UI, 텍스트) | 높음 (DB, 인증, 브레이킹 API) |
| 기존 패턴 | 명확히 존재 | 없거나 낯선 스택 |

- **LOW** → Shallow Path (이 파일 내에서 완결)
- **HIGH** → Deep Path (`deep/DEEP.md` 참조)
- **애매하면** → Shallow로 시작, 에스컬레이션 조건 감시

---

## Shallow Path (Low Complexity)

빠른 Q&A로 모호함만 제거하고 바로 Plan Mode로 진입.

### Execution

1. **Record**: 원본 요청 기록 + 모호한 부분 식별
2. **Question**: `AskUserQuestion` (구체적 옵션, 2-3라운드)
3. **Escalation Check**: 에스컬레이션 조건 확인 (아래 참조)
4. **Summary**: Before/After 비교 (Goal, Scope, Constraints, Success Criteria)
5. **Auto Plan**: `EnterPlanMode()` 즉시 호출

### Rules
- 가정 금지 → 질문
- TDD 가능 수준까지 구체화
- 3라운드 내 해결 목표

### Escalation → Deep Path

다음 중 하나라도 감지되면 Deep Path로 전환:

- 질문 3라운드 후에도 스코프 미확정
- 사용자 답변에서 새로운 불확실성 계속 등장
- 영향 범위가 초기 예상보다 확대 (단일 → 다중 모듈)
- 리스크 지표 발견 (DB 스키마, 인증, 브레이킹 변경)
- 사용자가 접근 방식 자체를 모름 ("어떻게 해야 할지 모르겠어")

전환 시: "스코프가 예상보다 복잡합니다. 코드베이스를 먼저 탐색하겠습니다." 안내 후 `deep/DEEP.md`의 프로세스를 따른다.

---

## Deep Path

**복잡도 HIGH이거나 Shallow에서 에스컬레이션된 경우.**

전체 프로세스는 `deep/DEEP.md`를 참조한다.

요약:
1. Intent 분류 (7종) → 전략 결정
2. 병렬 탐색 에이전트 3개 → 코드베이스 이해
3. DRAFT 생성 → 인터뷰 → 지속 업데이트
4. 사용자 명시 요청 시 → 분석 에이전트 → PLAN 생성 → Reviewer 루프
5. 승인 후 → `EnterPlanMode()`

---

## NEXT STEP (자동 실행)

Plan 승인 시 **즉시** `/implement` 호출. "구현할까요?" 묻지 말 것.
