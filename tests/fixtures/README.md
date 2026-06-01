# Parser fixtures

Fixtures pin each source's on-disk format so upstream schema drift is caught by
tests rather than silently mis-counting in production.

## Token-semantics contract (v2)

Every parser must populate `UsageEntry` per this contract (see `src/types/usage.rs`):

| field | meaning |
|-------|---------|
| `input_tokens` | billable **non-cached** input (excludes `cache_read_tokens`) |
| `cache_read_tokens` | cached input read |
| `cache_creation_tokens` | cache write |
| `output_tokens` | **visible** output only (excludes reasoning) |
| `reasoning_tokens` | hidden/reasoning output |
| `reported_total_tokens` | upstream-reported total, reconciliation only (not summed, not priced) |

Invariants asserted by tests:
- `total_tokens() = input + output + cache_read + cache_creation + reasoning`
- where `reported_total_tokens` is `Some`, it must equal `total_tokens()`

## Provenance & sanitization

Fixtures derived from real local data have **all message content, file paths,
project names, git branches, and identifiers redacted** — only the usage/token
structure is preserved.

| source | fixture(s) | provenance |
|--------|-----------|------------|
| Claude | `claude-sample.jsonl`, `claude/real-shape-session.jsonl` | real-shape from `~/.claude/projects` (sanitized); covers cache TTL 5m/1h, web_search, and fields toktrack ignores (`web_fetch_requests`, `service_tier`, `iterations`) |
| Codex | `codex/*.jsonl` | token_count delta sessions, session_meta provider |
| Gemini | `gemini/tmp*/chats/*` | legacy `.json` + streaming `.jsonl`; `total` reconciles with `total_tokens()` (input excludes cached) |
| Qwen | `qwen/proj/chats/*.jsonl` | Gemini CLI fork — identical format; parsed by the same parser with source "qwen" |
| OpenCode | `opencode/...`, in-test SQLite | message rows with reasoning + cache + cost |
| PI Agent | `pi_agent/*.jsonl` | assistant usage; nested `cost.total` |

## Notes
- Claude `usage.server_tool_use.web_fetch_requests` — captured into
  `UsageEntry.web_fetch_requests`. No LiteLLM price exists, so it is priced only
  via a custom `global.web_fetch_per_request` override (else $0).
