# Conventions

## Naming
| Type | Style | Example |
|------|-------|---------|
| files | snake_case | `claude_parser.rs` |
| structs | PascalCase | `ClaudeCodeParser` |
| traits | PascalCase | `CLIParser` |
| functions | snake_case | `parse_file` |
| constants | SCREAMING | `DEFAULT_CACHE_DIR` |

## TDD Cycle
```
RED → GREEN → REFACTOR
```
- No impl without test
- Test describes behavior
- Mock external deps

## Test
```rust
#[test]
fn test_parse_file_valid_jsonl() { ... }
```
Location: `#[cfg(test)]` in same file
Fixtures: `tests/fixtures/`

## Error
Use `ToktrackError` consistently. No `anyhow` in library code.
```rust
#[derive(thiserror::Error)]
enum ToktrackError {
    #[error("parse: {0}")] Parse(String),
    #[error("io: {0}")] Io(#[from] std::io::Error),
    #[error("cache: {0}")] Cache(String),
    #[error("pricing: {0}")] Pricing(String),
    #[error("config: {0}")] Config(String),
}
type Result<T> = std::result::Result<T, ToktrackError>;
```

## Data Loading: Degrade Gracefully
`DataLoaderService` aggregates many independent sources. A single source failing
must **never** abort the whole load. Log a warning and keep aggregating the rest:
```rust
eprintln!("[toktrack] Warning: {} failed: {}", source.label, e);
continue;
```
This applies to any new source, local or remote. For optional/remote sources that
sync over the network, prefer falling back to the last good snapshot/cache on
failure. Distinguish intent: a source the user selected **explicitly** may fail
loudly, but a source pulled in **implicitly** (e.g. a configured default) must not
break a plain command — degrade to local-only and warn.

A panic bypasses all of the above: parsers run under `rayon`'s `par_iter`, so one
panic takes down every source at once. Numbers read from a session file are
untrusted input — use `saturating_add`/`saturating_sub` on them, never bare
`+`/`-` (which panics in debug and wraps silently in release).

## Commits
```
type(scope): description

types: feat|fix|refactor|docs|test|chore|perf
scopes: parser|tui|services|cache|cli
```

## Performance
- simd-json for JSON
- rayon for parallel
- Minimize allocations
- Benchmark vs ccusage

## Project Decisions

See `.dev/DECISIONS.md` (local only, gitignored) for design decision history.
Verify new features do not conflict with existing decisions.

---

## Paradigm

### Trait-based Polymorphism
```rust
pub trait CLIParser: Send + Sync { ... }
Box<dyn CLIParser>  // Runtime polymorphism
```

### Functional Patterns
```rust
files.par_iter().flat_map(...).collect()
HashMap::entry().or_insert_with(...)
let result = ...;  // Immutable by default
```

### YAGNI
- Abstract only for planned extensions
- No speculative generalization
