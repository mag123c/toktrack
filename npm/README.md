# toktrack

[![CI](https://github.com/mag123c/toktrack/actions/workflows/ci.yml/badge.svg)](https://github.com/mag123c/toktrack/actions/workflows/ci.yml)
[![npm](https://img.shields.io/npm/v/toktrack)](https://www.npmjs.com/package/toktrack)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://github.com/mag123c/toktrack/blob/main/LICENSE)

Ultra-fast token & cost tracker for Claude Code, Codex CLI, Gemini CLI, OpenCode, and PI Agent. Built with Rust for ultra-fast performance (simd-json + rayon).

> **⚠️ Did you know?** Claude Code **deletes your session data after 30 days** by default. Once deleted, your token usage and cost history are gone forever — unless you preserve them.

![toktrack overview](https://raw.githubusercontent.com/mag123c/toktrack/main/assets/demo.gif)

## Why toktrack?

| Problem | Solution |
|---------|----------|
| 🐌 **Existing tools are slow** — 40+ seconds | ⚡ **1000x faster** — cached queries in ~0.04s |
| 🗑️ **Claude Code deletes data after 30 days** | 💾 **Persistent cache** — history survives |
| 📊 **No unified view** — each CLI has separate data | 🎯 **One dashboard** — all CLIs in one place |

## Installation

```bash
# No installation required
npx toktrack

# Or install globally
npm install -g toktrack
```

### Homebrew (macOS / Linux)

```bash
brew tap mag123c/toktrack
brew install toktrack
```

## Features

- **Ultra-Fast Parsing** — simd-json + rayon parallel processing (~3 GiB/s throughput)
- **TUI Dashboard** — 3 tabs (Overview, Stats, Models) with daily/weekly/monthly views
- **Multi-CLI Support** — Claude Code, Codex CLI, Gemini CLI, OpenCode, PI Agent
- **CLI Commands** — `daily`, `weekly`, `monthly`, `stats` with JSON output
- **Data Preservation** — Cached daily summaries survive CLI data deletion

## Supported AI CLIs

| CLI | Data Location |
|-----|---------------|
| Claude Code | `~/.claude/projects/` |
| Codex CLI | `~/.codex/sessions/` |
| Gemini CLI | `~/.gemini/tmp/*/chats/` |
| OpenCode | `~/.local/share/opencode/storage/message/` |
| PI Agent | `~/.pi/agent/sessions/` |

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

| Tool | Time | Speedup |
|------|------|---------|
| Existing tools | 40s+ | baseline |
| **toktrack** (cold) | **~1.0s** | **40x faster** |
| **toktrack** (cached) | **~0.04s** | **1000x faster** |

> Measured on Apple Silicon with 2,000+ JSONL files (3.4 GB).

## Links

- [GitHub](https://github.com/mag123c/toktrack)
- [Documentation](https://github.com/mag123c/toktrack#readme)
- [Releases](https://github.com/mag123c/toktrack/releases)
- [Changelog](https://github.com/mag123c/toktrack/blob/main/CHANGELOG.md)

## License

MIT
