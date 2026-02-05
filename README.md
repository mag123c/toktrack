<p align="center">
  <img src="assets/logo.png" width="120" alt="toktrack logo">
</p>

<h1 align="center">toktrack</h1>

<p align="center">
  <a href="https://github.com/mag123c/toktrack/actions/workflows/ci.yml"><img src="https://github.com/mag123c/toktrack/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://www.npmjs.com/package/toktrack"><img src="https://img.shields.io/npm/v/toktrack" alt="npm"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-yellow.svg" alt="License: MIT"></a>
</p>

**English** | [한국어](README.ko.md)

> **⚠️ Did you know?** Claude Code **deletes your session data after 30 days** by default. Once deleted, your token usage and cost history are gone forever — unless you preserve them.

Track token usage and costs across **all your AI coding CLIs** — Claude Code, Codex CLI, and Gemini CLI — in one dashboard.

Built with Rust for ultra-fast performance (simd-json + rayon parallel processing).

![toktrack overview](assets/demo.gif)

## Why toktrack?

| Problem | Solution |
|---------|----------|
| 🐌 **Existing tools are slow** — 40+ seconds on large datasets | ⚡ **1000x faster** — cached queries in ~0.04s |
| 🗑️ **Claude Code deletes data after 30 days** — your cost history disappears | 💾 **Persistent cache** — history survives even after CLI deletes files |
| 📊 **No unified view** — each CLI has separate data | 🎯 **One dashboard** — Claude Code, Codex CLI, Gemini CLI in one place |

### Performance Comparison

```
Dataset: 2,000+ JSONL files, 3.4 GB total

Existing tools:     ████████████████████████████████████████ 40s+
toktrack (cold):    █ ~1s (first run)
toktrack (cached):  ▏ ~0.04s (daily use)

                    └── up to 1000x faster
```

## Features

- **Ultra-Fast Parsing** — simd-json + rayon parallel processing (~3 GiB/s throughput)
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

| Tool | Time | Speedup |
|------|------|---------|
| Existing tools | 40s+ | baseline |
| **toktrack** (cold) | **~1.0s** | **40x faster** |
| **toktrack** (cached) | **~0.04s** | **1000x faster** |

> Measured on Apple Silicon with 2,000+ JSONL files (3.4 GB).
>
> **Why so fast?** SIMD JSON parsing ([simd-json](https://github.com/simd-lite/simd-json)) + parallel processing ([rayon](https://github.com/rayon-rs/rayon)) = ~3 GiB/s throughput.

## Data Preservation

> **The Problem:** You've been using Claude Code for 3 months, spending hundreds of dollars. One day you want to check your total spending — but Claude Code already deleted your session files from 2 months ago. That cost data is gone forever.

**toktrack solves this.** It caches daily cost summaries independently, so your usage history survives even after the CLI deletes the original files.

### CLI Data Retention Policies (The Hidden Risk)

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

![architecture](assets/architecture.png)

**Cold path** (first run): Full glob scan → parallel SIMD parsing → build cache → aggregate.

**Warm path** (cached): Load cached summaries → parse only recent files (24h mtime filter) → merge → aggregate.

> **Deep Dive:** [I Rewrote a Node.js CLI in Rust — It Went from 43s to 1s](https://medium.com/@diehreo/i-rewrote-a-node-js-cli-in-rust-it-went-from-43s-to-1s-c13e38e7fe88) | [한국어](https://mag1c.tistory.com/601)

## Development

```bash
make check    # fmt + clippy + test (pre-commit)
cargo test    # Run tests
cargo bench   # Benchmarks
```

## Roadmap

- [ ] **OpenCode support**

## Contributing

Issues and PRs welcome!

```bash
make check  # Run before PR
```

## License

MIT
