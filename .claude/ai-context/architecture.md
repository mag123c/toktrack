# Architecture

## Layers
```
TUI[ratatui] → CLI[clap] → Services → Parsers[trait] → Cache
```

## Paths
- `src/tui/` - TUI (app.rs, views/, components/)
- `src/cli/` - CLI commands
- `src/services/` - aggregator, pricing, cache
- `src/parsers/` - CLIParser trait + impls
- `src/cache/` - backup + perf cache
- `src/types/` - UsageEntry, errors

## Core Trait
```rust
trait CLIParser: Send + Sync {
    fn name(&self) -> &str;
    fn data_dir(&self) -> PathBuf;
    fn file_pattern(&self) -> &str;  // e.g., "**/*.jsonl"
    fn parse_file(&self, path: &Path) -> Result<Vec<UsageEntry>>;
}
```

## Implementations
| Version | Parser | Format |
|---------|--------|--------|
| MVP | ClaudeCodeParser | JSONL |
| v1.1 | OpenCodeParser | JSON |
| v1.2 | CodexParser | JSONL |
| v1.3 | GeminiParser | JSON |

## Data Flow
```
1. Scan data_dir (glob)
2. Parse files (simd-json, parallel)
3. Aggregate (daily/model/total)
4. Calculate cost (LiteLLM pricing)
5. Render TUI / Output JSON
```

## Cache (~/.toktrack/)
```
cache/
├── claude/      # Backup to prevent 30-day deletion
└── pricing.json # LiteLLM 1h TTL
```

## Deps
```toml
simd-json, ratatui, crossterm, clap, rayon, chrono, directories, serde
dev: insta, criterion
```

## Dev Workflow
```
/clarify → Plan Mode → /implement (TDD) → /verify → /review → /wrap
```

## Skills
| Skill | Purpose |
|-------|---------|
| /clarify | Requirements → auto Plan Mode |
| /implement | TDD (RED→GREEN→REFACTOR) |
| /verify | Self-healing test/clippy/fmt |
| /review | Negative perspective review |
| /wrap | Session wrap-up |

## Pre-commit
```bash
make check  # fmt + clippy + test
```
