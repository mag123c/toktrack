# toktrack

Ultra-fast AI CLI token usage tracker. Rust + simd-json + ratatui.

## Quick Start

```bash
cargo build --release
./target/release/toktrack
```

## Context

| File | Content |
|------|---------|
| [architecture.md](.claude/ai-context/architecture.md) | Layers, paths, traits, data flow |
| [conventions.md](.claude/ai-context/conventions.md) | Naming, TDD, error handling, commits |

## Dev Workflow (MUST FOLLOW)

```
/next → /clarify → Plan Mode → /implement → /verify → /review → /wrap
```

**Session start**: Run `/next` to see current progress and next task.

**Auto-transition rules**:
- **Plan provided directly** → skip clarify/plan, start with `/implement` skill immediately
- After plan approval → immediately run `/implement` skill
- After implement completes → auto-call `/verify`
- After verify passes → auto-call `/review`
- After review passes → auto-call `/wrap`
- `/wrap` must:
  - Update `ai-context/*.md` if architecture/conventions changed
  - Update `docs/planning/*.md` task checkboxes for completed work

See `.claude/skills/` for skill details.

## Commands

```bash
make check      # fmt + clippy + test (pre-commit)
make setup      # Configure git hooks
cargo test      # Run tests
cargo bench     # Benchmarks
```

## Commit Rules

```
{type}({scope}): {description}
```

types: `feat|fix|refactor|docs|test|chore|perf`
scopes: `parser|tui|services|cache|cli`
