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

insta::assert_debug_snapshot!(result);
```
Location: `#[cfg(test)]` in same file
Fixtures: `tests/fixtures/`

## Error
Use `ToktrackError` consistently across all modules. Do NOT use `anyhow` in library code.
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

## Paradigm

### Trait-based Polymorphism (OOP)
```rust
// Interface for extensibility (planned: OpenCode, Gemini parsers)
pub trait CLIParser: Send + Sync { ... }
Box<dyn CLIParser>  // Runtime polymorphism
```

### Functional Patterns (FP)
```rust
// Prefer iterators + combinators
files.par_iter().flat_map(...).collect()
HashMap::entry().or_insert_with(...)

// Immutability by default
let result = ...;  // not mut unless necessary
```

### YAGNI
- Abstract only for **planned** extensions (see architecture.md roadmap)
- No speculative generalization

## Docs
- `///` for pub items
- Include examples
