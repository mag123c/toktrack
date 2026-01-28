# toktrack

**English** | [한국어](README.ko.md)

Ultra-fast AI CLI token usage tracker. Built with Rust + simd-json + ratatui.

<!-- TODO: Add screenshot -->

## Features

- **Blazing Fast** - simd-json based parsing (~2 GiB/s throughput)
- **Beautiful TUI** - 4 views (Overview, Models, Daily, Stats)
- **CLI Commands** - `daily`, `stats` with JSON output support
- **Data Preservation** - Automatic backup before 30-day deletion

## Installation

**Recommended (No Rust required):**
```bash
npx toktrack
# or
bunx toktrack
```

**Other options:**
```bash
# Rust developers
cargo install toktrack

# From source
cargo install --git https://github.com/jaehojang/toktrack

# Direct download
# → github.com/jaehojang/toktrack/releases
```

## Usage

### TUI Mode (Default)

```bash
toktrack
```

### CLI Commands

```bash
# Daily usage report
toktrack daily
toktrack daily --json

# Statistics
toktrack stats
toktrack stats --json

# Manual backup
toktrack backup
```

### Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `1-4` | Switch tabs directly |
| `Tab` / `Shift+Tab` | Next / Previous tab |
| `j` / `k` or `↑` / `↓` | Scroll up / down |
| `?` | Toggle help |
| `q` | Quit |

## Supported AI CLIs

| CLI | Status | Data Location |
|-----|--------|---------------|
| Claude Code | ✅ MVP | `~/.claude/projects/` |
| OpenCode | 🔜 v1.1 | `~/.local/share/opencode/` |
| Codex CLI | 🔜 v1.2 | `~/.codex/sessions/` |
| Gemini CLI | 🔜 v1.3 | `~/.gemini/tmp/*/chats/` |

## Benchmarks

| Mode | Throughput |
|------|------------|
| Single file (simd-json) | ~1.0 GiB/s |
| Parallel (rayon) | ~2.0 GiB/s |

**Real-world performance** (2,000+ files / 2.9GB data):

| Tool | Time |
|------|------|
| ccusage (Node.js) | ~20s |
| ccusage (cached) | ~7s |
| **toktrack** | **< 500ms** |

## Data Preservation

Claude Code and Gemini CLI delete session data after 30 days by default.

toktrack automatically backs up your data to `~/.toktrack/cache/` on first run.

To disable auto-deletion in Claude Code:
```json
// ~/.claude/settings.json
{
  "cleanupPeriodDays": 9999999999
}
```

## Configuration

```toml
# ~/.toktrack/config.toml

[cache]
enabled = true
backup_on_start = true

[tui]
theme = "green"  # green, teal, blue, pink, purple, orange
```

## Architecture

```
toktrack/
├── src/
│   ├── parser/      # simd-json JSONL parsing
│   ├── services/    # Aggregation & file watching
│   ├── tui/         # ratatui-based terminal UI
│   └── cli/         # Command-line interface
```

## Development

```bash
# Run all checks (fmt + clippy + test)
make check

# Run tests
cargo test

# Run with watch
cargo watch -x test

# Benchmark
cargo bench

# Build release
cargo build --release
```

## License

MIT
