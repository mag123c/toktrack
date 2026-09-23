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
| `.claude/ai-context/architecture.md` | Layers, paths, traits, data flow |
| `.claude/ai-context/conventions.md` | Naming, TDD, error handling, commits |

## Workflow

Read `.claude/ai-context/workflow.md` for the shared Claude/Codex procedure and repository gates. Clear direct requirements or clarify → implement → verify → review → wrap. Continue authorized work; no Plan Mode or invocation marker is required. Investigation-only requests end at investigation.

## Commands
```bash
make check    # fmt + clippy + test (pre-commit)
cargo test    # Run tests
cargo bench   # Benchmarks
```

## CI/CD
```
PR → CI (3 OS) → main → release-please → Release PR → 5 platform builds + npm
```

## Commit Rules
```
{type}({scope}): {description}
```
types: `feat|fix|refactor|docs|test|chore|perf`
scopes: `parser|tui|services|cache|cli`
