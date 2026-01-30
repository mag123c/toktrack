# toktrack

[![CI](https://github.com/mag123c/toktrack/actions/workflows/ci.yml/badge.svg)](https://github.com/mag123c/toktrack/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/toktrack)](https://crates.io/crates/toktrack)
[![npm](https://img.shields.io/npm/v/toktrack)](https://www.npmjs.com/package/toktrack)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://github.com/mag123c/toktrack/blob/main/LICENSE)

Ultra-fast AI CLI token usage tracker. Built with Rust + simd-json + ratatui.

![toktrack overview](https://raw.githubusercontent.com/mag123c/toktrack/main/demo.gif)

> Scanning 2,000+ session files (3 GB) took over 40 seconds with existing tools — toktrack does it in ~1 second.

## Installation

```bash
# No installation required
npx toktrack

# Or install globally
npm install -g toktrack
```

## Features

- **Ultra-Fast Parsing** — simd-json + rayon parallel processing (~2 GiB/s throughput)
- **TUI Dashboard** — 4 views (Overview, Models, Daily, Stats) with daily/weekly/monthly breakdown
- **Multi-CLI Support** — Claude Code, Codex CLI, Gemini CLI
- **CLI Commands** — `daily`, `weekly`, `monthly`, `stats` with JSON output
- **Data Preservation** — Cached daily summaries survive CLI data deletion

## Supported AI CLIs

| CLI | Data Location |
|-----|---------------|
| Claude Code | `~/.claude/projects/` |
| Codex CLI | `~/.codex/sessions/` |
| Gemini CLI | `~/.gemini/tmp/*/chats/` |

## Supported Platforms

| Platform | Architecture |
|----------|-------------|
| macOS | x64, ARM64 |
| Linux | x64, ARM64 |
| Windows | x64 |

## Quick Usage

```bash
# Launch TUI dashboard
npx toktrack

# JSON output for scripting
npx toktrack daily --json
npx toktrack stats --json
```

## Performance

| Mode | Time |
|------|------|
| Cold start (no cache) | **~1.2s** |
| Warm start (cached) | **~0.05s** |

> Measured on Apple Silicon (9,000+ files / 3.4 GB).

## Links

- [GitHub](https://github.com/mag123c/toktrack)
- [Documentation](https://github.com/mag123c/toktrack#readme)
- [Releases](https://github.com/mag123c/toktrack/releases)
- [Changelog](https://github.com/mag123c/toktrack/blob/main/CHANGELOG.md)

## License

MIT
