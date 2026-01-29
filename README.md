# toktrack

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

**English** | [한국어](README.ko.md)

Ultra-fast AI CLI token usage tracker. Built with Rust + simd-json + ratatui.

![toktrack overview](demo.gif)

## Why toktrack?

| Tool | Time (2,000+ files / 3GB) | |
|------|---------------------------|---|
| ccusage (Node.js) | ~43s | 1x |
| **toktrack (Rust)** | **~3s** | **15x faster** |

> I hit ccusage's performance wall. After maxing out Node.js optimizations, I rewrote it in Rust.

## Features

- **Blazing Fast** - simd-json based parsing (~2 GiB/s throughput)
- **TUI Dashboard** - 4 views (Overview, Models, Daily, Stats) with daily/weekly/monthly breakdown
- **CLI Commands** - `daily`, `weekly`, `monthly`, `stats` with JSON output support
- **Data Preservation** - Cached summaries for fast repeated access

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
cargo install --git https://github.com/mag123c/toktrack

# Direct download
# → github.com/mag123c/toktrack/releases
```

## Usage

### TUI Mode (Default)

```bash
toktrack
```

### CLI Commands

```bash
# Open TUI at specific tab
toktrack daily     # Daily tab (daily view)
toktrack weekly    # Daily tab (weekly view)
toktrack monthly   # Daily tab (monthly view)
toktrack stats     # Stats tab

# JSON output (for scripting)
toktrack daily --json
toktrack weekly --json
toktrack monthly --json
toktrack stats --json
```

### Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `1-4` | Switch tabs directly |
| `Tab` / `Shift+Tab` | Next / Previous tab |
| `j` / `k` or `↑` / `↓` | Scroll up / down |
| `d` / `w` / `m` | Daily / Weekly / Monthly view (Daily tab) |
| `?` | Toggle help |
| `q` | Quit |

## Supported AI CLIs

| CLI | Status | Data Location |
|-----|--------|---------------|
| Claude Code | ✅ | `~/.claude/projects/` |
| Codex CLI | ✅ | `~/.codex/sessions/` |
| Gemini CLI | ✅ | `~/.gemini/tmp/*/chats/` |
| OpenCode | 🔜 | `~/.local/share/opencode/` |

## Benchmarks

| Mode | Throughput |
|------|------------|
| Single file (simd-json) | ~1.0 GiB/s |
| Parallel (rayon) | ~2.0 GiB/s |

**Real-world performance** (2,000+ files / 3GB data):

| Tool | Time | |
|------|------|---|
| ccusage (Node.js) | ~43s | 1x |
| **toktrack** | **~3s** | **15x faster** |

## Data Preservation

Claude Code and Gemini CLI delete session data after 30 days by default.

toktrack caches daily summaries to `~/.toktrack/cache/` for fast repeated access.

To disable auto-deletion in Claude Code:
```json
// ~/.claude/settings.json
{
  "cleanupPeriodDays": 9999999999
}
```

## Development

```bash
make check    # fmt + clippy + test (pre-commit)
cargo test    # Run tests
cargo bench   # Benchmarks
```

## Roadmap

- [ ] **Performance** - Target sub-1s for 3GB+ datasets
- [ ] **OpenCode support**

## Contributing

Issues and PRs welcome!

```bash
make check  # Run before PR
```

## License

MIT
