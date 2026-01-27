# Architecture

## Layers
```
TUI[ratatui] → CLI[clap] → Services → Parsers[trait] → Cache
```

## Paths
- `src/tui/` - TUI (app.rs, widgets/)
- `src/cli/` - CLI commands
- `src/services/` - aggregator, pricing, cache
- `src/parsers/` - CLIParser trait + impls
- `src/types/` - UsageEntry, errors

## TUI Widgets
| Widget | Purpose |
|--------|---------|
| `app.rs` | AppState enum, Tab state, event loop |
| `widgets/spinner.rs` | Loading animation |
| `widgets/heatmap.rs` | 52-week heatmap (2x2 blocks, 14 rows, responsive) |
| `widgets/overview.rs` | Overview layout (hero stat, sub-stats, keybindings) |
| `widgets/legend.rs` | Heatmap intensity legend |
| `widgets/tabs.rs` | Tab enum, TabBar widget |

## Core Trait
```rust
trait CLIParser: Send + Sync {
    fn name(&self) -> &str;
    fn data_dir(&self) -> PathBuf;
    fn file_pattern(&self) -> &str;  // e.g., "**/*.jsonl"
    fn parse_file(&self, path: &Path) -> Result<Vec<UsageEntry>>;
    fn parse_all(&self) -> Result<Vec<UsageEntry>>;  // rayon parallel
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

## Parser Optimizations
| Technique | Description | Throughput |
|-----------|-------------|------------|
| Zero-copy serde | `&'a str` borrowed, no String alloc | ~1.0 GiB/s |
| In-place buffer | `&mut [u8]` to simd-json | |
| SIMD parsing | simd-json AVX2/NEON | |
| rayon parallel | `parse_all()` file-level parallel | ~2.0 GiB/s |

## Cache (~/.toktrack/)
```
cache/
├── {cli}_daily.json  # DailySummary cache (past dates immutable)
└── pricing.json      # LiteLLM 1h TTL
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
