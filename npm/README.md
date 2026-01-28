# toktrack

Ultra-fast AI CLI token usage tracker. Built with Rust + simd-json + ratatui.

![toktrack overview](https://raw.githubusercontent.com/mag123c/toktrack/main/image.png)

## Installation

```bash
# No installation required
npx toktrack

# Or install globally
npm install -g toktrack
```

## Features

- **Blazing Fast** - 40x faster than Node.js alternatives (~2 GiB/s throughput)
- **Beautiful TUI** - 4 views (Overview, Models, Daily, Stats) with heatmap
- **Multi-CLI Support** - Claude Code, Codex CLI, Gemini CLI
- **CLI Commands** - `daily`, `stats` with JSON output
- **Auto Update** - Check for updates on startup

## Supported AI CLIs

| CLI | Data Location |
|-----|---------------|
| Claude Code | `~/.claude/projects/` |
| Codex CLI | `~/.codex/sessions/` |
| Gemini CLI | `~/.gemini/tmp/*/chats/` |

## Usage

### TUI Mode (Default)

```bash
toktrack
```

### CLI Commands

```bash
toktrack daily          # Daily usage report
toktrack daily --json   # JSON output

toktrack stats          # Statistics
toktrack stats --json   # JSON output
```

### Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `1-4` | Switch tabs |
| `Tab` | Next tab |
| `j/k` or `↑/↓` | Scroll |
| `?` | Help |
| `q` | Quit |

## Performance

| Tool | Time (2,000+ files) |
|------|---------------------|
| ccusage (Node.js) | ~20s |
| ccusage (cached) | ~7s |
| **toktrack** | **< 500ms** |

## Links

- [GitHub](https://github.com/mag123c/toktrack)
- [Releases](https://github.com/mag123c/toktrack/releases)

## License

MIT
