# toktrack

Ultra-fast AI CLI token usage tracker. Built with Rust + simd-json + ratatui.

## Quick Start

```bash
cargo build --release
./target/release/toktrack
```

## Architecture

```
┌─────────────┐
│     TUI     │  ratatui (4 views: Overview, Models, Daily, Stats)
└──────┬──────┘
       │
┌──────▼──────┐
│   Commands  │  CLI command handling (clap)
└──────┬──────┘
       │
┌──────▼──────┐
│   Services  │  Aggregation, pricing, caching
└──────┬──────┘
       │
┌──────▼──────┐
│   Parsers   │  trait CLIParser (Claude, OpenCode, Codex, Gemini)
└──────┬──────┘
       │
┌──────▼──────┐
│    Cache    │  ~/.toktrack/ (backup + perf cache)
└─────────────┘
```

## Directory Structure

```
src/
├── main.rs              # Entry point
├── cli/                 # CLI commands (clap)
├── tui/                 # TUI layer (ratatui)
│   ├── app.rs           # App state
│   ├── views/           # 4 views
│   └── components/      # Reusable components
├── services/            # Business logic
│   ├── aggregator.rs    # Token aggregation
│   ├── pricing.rs       # Cost calculation
│   └── mod.rs
├── parsers/             # CLI parsers (trait + impls)
│   ├── mod.rs           # CLIParser trait
│   ├── claude.rs        # Claude Code parser
│   └── ...
├── cache/               # Caching layer
└── types/               # Shared types
```

## Development Style: TDD

All implementations follow **Test-Driven Development**:

```
1. RED    - Write failing test first
2. GREEN  - Write minimal code to pass
3. REFACTOR - Clean up (tests must stay green)
```

### Running Tests

```bash
cargo test              # Unit tests
cargo test --release    # Release mode
cargo insta review      # Snapshot review
```

### Benchmarks

```bash
cargo bench             # criterion benchmarks
```

## Key Design Decisions

### 1. CLIParser Trait

Abstraction for multi-CLI support:

```rust
pub trait CLIParser: Send + Sync {
    fn name(&self) -> &str;
    fn data_dir(&self) -> PathBuf;
    fn parse_file(&self, path: &Path) -> Result<Vec<UsageEntry>>;
}
```

### 2. SIMD JSON Parsing

Maximum performance with simd-json:

```rust
let value: simd_json::BorrowedValue = simd_json::to_borrowed_value(&mut bytes)?;
```

### 3. Data Preservation

Auto-backup to prevent 30-day deletion:
- `~/.toktrack/cache/claude/` - Claude Code backup
- Enabled by default with user consent

## Commands

```bash
toktrack                # Launch TUI (default)
toktrack daily          # Daily usage report
toktrack daily --json   # JSON format
toktrack stats          # Statistics
toktrack backup         # Manual backup
```

## Configuration

```toml
# ~/.toktrack/config.toml

[cache]
enabled = true
backup_on_start = true

[tui]
theme = "green"  # green, halloween, teal, blue, pink, purple, orange
```

## AI Context

See `.claude/ai-context/` for detailed knowledge:

| File | Content |
|------|---------|
| `architecture.md` | Layer structure, dependencies |
| `conventions.md` | Naming, code style, TDD |

## Performance Goals

| Metric | Target | vs ccusage |
|--------|--------|------------|
| Cold Start | < 500ms | 14x+ |
| Warm Start | < 100ms | 70x+ |

## Dependencies

```toml
simd-json = "0.14"      # SIMD JSON parsing
ratatui = "0.29"        # TUI
crossterm = "0.28"      # Terminal control
clap = "4"              # CLI parsing
rayon = "1.10"          # Parallel processing
chrono = "0.4"          # Date/time
directories = "5"       # Cross-platform paths
serde = "1"             # Serialization
```

## Commit Rules

This project uses AI assistance (Claude). Include co-author in commits:

```
Co-Authored-By: Claude <noreply@anthropic.com>
```

## Contributing

TDD principles enforced:
1. PRs without tests will be rejected
2. All parsers must implement CLIParser trait
3. Snapshot tests for regression prevention
