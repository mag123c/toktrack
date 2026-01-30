# Plan Template

> Reference document for plan file structure. Designed for Orchestrator-Worker pattern where each TODO is executed by an isolated Worker agent.

**Schema Version**: 1.1

---

## Architecture Overview

```
Orchestrator (reads full PLAN)
    │
    ├── Worker 1 → TODO 1 only (isolated)
    ├── Worker 2 → TODO 2 only (isolated)
    │   ...
    └── Worker Final → Verification (read-only)
```

**Key Principles**:
- Each TODO must be **self-contained** (Worker sees only its TODO)
- **Inputs/Outputs** enable dependency between TODOs (with explicit types)
- Orchestrator handles **substitution**: `${todo-1.outputs.config_path}` → actual value
- Orchestrator handles **all commits** (Workers do NOT commit)
- **Verification** runs once after all TODOs complete (read-only)

---

## Required Sections

### 1. Header

```markdown
# {Plan Title}

> Brief description of what this plan accomplishes
```

### 2. Context

```markdown
## Context

### Original Request
[User's initial description]

### Interview Summary
**Key Discussions**:
- [Point 1]: [User's decision/preference]
- [Point 2]: [Agreed approach]

**Research Findings**:
- [Finding 1]: [Implication]
```

### 3. Work Objectives

```markdown
## Work Objectives

### Core Objective
[1-2 sentences: what we're achieving]

### Concrete Deliverables
- [Exact file/endpoint/feature]

### Definition of Done
- [ ] [Verifiable condition with command]

### Must NOT Do (Guardrails)
- [Explicit exclusion]
- [Scope boundary]
```

---

## Orchestrator Section

> **For Orchestrator only** - Workers do not see this section.

### 4. Task Flow

```markdown
## Task Flow

```
TODO-1 → TODO-2 → TODO-Final
```
```

### 5. Dependency Graph

```markdown
## Dependency Graph

| TODO | Requires (Inputs) | Produces (Outputs) | Type |
|------|-------------------|-------------------|------|
| 1 | - | `config_path` (file) | work |
| 2 | `todo-1.config_path` | `api_module` (file) | work |
| Final | all outputs | - | verification |
```

### 6. Parallelization

```markdown
## Parallelization

| Group | TODOs | Reason |
|-------|-------|--------|
| - | - | (define if parallel tasks exist) |
```

### 7. Commit Strategy

> **Orchestrator commits on behalf of Workers** - Workers do NOT touch git.

```markdown
## Commit Strategy

| After TODO | Message | Files | Condition |
|------------|---------|-------|-----------|
| 1 | `chore(setup): initialize config` | `config/*` | always |
| 2 | `feat(api): add main module` | `src/api/*` | always |

> **Note**: No commit after Final (Verification is read-only). Final cleanup commit only if Orchestrator detects uncommitted changes before verification.
```

### 8. Error Handling

```markdown
## Error Handling

### Failure Categories

| Category | Examples | Detection Pattern |
|----------|----------|-------------------|
| `env_error` | API key missing, permission denied, network timeout | `/EACCES\|ECONNREFUSED\|timeout\|401\|403/i` |
| `code_error` | Type error, lint failure, test failure | `/TypeError\|SyntaxError\|lint\|test failed/i` |
| `unknown` | Unclassifiable errors | Default fallback |

### Failure Handling Flow

| Scenario | Action |
|----------|--------|
| work fails | Retry up to 2 times → Analyze → (see below) |
| verification fails | Analyze immediately (no retry) → (see below) |
| Worker times out | Halt and report |
| Missing Input | Skip dependent TODOs, halt |

### After Analyze

| Category | Action |
|----------|--------|
| `env_error` | Halt + log to `issues.md` |
| `code_error` | Create Fix Task (depth=1 limit) |
| `unknown` | Halt + log to `issues.md` |

### Fix Task Rules

- Fix Task type is always `work`
- Fix Task failure → Halt (no further Fix Task creation)
- Max depth = 1 (prevents infinite loop)
```

### 9. Runtime Contract

```markdown
## Runtime Contract

| Aspect | Specification |
|--------|---------------|
| Working Directory | Repository root |
| Network Access | Allowed (for API calls, downloads) |
| Package Install | Denied (use existing deps only) |
| File Access | Repository only (no system files) |
| Max Execution Time | 5 minutes per TODO |
| Git Operations | Denied (Orchestrator handles) |
```

---

## TODO Section

> **For Workers** - Each Worker receives only its assigned TODO.

### 10. TODOs

```markdown
## TODOs

### [ ] TODO 1: {Task Title}

**Type**: work

**Required Tools**: (none)

**Inputs**: (none - first task)

**Outputs**:
- `config_path` (file): `./config/app.json` - Application configuration file

**Steps**:
- [ ] Create config directory structure
- [ ] Generate initial configuration file
- [ ] Validate config schema

**Must NOT do**:
- Do not modify existing configs
- Do not add external dependencies
- Do not run git commands

**References**:
- `src/types/config.ts:10-30` - Config type definitions
- `docs/config-spec.md` - Configuration requirements

**Acceptance Criteria**:

*Functional:*
- [ ] File exists: `./config/app.json`
- [ ] Config contains required fields: `name`, `version`

*Static:*
- [ ] `cat ./config/app.json` → Valid JSON (parseable)

*Runtime:*
- [ ] (no tests for config-only task)

---

### [ ] TODO 2: {Task Title}

**Type**: work

**Required Tools**: (none)

**Inputs**:
- `config_path` (file): `${todo-1.outputs.config_path}` - Configuration file from TODO 1

**Outputs**:
- `api_module` (file): `src/api/index.ts` - Main API module

**Steps**:
- [ ] Read configuration from `${config_path}`
- [ ] Create API module structure at `src/api/`
- [ ] Implement endpoints based on config
- [ ] Export module

**Must NOT do**:
- Do not hardcode config values (read from input)
- Do not modify the input config file
- Do not install new packages
- Do not run git commands

**References**:
- `src/api/template.ts:1-50` - API pattern to follow

**Acceptance Criteria**:

*Functional:*
- [ ] File exists: `src/api/index.ts`
- [ ] Module exports `api` function

*Static:*
- [ ] `tsc --noEmit src/api/index.ts` → exit 0

*Runtime:*
- [ ] `npm test -- api.test.ts` → passes

---

### [ ] TODO Final: Verification

**Type**: verification (read-only)

**Required Tools**: (from DRAFT's Agent Findings > Project Commands)

**Inputs**:
- `config_path` (file): `${todo-1.outputs.config_path}`
- `api_module` (file): `${todo-2.outputs.api_module}`

**Outputs**: (none)

**Steps**:
- [ ] Run type check (if applicable)
- [ ] Run lint (if applicable)
- [ ] Run tests
- [ ] Verify all deliverables exist

**Must NOT do**:
- Do not modify any files
- Do not add new features
- Do not fix errors (report only)
- Do not run git commands

**Acceptance Criteria**:

> Use commands from DRAFT's Agent Findings > Project Commands

*Functional:*
- [ ] All deliverables from Work Objectives exist
- [ ] `config_path` file exists and is valid
- [ ] `api_module` file exists and exports expected functions

*Static:*
- [ ] `{type-check-command}` → exit 0 (e.g., `tsc --noEmit`, `mypy .`, `go vet ./...`)
- [ ] `{lint-command}` → no errors (e.g., `eslint .`, `ruff check .`, `golangci-lint run`)

*Runtime:*
- [ ] `{test-command}` → all pass (e.g., `npm test`, `pytest`, `go test ./...`)
```

---

## Worker Output Protocol

> How Workers report results to Orchestrator.

Worker outputs **JSON** in a ```json code block. The PostToolUse hook re-executes each `command` in `acceptance_criteria` to verify Worker's report.

### Output Schema

```json
{
  "outputs": {
    "config_path": "./config/app.json",
    "exported_name": "configLoader"
  },
  "acceptance_criteria": [
    {
      "id": "file_exists",
      "category": "functional",
      "description": "File exists: ./config/app.json",
      "command": "test -f ./config/app.json",
      "status": "PASS"
    },
    {
      "id": "exports_function",
      "category": "functional",
      "description": "File exports configLoader function",
      "command": "grep -q 'export.*configLoader' ./config/app.json",
      "status": "PASS"
    },
    {
      "id": "tsc_check",
      "category": "static",
      "description": "tsc --noEmit passes",
      "command": "tsc --noEmit ./config/app.json",
      "status": "PASS"
    },
    {
      "id": "eslint_check",
      "category": "static",
      "description": "eslint passes",
      "command": "eslint ./config/app.json",
      "status": "FAIL",
      "reason": "Unexpected console.log statement (line 42)"
    },
    {
      "id": "test_config",
      "category": "runtime",
      "description": "Config tests pass",
      "command": "npm test -- config.test.ts",
      "status": "SKIP",
      "reason": "No test file exists"
    }
  ],
  "learnings": ["Config uses JSON schema validation"],
  "issues": ["Existing type definitions incomplete (out of scope)"],
  "decisions": ["Used standard JSON format over YAML for simplicity"]
}
```

### Field Specifications

| Field | Required | Description |
|-------|----------|-------------|
| `outputs` | ✅ | Key-value pairs matching TODO's **Outputs** field |
| `acceptance_criteria` | ✅ | Array of verification items (see below) |
| `learnings` | ❌ | Patterns discovered and **applied** (tips for next Worker) |
| `issues` | ❌ | Problems found but **not resolved** (out of scope) |
| `decisions` | ❌ | Decisions made and why |

### acceptance_criteria Item Structure

| Field | Required | Description |
|-------|----------|-------------|
| `id` | ✅ | Unique identifier (e.g., `tsc_check`, `test_auth`) |
| `category` | ✅ | `functional` / `static` / `runtime` / `cleanup` |
| `description` | ✅ | Human-readable description |
| `command` | ✅ | Re-executable shell command for verification |
| `status` | ✅ | `PASS` / `FAIL` / `SKIP` |
| `reason` | ❌ | Required when status is `FAIL` or `SKIP` |

### Category Requirements

| Category | Required | What to Verify |
|----------|----------|----------------|
| `functional` | ✅ | Feature works (file exists, exports correct, behavior correct) |
| `static` | ✅ | `tsc --noEmit`, `eslint` pass for modified files |
| `runtime` | ✅ | Related tests pass (SKIP if no tests) |
| `cleanup` | ❌ | Unused imports/files removed (only if specified in TODO) |

**Completion Rule**: All required categories must have all items `PASS` or `SKIP`

### Verification Flow

```
Worker completes TODO
        ↓
Worker outputs JSON with acceptance_criteria
        ↓
PostToolUse Hook triggers (dev-worker-verify.sh)
        ↓
Hook re-executes each command in acceptance_criteria
        ↓
Hook outputs: VERIFIED (all pass) or FAILED (mismatch detected)
        ↓
Orchestrator receives Hook result
        ↓
[VERIFIED] → Mark TODO complete    [FAILED] → Retry Worker
```

⚠️ **Hook re-verifies Worker's claims.** Worker saying "PASS" is not trusted—the command is re-executed.

---

## Field Specifications

### Type Field

Declares the nature of the TODO.

| Type | Description | Can Modify Files? |
|------|-------------|-------------------|
| `work` | Implementation task | Yes |
| `verification` | Quality gate (read-only) | No |

### Required Tools Field

Explicit declaration of environment dependencies.

```markdown
**Required Tools**: `npm`, `jq`, `curl`
```

If a tool is not available, Worker should fail immediately with clear error.

### Inputs Field

Declares dependencies on outputs from previous TODOs. **Keys must match output names exactly.**

```markdown
**Inputs**:
- `{output_name}` ({type}): `${todo-N.outputs.output_name}` - Description
```

**Types**:
- `file` - File path
- `json` - JSON object/value
- `string` - Plain string
- `list` - Array of values

**Examples**:
```markdown
**Inputs**:
- `config_path` (file): `${todo-1.outputs.config_path}` - Config file path
- `api_spec` (json): `${todo-1.outputs.api_spec}` - API specification object
```

### Outputs Field

Declares what this TODO produces. **Orchestrator collects these after Worker completes.**

```markdown
**Outputs**:
- `{output_name}` ({type}): `{value}` - Description
```

**Examples**:
```markdown
**Outputs**:
- `config_path` (file): `./config/app.json` - Generated config file
- `test_report` (json): `coverage/report.json` - Test coverage data
- `files_created` (list): `["src/a.ts", "src/b.ts"]` - List of created files
```

### Steps Field

Actionable items the Worker must complete. **All items must be checkboxes.**

```markdown
**Steps**:
- [ ] Clear action 1
- [ ] Clear action 2
- [ ] Clear action 3
```

### Acceptance Criteria Field

Verifiable conditions that prove the TODO is complete. **All required categories must pass for completion.**

**Categories:**

| Category | Required | Description |
|----------|----------|-------------|
| *Functional* | ✅ | Feature works as expected (business logic) |
| *Static* | ✅ | Type check, lint pass for modified files |
| *Runtime* | ✅ | Related tests pass |
| *Cleanup* | ❌ | Unused imports/files removed (when applicable) |

**Worker Completion Rule**: `Functional ✅ AND Static ✅ AND Runtime ✅ (AND Cleanup ✅ if specified)`

```markdown
**Acceptance Criteria**:

*Functional:*
- [ ] Feature behavior check (e.g., "Returns 401 without token")
- [ ] Output exists and is valid

*Static:*
- [ ] `tsc --noEmit` passes for modified files
- [ ] `eslint` passes for modified files

*Runtime:*
- [ ] `npm test -- <related-test>` passes

*Cleanup:* (optional)
- [ ] No unused imports in modified files
- [ ] Removed deprecated files listed in Outputs
```

---

## Worker Execution Flow

Each Worker follows this flow for its assigned TODO:

```
1. Validate Environment
   └─ Check Required Tools are available

2. Receive TODO
   └─ Orchestrator has already substituted ${...} references

3. Validate Inputs
   └─ Check all input files/values exist

4. Execute Steps
   ├─ Work through each checkbox
   └─ Mark completed as done

5. Verify Acceptance Criteria (ALL required categories must pass)
   ├─ Functional: Feature works as specified
   ├─ Static: tsc, eslint pass for modified files
   ├─ Runtime: Related tests pass
   └─ Cleanup (if specified): Unused code removed

6. Report Results
   └─ Output JSON to stdout (success or failure)

Completion Rule: Functional ✅ AND Static ✅ AND Runtime ✅ (AND Cleanup ✅)
(Worker does NOT commit - Orchestrator handles git)
```

---

## Orchestrator Execution Flow

```
1. Parse PLAN
   └─ Validate Dependency Graph consistency

2. For each TODO (respecting dependencies):
   ├─ Substitute ${...} references with actual values
   ├─ Dispatch to Worker
   ├─ Collect Worker output (stdout JSON)
   ├─ If success: store outputs, commit if specified
   └─ If failure: apply Error Handling rules

3. After all work TODOs:
   └─ Dispatch TODO Final (verification)

4. If verification passes:
   └─ Report success

5. If verification fails:
   └─ Report failures (do NOT auto-fix)
```

---

## Example: Complete TODO

```markdown
### [ ] TODO 2: Add authentication middleware

**Type**: work

**Required Tools**: (none)

**Inputs**:
- `config_path` (file): `${todo-1.outputs.config_path}` - JWT configuration

**Outputs**:
- `middleware_path` (file): `src/middleware/auth.ts` - Auth middleware module

**Steps**:
- [ ] Read JWT settings from `${config_path}`
- [ ] Create `src/middleware/auth.ts`
- [ ] Implement token validation using existing pattern from references
- [ ] Add middleware to Express router chain
- [ ] Export middleware function

**Must NOT do**:
- Do not modify existing auth logic in other files
- Do not add new npm dependencies
- Do not change the JWT secret handling
- Do not run git commands

**References**:
- `src/middleware/logging.ts:10-25` - Middleware pattern to follow
- `src/utils/jwt.ts:verify()` - Use this for token validation

**Acceptance Criteria**:

*Functional:*
- [ ] File exists: `src/middleware/auth.ts`
- [ ] File exports `authMiddleware` function
- [ ] Request without token → 401 Unauthorized
- [ ] Request with valid token → Passes to next handler

*Static:*
- [ ] `tsc --noEmit src/middleware/auth.ts` → exit 0
- [ ] `eslint src/middleware/auth.ts` → no errors

*Runtime:*
- [ ] `npm test -- auth.test.ts` → passes
```

---

## Example: TODO Final

> This example uses Node.js commands. Replace with project-specific commands from DRAFT's Agent Findings > Project Commands.

```markdown
### [ ] TODO Final: Verification

**Type**: verification (read-only)

**Required Tools**: (per project - e.g., npm, cargo, go, pytest)

**Inputs**:
- `middleware_path` (file): `${todo-2.outputs.middleware_path}`
- `routes_path` (file): `${todo-3.outputs.routes_path}`

**Outputs**: (none)

**Steps**:
- [ ] Run type check (if applicable)
- [ ] Run lint (if applicable)
- [ ] Run tests
- [ ] Verify middleware is imported in routes file

**Must NOT do**:
- Do not modify any files
- Do not add new features
- Do not fix errors (report only)
- Do not run git commands

**Acceptance Criteria**:

*Functional:*
- [ ] All deliverables from Work Objectives exist
- [ ] `middleware_path` file exists
- [ ] `routes_path` file imports middleware

*Static:*
- [ ] Type check passes (e.g., `tsc --noEmit`, `mypy .`, `go vet ./...`)
- [ ] Lint passes (e.g., `eslint .`, `ruff check .`, `golangci-lint run`)

*Runtime:*
- [ ] Tests pass (e.g., `npm test`, `pytest`, `go test ./...`, `cargo test`)
```
