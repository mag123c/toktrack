//! OpenCode CLI parser: reads SQLite (`opencode.db`, v1.2.0+) when present,
//! falls back to legacy JSON files (`storage/message/**/msg_*.json`, <= v1.1.65).

use crate::types::{Result, ToktrackError, UsageEntry};
use chrono::DateTime;
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::CLIParser;

/// JSON file shape (legacy: one message per file, includes id/sessionID).
#[derive(Deserialize)]
struct OpenCodeMessageFile {
    id: String,
    #[serde(rename = "sessionID")]
    session_id: String,
    #[serde(flatten)]
    data: OpenCodeMessageData,
}

/// SQLite `data` column shape (id/sessionID live in row columns).
#[derive(Deserialize)]
struct OpenCodeMessageData {
    #[serde(rename = "modelID")]
    model_id: Option<String>,
    #[serde(rename = "providerID")]
    provider_id: Option<String>,
    time: OpenCodeTime,
    tokens: Option<OpenCodeTokens>,
    cost: Option<f64>,
}

#[derive(Deserialize)]
struct OpenCodeTime {
    created: u64,
}

#[derive(Deserialize)]
struct OpenCodeTokens {
    input: u64,
    output: u64,
    #[serde(default)]
    reasoning: u64,
    #[serde(default)]
    cache: Option<OpenCodeCache>,
}

#[derive(Deserialize)]
struct OpenCodeCache {
    read: u64,
    write: u64,
}

pub struct OpenCodeParser {
    json_dir: PathBuf,
    db_path: PathBuf,
}

impl OpenCodeParser {
    /// Default: `~/.local/share/opencode/{opencode.db, storage/message/}`.
    /// OpenCode uses XDG layout on every platform.
    pub fn new() -> Self {
        let base = directories::BaseDirs::new()
            .map(|d| d.home_dir().join(".local").join("share"))
            .unwrap_or_else(|| {
                eprintln!("[toktrack] Warning: Could not determine home directory");
                PathBuf::from(".")
            })
            .join("opencode");
        Self::with_base_dir(base)
    }

    /// Construct from an OpenCode base directory (contains both `opencode.db`
    /// and `storage/message/`). Preferred for SQLite-aware tests.
    pub fn with_base_dir(base_dir: PathBuf) -> Self {
        let json_dir = base_dir.join("storage").join("message");
        let db_path = base_dir.join("opencode.db");
        Self { json_dir, db_path }
    }

    /// Legacy constructor pointing directly at the JSON message directory.
    /// SQLite mode is disabled (db_path set to a sibling that won't exist).
    #[cfg(test)]
    pub fn with_data_dir(json_dir: PathBuf) -> Self {
        let db_path = json_dir.join("__no_sqlite__.db");
        Self { json_dir, db_path }
    }

    fn parse_sqlite_rows(&self, since_ms: Option<i64>) -> Result<Vec<UsageEntry>> {
        let rows = match self.query_sqlite_rows(since_ms) {
            Ok(rows) => rows,
            Err(e) => {
                eprintln!(
                    "[toktrack] Warning: failed to read OpenCode SQLite db {:?}: {}",
                    self.db_path, e
                );
                return Ok(Vec::new());
            }
        };

        let mut entries = Vec::with_capacity(rows.len());
        for SqliteMessageRow {
            id,
            session_id,
            mut data_json,
        } in rows
        {
            // SAFETY: `data_json` is exclusively owned and not aliased; safe for simd_json in-place mutation.
            let data: OpenCodeMessageData = match unsafe { simd_json::from_str(&mut data_json) } {
                Ok(d) => d,
                Err(e) => {
                    eprintln!(
                        "[toktrack] Warning: failed to parse OpenCode SQLite row {}: {}",
                        id, e
                    );
                    continue;
                }
            };
            if let Some(entry) = to_usage_entry(id, session_id, data) {
                entries.push(entry);
            }
        }
        Ok(entries)
    }

    fn query_sqlite_rows(&self, since_ms: Option<i64>) -> rusqlite::Result<Vec<SqliteMessageRow>> {
        // Pass the Path directly so paths with spaces (Windows usernames, etc.) don't
        // need URI percent-encoding. SQLITE_OPEN_READ_ONLY alone gives read-only access.
        let conn = Connection::open_with_flags(&self.db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;

        let base_sql = "SELECT id, session_id, data \
             FROM message \
             WHERE json_extract(data, '$.role') = 'assistant' \
               AND COALESCE(json_extract(data, '$.tokens.input'), 0) \
                 + COALESCE(json_extract(data, '$.tokens.output'), 0) > 0";

        let map_row = |r: &rusqlite::Row<'_>| -> rusqlite::Result<SqliteMessageRow> {
            Ok(SqliteMessageRow {
                id: r.get(0)?,
                session_id: r.get(1)?,
                data_json: r.get(2)?,
            })
        };

        match since_ms {
            Some(ms) => {
                let sql = format!("{} AND time_created >= ?", base_sql);
                let mut stmt = conn.prepare(&sql)?;
                stmt.query_map([ms], map_row).and_then(Iterator::collect)
            }
            None => {
                let mut stmt = conn.prepare(base_sql)?;
                stmt.query_map([], map_row).and_then(Iterator::collect)
            }
        }
    }
}

struct SqliteMessageRow {
    id: String,
    session_id: String,
    data_json: String,
}

fn to_usage_entry(id: String, session_id: String, msg: OpenCodeMessageData) -> Option<UsageEntry> {
    let tokens = msg.tokens?;

    let timestamp = i64::try_from(msg.time.created)
        .ok()
        .and_then(DateTime::from_timestamp_millis)?;

    let (cache_read, cache_write) = match tokens.cache {
        Some(c) => (c.read, c.write),
        None => (0, 0),
    };

    Some(UsageEntry {
        timestamp,
        model: msg.model_id,
        input_tokens: tokens.input,
        output_tokens: tokens.output,
        cache_read_tokens: cache_read,
        cache_creation_tokens: cache_write,
        thinking_tokens: tokens.reasoning,
        cache_creation_5m_tokens: 0,
        cache_creation_1h_tokens: 0,
        web_search_requests: 0,
        cost_usd: msg.cost,
        message_id: Some(id),
        request_id: Some(session_id),
        source: Some("opencode".into()),
        provider: msg.provider_id,
    })
}

impl Default for OpenCodeParser {
    fn default() -> Self {
        Self::new()
    }
}

impl CLIParser for OpenCodeParser {
    fn name(&self) -> &str {
        "opencode"
    }

    fn data_dir(&self) -> &Path {
        &self.json_dir
    }

    fn file_pattern(&self) -> &str {
        "**/msg_*.json"
    }

    fn parse_file(&self, path: &Path) -> Result<Vec<UsageEntry>> {
        let mut content = fs::read_to_string(path).map_err(ToktrackError::Io)?;
        // SAFETY: `content` is exclusively owned and not aliased; safe for simd_json in-place mutation.
        let file: OpenCodeMessageFile = unsafe {
            simd_json::from_str(&mut content).map_err(|e| ToktrackError::Parse(e.to_string()))?
        };
        Ok(to_usage_entry(file.id, file.session_id, file.data)
            .map(|e| vec![e])
            .unwrap_or_default())
    }

    fn parse_all(&self) -> Result<Vec<UsageEntry>> {
        if self.db_path.exists() {
            return self.parse_sqlite_rows(None);
        }
        let files = self.collect_files();
        Self::parse_and_dedup(self, &files)
    }

    fn parse_recent_files(&self, since: SystemTime) -> Result<Vec<UsageEntry>> {
        if self.db_path.exists() {
            let since_ms = since
                .duration_since(UNIX_EPOCH)
                .ok()
                .and_then(|d| i64::try_from(d.as_millis()).ok())
                .unwrap_or(0);
            return self.parse_sqlite_rows(Some(since_ms));
        }
        let all_files = self.collect_files();
        let recent: Vec<PathBuf> = all_files
            .into_iter()
            .filter(|f| {
                f.metadata()
                    .and_then(|m| m.modified())
                    .map(|mtime| mtime >= since)
                    .unwrap_or(true)
            })
            .collect();
        Self::parse_and_dedup(self, &recent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use tempfile::TempDir;

    fn fixture_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("opencode")
            .join("storage")
            .join("message")
    }

    fn fixture_path(filename: &str) -> PathBuf {
        fixture_dir().join("ses_test").join(filename)
    }

    /// Create a SQLite db with the OpenCode v1.2.0 message-table schema and seed rows.
    fn create_test_db(path: &Path, rows: &[(&str, &str, i64, &str)]) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL,
                data TEXT NOT NULL
            );",
        )
        .unwrap();
        for (id, sid, created, data) in rows {
            conn.execute(
                "INSERT INTO message (id, session_id, time_created, time_updated, data) \
                 VALUES (?, ?, ?, ?, ?)",
                params![id, sid, created, created, data],
            )
            .unwrap();
        }
    }

    fn assistant_data(model: &str, ts_ms: u64, input: u64, output: u64) -> String {
        format!(
            r#"{{"role":"assistant","modelID":"{model}","providerID":"anthropic",
                "time":{{"created":{ts_ms}}},
                "tokens":{{"input":{input},"output":{output},"reasoning":0,
                  "cache":{{"read":0,"write":0}}}},
                "cost":0.01}}"#
        )
    }

    // ===== Existing JSON-mode tests (regression coverage) =====

    #[test]
    fn parses_single_json_message() {
        let parser = OpenCodeParser::with_data_dir(fixture_dir());
        let entries = parser.parse_file(&fixture_path("msg_001.json")).unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn extracts_json_message_details() {
        let parser = OpenCodeParser::with_data_dir(fixture_dir());
        let entries = parser.parse_file(&fixture_path("msg_001.json")).unwrap();
        let entry = &entries[0];
        assert_eq!(entry.model, Some("claude-sonnet-4-20250514".to_string()));
        assert_eq!(entry.input_tokens, 1000);
        assert_eq!(entry.output_tokens, 500);
        assert_eq!(entry.cache_read_tokens, 100);
        assert_eq!(entry.cache_creation_tokens, 50);
        assert_eq!(entry.thinking_tokens, 0);
        assert_eq!(entry.cost_usd, Some(0.05));
        assert_eq!(entry.source, Some("opencode".into()));
        assert_eq!(entry.message_id, Some("msg_001".to_string()));
        assert_eq!(entry.request_id, Some("ses_test".to_string()));
    }

    #[test]
    fn parses_json_reasoning_tokens() {
        let parser = OpenCodeParser::with_data_dir(fixture_dir());
        let entries = parser.parse_file(&fixture_path("msg_002.json")).unwrap();
        let entry = &entries[0];
        assert_eq!(entry.input_tokens, 2000);
        assert_eq!(entry.output_tokens, 800);
        assert_eq!(entry.cache_read_tokens, 200);
        assert_eq!(entry.cache_creation_tokens, 100);
        assert_eq!(entry.thinking_tokens, 150);
        assert_eq!(entry.cost_usd, Some(0.12));
    }

    #[test]
    fn skips_json_message_without_tokens() {
        let parser = OpenCodeParser::with_data_dir(fixture_dir());
        let entries = parser
            .parse_file(&fixture_path("msg_003_no_tokens.json"))
            .unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn parser_name_is_opencode() {
        let parser = OpenCodeParser::new();
        assert_eq!(parser.name(), "opencode");
    }

    #[test]
    fn parser_uses_msg_glob_pattern() {
        let parser = OpenCodeParser::new();
        assert_eq!(parser.file_pattern(), "**/msg_*.json");
    }

    #[test]
    fn returns_error_for_missing_json_file() {
        let parser = OpenCodeParser::new();
        let result = parser.parse_file(Path::new("/nonexistent/file.json"));
        assert!(result.is_err());
    }

    #[test]
    fn computes_total_tokens_for_json_entry() {
        let parser = OpenCodeParser::with_data_dir(fixture_dir());
        let entries = parser.parse_file(&fixture_path("msg_001.json")).unwrap();
        // 1000 + 500 + 100 + 50 + 0 = 1650
        assert_eq!(entries[0].total_tokens(), 1650);
    }

    #[test]
    fn computes_total_tokens_with_reasoning() {
        let parser = OpenCodeParser::with_data_dir(fixture_dir());
        let entries = parser.parse_file(&fixture_path("msg_002.json")).unwrap();
        // 2000 + 800 + 200 + 100 + 150 = 3250
        assert_eq!(entries[0].total_tokens(), 3250);
    }

    // ===== New SQLite-mode tests =====

    #[test]
    fn reads_from_sqlite_when_db_exists() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().to_path_buf();
        create_test_db(
            &base.join("opencode.db"),
            &[
                (
                    "msg_a",
                    "ses_a",
                    1_700_000_000_000,
                    &assistant_data("opus-4", 1_700_000_000_000, 1000, 500),
                ),
                (
                    "msg_b",
                    "ses_a",
                    1_700_000_001_000,
                    &assistant_data("opus-4", 1_700_000_001_000, 200, 100),
                ),
            ],
        );

        let parser = OpenCodeParser::with_base_dir(base);
        let entries = parser.parse_all().unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].source, Some("opencode".into()));
    }

    #[test]
    fn falls_back_to_json_when_db_missing() {
        // Re-uses existing JSON fixtures; no SQLite db present.
        let parser = OpenCodeParser::with_base_dir(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fixtures")
                .join("opencode"),
        );
        let entries = parser.parse_all().unwrap();
        // msg_001 + msg_002 yield entries; msg_003_no_tokens is skipped.
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn sqlite_takes_precedence_over_json() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().to_path_buf();

        // Seed JSON files that should be ignored when the db is present.
        let json_dir = base.join("storage").join("message").join("ses_legacy");
        fs::create_dir_all(&json_dir).unwrap();
        fs::write(
            json_dir.join("msg_legacy.json"),
            r#"{"id":"msg_legacy","sessionID":"ses_legacy","modelID":"old-model",
                "providerID":"anthropic","time":{"created":1700000000000},
                "tokens":{"input":99,"output":99,"reasoning":0,"cache":{"read":0,"write":0}},
                "cost":9.99}"#,
        )
        .unwrap();

        create_test_db(
            &base.join("opencode.db"),
            &[(
                "msg_db",
                "ses_db",
                1_700_000_500_000,
                &assistant_data("new-model", 1_700_000_500_000, 1, 1),
            )],
        );

        let parser = OpenCodeParser::with_base_dir(base);
        let entries = parser.parse_all().unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message_id, Some("msg_db".to_string()));
        assert_eq!(entries[0].model, Some("new-model".to_string()));
    }

    #[test]
    fn sqlite_skips_user_messages() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().to_path_buf();
        let user_data = r#"{"role":"user","time":{"created":1700000000000}}"#;
        create_test_db(
            &base.join("opencode.db"),
            &[
                ("user_1", "ses_a", 1_700_000_000_000, user_data),
                (
                    "asst_1",
                    "ses_a",
                    1_700_000_001_000,
                    &assistant_data("opus-4", 1_700_000_001_000, 100, 100),
                ),
            ],
        );

        let parser = OpenCodeParser::with_base_dir(base);
        let entries = parser.parse_all().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message_id, Some("asst_1".to_string()));
    }

    #[test]
    fn sqlite_skips_zero_token_messages() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().to_path_buf();
        create_test_db(
            &base.join("opencode.db"),
            &[
                (
                    "zero",
                    "ses_a",
                    1_700_000_000_000,
                    &assistant_data("opus-4", 1_700_000_000_000, 0, 0),
                ),
                (
                    "real",
                    "ses_a",
                    1_700_000_001_000,
                    &assistant_data("opus-4", 1_700_000_001_000, 1, 0),
                ),
            ],
        );

        let parser = OpenCodeParser::with_base_dir(base);
        let entries = parser.parse_all().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message_id, Some("real".to_string()));
    }

    #[test]
    fn sqlite_filters_recent_files_by_time() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().to_path_buf();
        create_test_db(
            &base.join("opencode.db"),
            &[
                (
                    "old",
                    "ses_a",
                    1_700_000_000_000,
                    &assistant_data("opus-4", 1_700_000_000_000, 100, 100),
                ),
                (
                    "new",
                    "ses_a",
                    1_700_000_500_000,
                    &assistant_data("opus-4", 1_700_000_500_000, 100, 100),
                ),
            ],
        );

        let parser = OpenCodeParser::with_base_dir(base);
        let since = UNIX_EPOCH + std::time::Duration::from_millis(1_700_000_400_000);
        let entries = parser.parse_recent_files(since).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message_id, Some("new".to_string()));
    }

    #[test]
    fn sqlite_skips_rows_with_malformed_data_blob() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().to_path_buf();
        // SQL `json_extract` returns NULL for invalid JSON (filtered out at SQL),
        // so to reach the simd_json fallback we need JSON that passes the role/token
        // filters but breaks struct deserialization (here: `time.created` as string).
        let bad_struct = r#"{"role":"assistant","modelID":"x","providerID":"y",
            "time":{"created":"not-a-number"},
            "tokens":{"input":1,"output":1,"reasoning":0,"cache":{"read":0,"write":0}},
            "cost":0.0}"#;
        create_test_db(
            &base.join("opencode.db"),
            &[
                ("bad", "ses_a", 1_700_000_000_000, bad_struct),
                (
                    "good",
                    "ses_a",
                    1_700_000_001_000,
                    &assistant_data("opus-4", 1_700_000_001_000, 1, 1),
                ),
            ],
        );

        let parser = OpenCodeParser::with_base_dir(base);
        let entries = parser.parse_all().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message_id, Some("good".to_string()));
    }

    #[test]
    fn sqlite_open_failure_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().to_path_buf();
        // Write a non-sqlite file at the db path so opening fails after the existence check.
        fs::write(base.join("opencode.db"), b"not a real sqlite file").unwrap();

        let parser = OpenCodeParser::with_base_dir(base);
        let entries = parser.parse_all().unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn sqlite_extracts_all_token_fields() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().to_path_buf();
        let data = r#"{"role":"assistant","modelID":"opus-4","providerID":"anthropic",
            "time":{"created":1700000000000},
            "tokens":{"input":1000,"output":500,"reasoning":150,
                "cache":{"read":200,"write":100}},
            "cost":0.42}"#;
        create_test_db(
            &base.join("opencode.db"),
            &[("m1", "s1", 1_700_000_000_000, data)],
        );

        let parser = OpenCodeParser::with_base_dir(base);
        let entries = parser.parse_all().unwrap();

        let e = &entries[0];
        assert_eq!(e.model, Some("opus-4".to_string()));
        assert_eq!(e.provider, Some("anthropic".to_string()));
        assert_eq!(e.input_tokens, 1000);
        assert_eq!(e.output_tokens, 500);
        assert_eq!(e.thinking_tokens, 150);
        assert_eq!(e.cache_read_tokens, 200);
        assert_eq!(e.cache_creation_tokens, 100);
        assert_eq!(e.cost_usd, Some(0.42));
        assert_eq!(e.message_id, Some("m1".to_string()));
        assert_eq!(e.request_id, Some("s1".to_string()));
    }
}
