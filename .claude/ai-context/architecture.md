# Architecture

## Layers
```
TUI[ratatui] → CLI[clap] → Services → Parsers[trait] → Cache
```

## Paths
- `src/tui/` - TUI (app.rs, theme.rs, widgets/)
- `src/cli/` - CLI commands
- `src/services/` - aggregator, pricing, cache, update_checker, normalizer, data_loader
- `src/parsers/` - CLIParser trait + impls
- `src/types/` - UsageEntry, errors

## TUI Widgets
| Widget | Purpose |
|--------|---------|
| `theme.rs` | Theme enum (Dark/Light), auto-detect via `terminal-light`, 10 semantic color methods + heatmap_color |
| `app.rs` | ViewMode (Dashboard{tab}/SourceDetail), TuiConfig (initial_view_mode, initial_tab), event loop, Theme::detect() before raw mode |
| `widgets/spinner.rs` | Loading animation (dots/braille) |
| `widgets/heatmap.rs` | 52-week heatmap (2x2 blocks, 14 rows, responsive, colorblind-accessible) |
| `widgets/tabs.rs` | Tab enum (Overview/Stats/Models), TabBar widget, next/prev/from_number |
| `widgets/stats.rs` | Stats view (6 cards: tokens, avg, peak, cost, avg cost, active days) with TabBar |
| `widgets/overview.rs` | Overview tab (hero stat, cost, heatmap, source bars, TabBar, keybindings) |
| `widgets/source_detail.rs` | Source drill-down (per-source daily table + models + stats, d/w/m modes) |
| `widgets/models.rs` | Models tab (per-model breakdown, cost %, percentage bar, TabBar, up to 10 models) |
| `widgets/daily.rs` | Daily view (daily/weekly/monthly modes, sparklines, scroll, responsive columns) |
| `widgets/help.rs` | Help popup (keyboard shortcuts overlay, `?` toggle) |
| `widgets/quit_confirm.rs` | Quit confirm popup (Ctrl+C trigger, y/n/Enter/Esc response) |
| `widgets/model_breakdown.rs` | Model breakdown popup (Enter on Daily row, shows per-model cost) |
| `widgets/legend.rs` | Heatmap intensity legend |

## Core Trait
```rust
trait CLIParser: Send + Sync {
    fn name(&self) -> &str;
    fn data_dir(&self) -> &Path;
    fn file_pattern(&self) -> &str;
    fn parse_file(&self, path: &Path) -> Result<Vec<UsageEntry>>;
    fn parse_all(&self) -> Result<Vec<UsageEntry>>;           // full parse (rayon)
    fn parse_recent_files(&self, since: SystemTime) -> Result<Vec<UsageEntry>>; // mtime filter
    fn collect_files(&self) -> Vec<PathBuf>;                  // glob collect
    fn parse_and_dedup(&self, files: &[PathBuf]) -> Result<Vec<UsageEntry>>;    // shared logic
}
```

## Implementations
| Parser | Format | Data Dir | Status |
|--------|--------|----------|--------|
| ClaudeCodeParser | JSONL | ~/.claude/projects/ | ✅ |
| CodexParser | JSONL | ~/.codex/sessions/ | ✅ |
| GeminiParser | JSON + JSONL | ~/.gemini/tmp/*/chats/ | ✅ |
| GeminiParser (qwen) | JSON + JSONL | ~/.qwen/tmp/*/chats/ | ✅ Qwen Code (Gemini fork, source="qwen") |
| OpenCodeParser | SQLite (v1.2.0+) + JSON fallback | ~/.local/share/opencode/{opencode.db, storage/message/} | ✅ |
| PiAgentParser | JSONL | ~/.pi/agent/sessions/ | ✅ |
| AntigravityParser | SQLite + protobuf blobs | ~/.gemini/antigravity-{ide,cli}/conversations/*.db | ✅ (gen_metadata → ChatModelMetadata/ModelUsageStats) |
| GrokParser | JSONL | ~/.grok/sessions/*/*/updates.jsonl | ✅ (`_x.ai/session/update` + `sessionUpdate == turn_completed` → `usage.modelUsage`; cost from `costUsdTicks`, the only parser deriving it from an integer tick count; absent ticks stay `None` so the turn is not reported as an exact $0) |

## Data Flow
```
[Warm Path] cache exists:
1. PricingService::from_cache_only()    → no network
2. parse_recent_files(yesterday 00:00)   → mtime filter
3. cache.load_or_compute(entries)       → cached past + fresh today
4. Aggregator::total_from_daily()       → no raw entries needed
5. Aggregator::by_model_from_daily()    → no raw entries needed

[Cold Path] first run / no cache:
1. parse_all() per parser               → full glob + rayon
2. PricingService::new()                → network fallback
3. cache.load_or_compute(entries)       → build cache
4. Aggregator from summaries

[Aggregation]
- normalize_model_name()       → date suffix removal, dot→hyphen
- display_name()               → TUI friendly: "claude-opus-4-5"→"Opus 4.5"
- Aggregator::merge_by_date()  → combine multi-source summaries
- Aggregator::by_source()      → SourceUsage per CLI
- is_copilot_provider()        → github-copilot cost=0
```

## Parser Optimizations
| Technique | Description | Throughput |
|-----------|-------------|------------|
| Zero-copy serde | `&'a str` borrowed, no String alloc | ~1.0 GiB/s |
| In-place buffer | `&mut [u8]` to simd-json | |
| SIMD parsing | simd-json AVX2/NEON | |
| rayon parallel | `parse_all()` file-level parallel | ~3.0 GiB/s |

## Cache (~/.toktrack/)
```
cache/
├── {cli}_daily.json  # DailySummary cache (recomputes dates with new entries)
└── pricing.json      # LiteLLM 1h TTL
```

## Deps
```toml
simd-json, ratatui, crossterm, clap, rayon, chrono, directories, serde, reqwest, tokio, fs2, terminal-light
dev: criterion, tempfile
```

## Pre-commit
```bash
make check  # fmt + clippy + test
```
