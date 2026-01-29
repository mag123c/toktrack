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

## Dev Workflow

```
/next → /clarify → Plan Mode → /implement → /verify → /review → /wrap
```

Session start: Run `/next` to see current progress and next task.

## Skill Chain

| Completed | Next | Condition |
|-----------|------|-----------|
| Plan approved | `/implement` | Immediately |
| `/implement` | `/verify` | On completion |
| `/verify` | `/review` | On pass |
| `/review` | `/wrap` | On PASS |

Auto-chain: Each step proceeds without user confirmation.

Exceptions:
- `/verify` fail → fix and retry
- `/review` FAIL → return to `/implement`
- User explicitly requests stop

Plan provided directly → skip clarify/plan, start `/implement`

See `.claude/skills/` for skill details.

## Commands

```bash
make check      # fmt + clippy + test (pre-commit)
make setup      # Configure git hooks
cargo test      # Run tests
cargo bench     # Benchmarks
```

## CI/CD Workflow

```
PR → CI (3 OS) → main merge
                    ↓
            release-please (CI skip)
                    ↓
            Release PR → CI → auto-merge
                    ↓
        tag + workflow_dispatch → release.yml (5 builds + npm)
```

| Workflow | Trigger | Action |
|----------|---------|--------|
| `ci.yml` | PR, main push | fmt, clippy, test (3 OS) |
| `release-please.yml` | main push | Release PR, auto-merge, trigger release.yml |
| `release.yml` | workflow_dispatch (from release-please) | 5 platform builds, npm deploy |

Optimizations:
- release-please commits skip CI (paths-ignore)
- Release PR auto-merges on CI pass
- release-please triggers release.yml via workflow_dispatch (GITHUB_TOKEN tags can't trigger other workflows)

## Commit Rules

```
{type}({scope}): {description}
```

types: `feat|fix|refactor|docs|test|chore|perf`
scopes: `parser|tui|services|cache|cli`
