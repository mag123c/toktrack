# toktrack

[![CI](https://github.com/mag123c/toktrack/actions/workflows/ci.yml/badge.svg)](https://github.com/mag123c/toktrack/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/toktrack)](https://crates.io/crates/toktrack)
[![npm](https://img.shields.io/npm/v/toktrack)](https://www.npmjs.com/package/toktrack)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

**English** | [한국어](README.ko.md)

Ultra-fast AI CLI token usage tracker. Built with Rust + simd-json + ratatui.

![toktrack overview](demo.gif)

> Scanning 2,000+ session files (3 GB) took over 40 seconds with existing tools — toktrack does it in ~1 second.

## Features

- **Ultra-Fast Parsing** — simd-json + rayon parallel processing (~2 GiB/s throughput)
- **TUI Dashboard** — 4 views (Overview, Models, Daily, Stats) with daily/weekly/monthly breakdown
- **CLI Commands** — `daily`, `weekly`, `monthly`, `stats` with JSON output support
- **Multi-CLI Support** — Claude Code, Codex CLI, Gemini CLI in one place
- **Data Preservation** — Cached daily summaries survive CLI data deletion

## Installation

### npx (Recommended)

No Rust toolchain required. Downloads the correct binary for your platform automatically.

```bash
npx toktrack
# or
bunx toktrack
```

### Cargo

```bash
cargo install toktrack
```

### From Source

```bash
cargo install --git https://github.com/mag123c/toktrack
```

### Pre-built Binaries

Download from [GitHub Releases](https://github.com/mag123c/toktrack/releases).

| Platform | Architecture |
|----------|-------------|
| macOS | x64, ARM64 |
| Linux | x64, ARM64 |
| Windows | x64 |

## Quick Start

```bash
# Launch TUI dashboard
npx toktrack

# Get today's cost in JSON
npx toktrack daily --json

# Monthly summary
npx toktrack monthly --json
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

## Performance

| Mode | Time |
|------|------|
| Cold start (no cache) | **~1.2s** |
| Warm start (cached) | **~0.05s** |

> Measured on Apple Silicon (9,000+ files / 3.4 GB).

## Data Preservation

AI CLIs delete or rotate session data on their own schedules. toktrack caches daily cost summaries independently, so your usage history survives even after the original data is gone.

### CLI Data Retention Policies

| CLI | Default Retention | Policy |
|-----|-------------------|--------|
| Claude Code | **30 days** | `cleanupPeriodDays` (default: 30) |
| Gemini CLI | Unlimited | opt-in `sessionRetention` |
| Codex CLI | Unlimited | size-cap only (`max_bytes`) |

### toktrack Cache Structure

```
~/.toktrack/
├── cache/
│   ├── claude-code_daily.json   # Daily cost summaries
│   ├── codex_daily.json
│   └── gemini_daily.json
└── pricing.json                 # LiteLLM pricing (1h TTL)
```

Past dates in each `*_daily.json` are **immutable** — once a day is summarized, the cached result is never modified. Only the current day is recomputed on each run. This means even if Claude Code deletes session files after 30 days, your cost history remains intact in the cache.

### Disable Claude Code Auto-Deletion

```json
// ~/.claude/settings.json
{
  "cleanupPeriodDays": 9999999999
}
```

### Reset Cache

```bash
rm -rf ~/.toktrack/cache/
```

The next run will rebuild the cache from available session data.

## How It Works

```
┌─────────────────────────────────────────────────┐
│                   CLI / TUI                     │
└──────────────────────┬──────────────────────────┘
                       │
              ┌────────▼────────┐
              │   Aggregator    │
              └────────┬────────┘
                       │
         ┌─────────────┼─────────────┐
         ▼             ▼             ▼
   ┌──────────┐  ┌──────────┐  ┌──────────┐
   │  Claude  │  │  Codex   │  │  Gemini  │
   │  Parser  │  │  Parser  │  │  Parser  │
   └────┬─────┘  └────┬─────┘  └────┬─────┘
        │              │              │
        ▼              ▼              ▼
   simd-json     simd-json      simd-json
   + rayon       + rayon        + rayon
        │              │              │
        └──────────────┼──────────────┘
                       ▼
              ┌────────────────┐
              │     Cache      │
              │ ~/.toktrack/   │
              └────────────────┘
```

**Cold path** (first run): Full glob scan → parallel SIMD parsing → build cache → aggregate.

**Warm path** (cached): Load cached summaries → parse only recent files (24h mtime filter) → merge → aggregate.

## Development

```bash
make check    # fmt + clippy + test (pre-commit)
cargo test    # Run tests
cargo bench   # Benchmarks
```

## Roadmap

- [ ] **Performance** — Target sub-1s cold start for 3GB+ datasets
- [ ] **OpenCode support**

## Contributing

Issues and PRs welcome!

```bash
make check  # Run before PR
```

## License

MIT
