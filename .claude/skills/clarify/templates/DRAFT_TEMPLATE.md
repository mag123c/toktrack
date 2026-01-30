# Draft Template

> Reference document for draft file structure during Interview Mode.
> DRAFT = Discovery (consensus building) - Collecting What/Why + How direction

**Schema Version**: 1.1

---

## File Location

`.dev/specs/{name}/DRAFT.md`

---

## Structure

```markdown
# Draft: {name}

## Intent Classification
- **Type**: [Refactoring|New Feature|Bug Fix|Architecture|Research|Migration|Performance]
- **Strategy**: [Strategy for this type]

## What & Why

### What (Goal)
- [1-2 sentences: what to achieve]

### Why (Background)
- [Why this is needed]
- [Current problems]

### Deliverables
- [ ] [Specific file/feature 1]
- [ ] [Specific file/feature 2]

## Boundaries

### Must NOT Do
- [Things not allowed]
- [Out of scope items]

### Constraints
- [Technical constraints]
- [Business constraints]

## Success Criteria
- [ ] [Verifiable condition 1]
- [ ] [Verifiable condition 2]

## User Decisions

| Question | Decision | Notes |
|----------|----------|-------|
| [Question 1] | [Decision] | [Rationale/Context] |

## Agent Findings

### Patterns
- `file:line` - Description

### Structure
- [File structure findings]

### Project Commands
- Type check: `command`
- Lint: `command`
- Test: `command`

## Open Questions

### Critical (Must resolve before Plan)
- [ ] [Unresolved question]

### Nice-to-have (Can decide later)
- [ ] [Nice but not required]

## Direction

### Approach
- [High-level implementation direction]

### Work Breakdown (Draft)
1. [Task 1] → outputs: [deliverable]
2. [Task 2] → depends on: Task 1
3. [Task 3] → parallel with: Task 2
```

---

## Field Descriptions

### Intent Classification

Identify task type and establish strategy.

| Intent Type | Keywords | Strategy |
|-------------|----------|----------|
| **Refactoring** | "refactoring", "cleanup", "improve", "migrate" | Safety first, regression prevention |
| **New Feature** | "add", "new", "implement" | Pattern exploration, integration points |
| **Bug Fix** | "bug", "error", "broken", "fix" | Reproduce → Root cause → Fix |
| **Architecture** | "design", "structure", "architecture" | Trade-off analysis, oracle consultation |
| **Research** | "investigate", "analyze", "understand" | Investigation only, NO implementation |
| **Migration** | "migration", "upgrade", "transition" | Phased approach, rollback plan |
| **Performance** | "performance", "optimize", "slow" | Measure first, profile → optimize |

### What & Why

**User domain** - Collected through conversation

- **What**: What to achieve (goal)
- **Why**: Why this is needed (background, problems)
- **Deliverables**: Specific outputs required

### Boundaries

**User domain** - Must ask

- **Must NOT Do**: Explicitly forbidden actions
- **Constraints**: Technical/business constraints

### Success Criteria

**Consensus domain** - Requires user agreement

- Write as verifiable conditions
- Maps to PLAN's Definition of Done

### User Decisions

**Record domain** - Track user decisions

| Column | Description |
|--------|-------------|
| Question | What choices existed |
| Decision | What user selected |
| Notes | Reason or context for selection |

### Agent Findings

**Agent domain** - Investigation results

- **Patterns**: Existing code patterns (`file:line` format required)
- **Structure**: File/directory structure
- **Project Commands**: lint, test, and other project commands

> Maps to PLAN's References and Completion Protocol

### Open Questions

**Uncertainty management** - Plan transition criteria

| Priority | Meaning | Plan Transition |
|----------|---------|-----------------|
| **Critical** | Can't create Plan without this | Must resolve |
| **Nice-to-have** | Can decide later | Not required |

### Direction

**How direction** - Details go in PLAN

- **Approach**: High-level implementation direction
- **Work Breakdown**: TODO split draft (with dependencies)

---

## PLAN Mapping

| DRAFT Section | PLAN Section |
|---------------|--------------|
| What & Why | Context > Original Request |
| User Decisions | Context > Interview Summary |
| Agent Findings (partial) | Context > Research Findings |
| Deliverables | Work Objectives > Concrete Deliverables |
| Boundaries | Work Objectives > Must NOT Do |
| Success Criteria | Work Objectives > Definition of Done |
| Agent Findings > Patterns | TODOs > References |
| Agent Findings > Commands | TODO Final > Verification commands |
| Direction > Work Breakdown | TODOs + Dependency Graph |

---

## Question Principles

### What to ASK (only user knows)
- Boundaries (things not allowed)
- Trade-off decisions (A vs B)
- Business constraints

### What to INVESTIGATE (agent discovers)
- Existing patterns, file structure
- Project commands
- Impact scope

### What to PROPOSE (confirm after investigation)
- "This approach should work" → Y/N
- Recommendations based on existing patterns

> **Key**: Minimize questions, maximize proposals after research

---

## Usage

### When to Create
- When user requests a task

### When to Update
- After user response
- After background agent completes
- When decisions change

### Plan Transition Conditions
- [ ] All Critical Open Questions resolved
- [ ] Key decisions recorded in User Decisions
- [ ] Success Criteria agreed
- [ ] User requests "make it a plan"

### When to Delete
- After Plan is approved by reviewer

---

## Example

```markdown
# Draft: api-auth

## Intent Classification
- **Type**: New Feature
- **Strategy**: Pattern exploration, integration points

## What & Why

### What (Goal)
- Add JWT-based authentication to API endpoints

### Why (Background)
- All APIs currently exposed as public
- Need user-specific data access control

### Deliverables
- [ ] `src/middleware/auth.ts` - Authentication middleware
- [ ] `src/config/auth.json` - JWT configuration file

## Boundaries

### Must NOT Do
- Do not modify existing public endpoints
- Do not install new npm packages

### Constraints
- Use existing jsonwebtoken library
- Follow Express middleware pattern

## Success Criteria
- [ ] Request without token → 401 Unauthorized
- [ ] Valid token → Pass to next handler
- [ ] All existing tests pass

## User Decisions

| Question | Decision | Notes |
|----------|----------|-------|
| Auth method? | JWT | Using existing library |
| Token expiry handling? | Return 401 | No refresh token |
| Routes to protect? | /api/users/* | Excluding public |

## Agent Findings

### Patterns
- `src/middleware/logging.ts:10-25` - Middleware pattern
- `src/middleware/error.ts:5-15` - Error handling pattern
- `src/utils/jwt.ts:verify()` - Token verification function (existing)

### Structure
- Middleware: `src/middleware/`
- Config: `src/config/`
- Router: `src/routes/`

### Project Commands
- Type check: `npm run type-check`
- Lint: `npm run lint`
- Test: `npm test`

## Open Questions

### Critical (Must resolve before Plan)
- (None)

### Nice-to-have (Can decide later)
- [ ] Token expiration time value?

## Direction

### Approach
- Create auth.ts following existing logging.ts middleware pattern
- Connect to Express router as middleware chain
- Use existing jwt.ts verify() function

### Work Breakdown (Draft)
1. Create JWT config file → outputs: `config_path`
2. Implement auth middleware → depends on: config file
3. Connect middleware to router → depends on: middleware
4. Verification → depends on: all complete
```
