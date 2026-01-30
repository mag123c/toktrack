# Deep Path — Interview-Driven Planning

복잡도 HIGH이거나 Shallow에서 에스컬레이션된 경우의 전체 프로세스.

---

## Interview Mode (Default)

### Step 1: Initialize

#### 1.1 Classify Intent

| Intent Type | Keywords | Strategy |
|-------------|----------|----------|
| **Refactoring** | "refactoring", "cleanup", "improve" | Safety first, regression prevention |
| **New Feature** | "add", "new", "implement" | Pattern exploration, integration points |
| **Bug Fix** | "bug", "error", "broken", "fix" | Reproduce → Root cause → Fix |
| **Architecture** | "design", "structure", "architecture" | Trade-off analysis |
| **Research** | "investigate", "analyze", "understand" | Investigation only, NO implementation |
| **Migration** | "migration", "upgrade", "transition" | Phased approach, rollback plan |
| **Performance** | "performance", "optimize", "slow" | Measure first, profile → optimize |

#### 1.2 Launch Parallel Exploration

3개 에이전트를 **하나의 메시지에서 병렬** 실행:

```
Task(subagent_type="Explore",
     prompt="Find: existing patterns for [feature type]. Report as file:line format.")

Task(subagent_type="Explore",
     prompt="Find: project structure, build/test/lint commands")

Task(subagent_type="Explore",
     prompt="Find internal documentation: ADRs, conventions, constraints, READMEs")
```

#### 1.3 Create Draft File

```
Write(".dev/specs/{name}/DRAFT.md", initial_draft)
```

`templates/DRAFT_TEMPLATE.md` 구조를 따른다.

### Step 1.5: Present Exploration Summary

병렬 탐색 완료 후, 인터뷰 시작 전에 탐색 결과 요약을 사용자에게 제시:

```
"코드베이스 탐색 결과:
 - 구조: [주요 디렉토리 구조]
 - 관련 패턴: [발견된 기존 패턴 2-3개]
 - 내부 문서: [관련 ADR/컨벤션]
 - 프로젝트 명령어: lint/test/build

이 맥락이 맞는지 확인 후 진행하겠습니다."
```

### Step 2: Gather Requirements

#### ASK (사용자만 아는 것)
- **Boundaries**: 하면 안 되는 것
- **Trade-offs**: 여러 유효한 선택지
- **Success Criteria**: 완료 조건

#### DISCOVER (에이전트가 탐색)
- 파일 위치, 기존 패턴, 통합 지점, 프로젝트 명령어

#### PROPOSE (리서치 후 제안)
- 탐색 결과 기반으로 제안 → 사용자는 승인/수정만

> **핵심**: 질문 최소화, 리서치 기반 제안 최대화

### Step 3: Update Draft Continuously

- 사용자 답변 → **User Decisions** 테이블 업데이트
- 탐색 결과 → **Agent Findings** 업데이트
- 해결된 항목 → **Open Questions**에서 제거
- 방향 합의 시 → **Direction** 업데이트

### Step 4: Plan Transition Check

조건:
- [ ] Critical Open Questions 전부 해결
- [ ] User Decisions에 핵심 결정 기록
- [ ] Success Criteria 합의
- [ ] 사용자가 명시적으로 플랜 요청 ("플랜 만들어", "make it a plan" 등)

**사용자가 요청하지 않으면 플랜을 생성하지 않는다.**

---

## Plan Generation Mode (명시적 요청 시)

### Step 1: Validate Draft Completeness

DRAFT에 다음이 있는지 확인:
- [ ] What & Why 완성
- [ ] Boundaries 명시
- [ ] Success Criteria 정의
- [ ] Critical Open Questions 비어있음
- [ ] Agent Findings에 Patterns, Commands 존재

미완성 시 → Interview Mode로 복귀.

### Step 2: Run Parallel Analysis Agents

```
Task(subagent_type="general-purpose",
     prompt="Gap analysis: missing requirements, AI pitfalls, must-NOT-do items.
             Goal: [DRAFT What & Why]
             Current Understanding: [DRAFT summary]")

Task(subagent_type="general-purpose",
     prompt="Tradeoff analysis: risk per change area, simpler alternatives, dangerous changes.
             Proposed Approach: [DRAFT Direction]
             Boundaries: [DRAFT Boundaries]")
```

외부 리서치 (migration, 새 라이브러리, 낯선 기술일 때만):
```
Task(subagent_type="general-purpose",
     prompt="Research official docs for [library/framework]: [specific question]")
```

**Gap 분석 결과**: Must NOT Do에 추가
**Tradeoff 분석 결과**: 리스크 태그 (LOW/MEDIUM/HIGH) 부여, HIGH는 사용자 승인 필요

### Step 3: Decision Summary Checkpoint

플랜 생성 전 모든 결정사항 (사용자 결정 + 에이전트 자동 결정) 요약 제시:

```
AskUserQuestion(
  question: "다음 결정 사항을 확인해주세요. 수정이 필요한 항목이 있나요?",
  options: [
    { label: "확인 완료", description: "모든 결정 사항이 맞습니다" },
    { label: "수정 필요", description: "일부 항목을 변경하고 싶습니다" }
  ]
)
```

### Step 4: Create Plan File

```
Write(".dev/specs/{name}/PLAN.md", plan_content)
```

`templates/PLAN_TEMPLATE.md` 구조를 따른다.

#### DRAFT → PLAN 매핑

| DRAFT Section | PLAN Section |
|---------------|--------------|
| What & Why | Context > Original Request |
| User Decisions | Context > Interview Summary |
| Agent Findings | Context > Research Findings |
| Deliverables | Work Objectives > Concrete Deliverables |
| Boundaries | Work Objectives > Must NOT Do |
| Success Criteria | Work Objectives > Definition of Done |
| Agent Findings > Patterns | TODOs > References |
| Agent Findings > Commands | TODO Final > Verification commands |
| Direction > Work Breakdown | TODOs + Dependency Graph |

### Step 5: Reviewer

```
Task(subagent_type="feature-dev:code-reviewer",
     prompt="Review this plan: .dev/specs/{name}/PLAN.md")
```

### Step 6: Handle Reviewer Response

**REJECT (Cosmetic)** — 포맷, 명확성, 필드 누락:
→ 자동 수정 → 재심사 → OKAY까지 반복

**REJECT (Semantic)** — 요구사항, 스코프, 로직 변경:
→ 사용자에게 제시 → 사용자 선택에 따라 수정 → 재심사

Semantic 판단 기준 — 다음이 변경되면 Semantic:
- Work Objectives (scope, deliverables, definition of done)
- TODO steps or acceptance criteria
- Risk level or rollback strategy
- Must NOT Do items

그 외 (문구, 포맷, 필드 완성도) → Cosmetic.

**OKAY**:
1. DRAFT 삭제: `Bash("rm .dev/specs/{name}/DRAFT.md")`
2. 사용자에게 플랜 준비 완료 안내
3. `EnterPlanMode()` 호출

---

## Risk Tagging

| Risk | Meaning | Requirements |
|------|---------|--------------|
| LOW | Reversible, isolated | Standard verification |
| MEDIUM | Multiple files, API changes | Verify block + reviewer |
| HIGH | DB schema, auth, breaking API | Verify + rollback + human approval |

---

## File Locations

| Type | Path | When |
|------|------|------|
| Draft | `.dev/specs/{name}/DRAFT.md` | Interview 중 |
| Plan | `.dev/specs/{name}/PLAN.md` | Plan generation 후 |
