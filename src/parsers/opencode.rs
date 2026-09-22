//! OpenCode CLI parser: reads v1/v2 SQLite tables (`opencode.db`) when present,
//! falls back to legacy JSON files (`storage/message/**/msg_*.json`, <= v1.1.65).

use crate::types::{Result, ToktrackError, UsageEntry};
use chrono::DateTime;
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use std::collections::HashSet;
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
    model: Option<OpenCodeModel>,
    time: OpenCodeTime,
    tokens: Option<OpenCodeTokens>,
    cost: Option<f64>,
}

#[derive(Deserialize)]
struct OpenCodeModel {
    id: Option<String>,
    #[serde(rename = "providerID")]
    provider_id: Option<String>,
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
    ///
    /// Honors `OPENCODE_DATA_DIR` (the full data dir) then `XDG_DATA_HOME`
    /// (base; `<XDG_DATA_HOME>/opencode`), falling back to the XDG default.
    pub fn new() -> Self {
        let base = super::discovery::first_env_dir(&["OPENCODE_DATA_DIR"])
            .or_else(|| {
                super::discovery::first_env_dir(&["XDG_DATA_HOME"]).map(|x| x.join("opencode"))
            })
            .unwrap_or_else(|| {
                directories::BaseDirs::new()
                    .map(|d| d.home_dir().join(".local").join("share").join("opencode"))
                    .unwrap_or_else(|| {
                        eprintln!("[toktrack] Warning: Could not determine home directory");
                        PathBuf::from(".")
                    })
            });
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
        let mut entries = Vec::new();
        let mut seen = HashSet::new();
        // Parse each row as it is read: v2 `data` embeds the full response
        // content, so collecting every row first holds the whole transcript in
        // memory.
        let visited = self.visit_sqlite_rows(since_ms, &mut |row| {
            let SqliteMessageRow {
                id,
                session_id,
                mut data_json,
                directory,
            } = row;
            // SAFETY: `data_json` is exclusively owned and not aliased; safe for simd_json in-place mutation.
            let data: OpenCodeMessageData = match unsafe { simd_json::from_str(&mut data_json) } {
                Ok(d) => d,
                Err(e) => {
                    eprintln!(
                        "[toktrack] Warning: failed to parse OpenCode SQLite row {}: {}",
                        id, e
                    );
                    return;
                }
            };
            if let Some(mut entry) = to_usage_entry(id, session_id, data) {
                // v2 is read first. Only successful, nonempty usage claims an ID:
                // a malformed/incomplete migrated row must not hide valid v1 usage.
                if entry.total_tokens() == 0
                    || !seen.insert((entry.message_id.clone(), entry.request_id.clone()))
                {
                    return;
                }
                // Attribute to the session's working directory (empty → no project).
                entry.project = directory.filter(|d| !d.is_empty());
                entries.push(entry);
            }
        });
        if let Err(e) = visited {
            eprintln!(
                "[toktrack] Warning: failed to read OpenCode SQLite db {:?}: {}",
                self.db_path, e
            );
        }
        Ok(entries)
    }

    fn visit_sqlite_rows(
        &self,
        since_ms: Option<i64>,
        visit: &mut dyn FnMut(SqliteMessageRow),
    ) -> rusqlite::Result<()> {
        // Pass the Path directly so paths with spaces (Windows usernames, etc.) don't
        // need URI percent-encoding. SQLITE_OPEN_READ_ONLY alone gives read-only access.
        let mut conn =
            Connection::open_with_flags(&self.db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        // Keep both schema reads on the same snapshot during migrations/writes.
        let tx = conn.transaction()?;
        for (messages, sessions, role_filter) in [
            // v2 records a compaction request's usage only on its compaction
            // row; no assistant row carries it.
            (
                "session_message",
                "session_v2",
                "m.type IN ('assistant', 'compaction')",
            ),
            (
                "message",
                "session",
                "CASE WHEN json_valid(m.data) THEN json_extract(m.data, '$.role') END = 'assistant'",
            ),
        ] {
            // A failure in one schema must not discard rows from the other.
            if let Err(e) =
                Self::visit_message_rows(&tx, since_ms, messages, sessions, role_filter, visit)
            {
                eprintln!(
                    "[toktrack] Warning: failed to read OpenCode {messages} in {:?}: {e}",
                    self.db_path
                );
            }
        }
        Ok(())
    }

    fn visit_message_rows(
        conn: &Connection,
        since_ms: Option<i64>,
        messages: &str,
        sessions: &str,
        role_filter: &str,
        visit: &mut dyn FnMut(SqliteMessageRow),
    ) -> rusqlite::Result<()> {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?)",
            [messages],
            |r| r.get(0),
        )?;
        if !exists {
            return Ok(());
        }
        // Identifiers and predicates are fixed internal literals, never user input.
        let select = |directory: &str, join: &str| {
            let mut sql = format!(
                "SELECT m.id, m.session_id, m.data, {directory} FROM {messages} m {join} WHERE {role_filter}"
            );
            if since_ms.is_some() {
                sql.push_str(" AND m.time_created >= ?");
            }
            sql
        };
        // Directory metadata is optional: if the joined read fails at any point
        // (missing or unreadable session table/column), read again without a
        // project. Rows visited before the failure come back with the same
        // (message, session) key, which the caller already deduplicates.
        let join = format!("LEFT JOIN {sessions} s ON s.id = m.session_id");
        Self::visit_query(conn, &select("s.directory", &join), since_ms, visit)
            .or_else(|_| Self::visit_query(conn, &select("NULL", ""), since_ms, visit))
    }

    fn visit_query(
        conn: &Connection,
        sql: &str,
        since_ms: Option<i64>,
        visit: &mut dyn FnMut(SqliteMessageRow),
    ) -> rusqlite::Result<()> {
        let mut stmt = conn.prepare(sql)?;
        let mut rows = stmt.query(rusqlite::params_from_iter(since_ms))?;
        while let Some(r) = rows.next()? {
            visit(SqliteMessageRow {
                id: r.get(0)?,
                session_id: r.get(1)?,
                data_json: r.get(2)?,
                // A non-text directory costs only this row its project.
                directory: r.get(3).ok().flatten(),
            });
        }
        Ok(())
    }
}

struct SqliteMessageRow {
    id: String,
    session_id: String,
    data_json: String,
    /// `session.directory` (the session's working directory) when available —
    /// used as the project identifier.
    directory: Option<String>,
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
        fast_speed: false,
        timestamp,
        model: msg
            .model_id
            .or_else(|| msg.model.as_ref().and_then(|m| m.id.clone())),
        input_tokens: tokens.input,
        output_tokens: tokens.output,
        cache_read_tokens: cache_read,
        cache_creation_tokens: cache_write,
        reasoning_tokens: tokens.reasoning,
        cache_creation_5m_tokens: 0,
        cache_creation_1h_tokens: 0,
        web_search_requests: 0,
        web_fetch_requests: 0,
        reported_total_tokens: None,
        cost_usd: msg.cost,
        message_id: Some(id),
        request_id: Some(session_id),
        source: Some("opencode".into()),
        provider: msg
            .provider_id
            .or_else(|| msg.model.and_then(|m| m.provider_id)),
        project: None,
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
        assert_eq!(entry.reasoning_tokens, 0);
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
        assert_eq!(entry.reasoning_tokens, 150);
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
    fn opencode_data_dir_env_override() {
        // OPENCODE_DATA_DIR is the full data dir → json/db derive from it.
        let saved = std::env::var("OPENCODE_DATA_DIR").ok();
        std::env::set_var("OPENCODE_DATA_DIR", "/tmp/toktrack-oc-data");
        let parser = OpenCodeParser::new();
        assert_eq!(
            parser.data_dir(),
            Path::new("/tmp/toktrack-oc-data/storage/message")
        );
        match saved {
            Some(v) => std::env::set_var("OPENCODE_DATA_DIR", v),
            None => std::env::remove_var("OPENCODE_DATA_DIR"),
        }
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
        assert_eq!(entries[0].total_tokens(), 1650);
    }

    #[test]
    fn computes_total_tokens_with_reasoning() {
        let parser = OpenCodeParser::with_data_dir(fixture_dir());
        let entries = parser.parse_file(&fixture_path("msg_002.json")).unwrap();
        assert_eq!(entries[0].total_tokens(), 3250);
    }

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
        let parser = OpenCodeParser::with_base_dir(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fixtures")
                .join("opencode"),
        );
        let entries = parser.parse_all().unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn sqlite_takes_precedence_over_json() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().to_path_buf();

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
        // Valid JSON passes the SQL role filter but can still fail typed
        // deserialization (here: `time.created` as a string).
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
        fs::write(base.join("opencode.db"), b"not a real sqlite file").unwrap();

        let parser = OpenCodeParser::with_base_dir(base);
        let entries = parser.parse_all().unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn sqlite_attributes_project_from_session_directory() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().to_path_buf();
        let db = base.join("opencode.db");
        create_test_db(
            &db,
            &[(
                "m1",
                "ses_x",
                1_700_000_000_000,
                &assistant_data("opus-4", 1_700_000_000_000, 100, 50),
            )],
        );
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE session (id TEXT PRIMARY KEY, directory TEXT NOT NULL);\n\
             INSERT INTO session (id, directory) VALUES ('ses_x', '/work/proj');",
        )
        .unwrap();
        drop(conn);

        let parser = OpenCodeParser::with_base_dir(base);
        let entries = parser.parse_all().unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].project.as_deref(), Some("/work/proj"));
    }

    #[test]
    fn sqlite_project_none_when_no_session_table() {
        // Older schema without a `session` table must fall back gracefully and
        // leave project = None (no error).
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().to_path_buf();
        create_test_db(
            &base.join("opencode.db"),
            &[(
                "m1",
                "ses_x",
                1_700_000_000_000,
                &assistant_data("opus-4", 1_700_000_000_000, 100, 50),
            )],
        );

        let parser = OpenCodeParser::with_base_dir(base);
        let entries = parser.parse_all().unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].project, None);
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
        assert_eq!(e.reasoning_tokens, 150);
        assert_eq!(e.cache_read_tokens, 200);
        assert_eq!(e.cache_creation_tokens, 100);
        assert_eq!(e.cost_usd, Some(0.42));
        assert_eq!(e.message_id, Some("m1".to_string()));
        assert_eq!(e.request_id, Some("s1".to_string()));
    }

    fn create_v2_db(path: &Path, rows: &[(&str, &str, i64, &str)]) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE session_message (
                id TEXT PRIMARY KEY, session_id TEXT NOT NULL, type TEXT NOT NULL,
                seq INTEGER NOT NULL, time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL, data TEXT NOT NULL
            );
            CREATE TABLE session_v2 (id TEXT PRIMARY KEY, directory TEXT NOT NULL);
            INSERT INTO session_v2 VALUES ('s1', '/work/v2');",
        )
        .unwrap();
        for (seq, (id, kind, created, data)) in rows.iter().enumerate() {
            conn.execute(
                "INSERT INTO session_message VALUES (?, 's1', ?, ?, ?, ?, ?)",
                params![id, kind, seq as i64, created, created, data],
            )
            .unwrap();
        }
    }

    fn v2_data(ts: u64) -> String {
        format!(
            r#"{{"model":{{"id":"mimo-v2.6-flash","providerID":"opencode-go"}},
            "time":{{"created":{ts}}},"cost":0.125,
            "tokens":{{"input":100,"output":20,"reasoning":30,
            "cache":{{"read":400,"write":50}}}}}}"#
        )
    }

    #[test]
    fn sqlite_v2_only_extracts_usage_and_project() {
        let tmp = TempDir::new().unwrap();
        create_v2_db(
            &tmp.path().join("opencode.db"),
            &[
                ("v2", "assistant", 1700000000000, &v2_data(1700000000000)),
                ("user", "user", 1700000000000, &v2_data(1700000000000)),
                (
                    "event",
                    "model-switched",
                    1700000000000,
                    &v2_data(1700000000000),
                ),
            ],
        );
        let entries = OpenCodeParser::with_base_dir(tmp.path().into())
            .parse_all()
            .unwrap();
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.model.as_deref(), Some("mimo-v2.6-flash"));
        assert_eq!(e.provider.as_deref(), Some("opencode-go"));
        assert_eq!(e.project.as_deref(), Some("/work/v2"));
        assert_eq!(e.message_id.as_deref(), Some("v2"));
        assert_eq!(e.request_id.as_deref(), Some("s1"));
        assert_eq!(e.source.as_deref(), Some("opencode"));
        assert_eq!(e.timestamp.timestamp_millis(), 1700000000000);
        assert_eq!(
            (e.input_tokens, e.output_tokens, e.reasoning_tokens),
            (100, 20, 30)
        );
        assert_eq!((e.cache_read_tokens, e.cache_creation_tokens), (400, 50));
        assert_eq!(e.cost_usd, Some(0.125));
    }

    #[test]
    fn sqlite_v2_migration_deduplicates_but_retains_legacy_only_rows() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("opencode.db");
        let old = assistant_data("old-model", 1700000000000, 10, 5);
        create_test_db(
            &db,
            &[
                ("shared", "s1", 1700000000000, &old),
                ("legacy", "s1", 1700000000000, &old),
            ],
        );
        create_v2_db(
            &db,
            &[
                (
                    "shared",
                    "assistant",
                    1700000000000,
                    &v2_data(1700000000000),
                ),
                ("new", "assistant", 1700000000000, &v2_data(1700000000000)),
            ],
        );
        let parser = OpenCodeParser::with_base_dir(tmp.path().into());
        let entries = parser.parse_all().unwrap();
        assert_eq!(entries.len(), 3);
        let shared = entries
            .iter()
            .find(|e| e.message_id.as_deref() == Some("shared"))
            .unwrap();
        assert_eq!(shared.model.as_deref(), Some("mimo-v2.6-flash"));
        assert_eq!(shared.input_tokens, 100);
        assert_eq!(shared.project.as_deref(), Some("/work/v2"));
        let recent = parser
            .parse_recent_files(UNIX_EPOCH + std::time::Duration::from_millis(1700000000000))
            .unwrap();
        assert_eq!(recent.len(), 3);
    }

    #[test]
    fn sqlite_v2_recent_cutoff_and_missing_session_table() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("opencode.db");
        create_v2_db(
            &db,
            &[
                ("old", "assistant", 1699999999999, &v2_data(1699999999999)),
                (
                    "boundary",
                    "assistant",
                    1700000000000,
                    &v2_data(1700000000000),
                ),
            ],
        );
        Connection::open(&db)
            .unwrap()
            .execute_batch("DROP TABLE session_v2")
            .unwrap();
        let entries = OpenCodeParser::with_base_dir(tmp.path().into())
            .parse_recent_files(UNIX_EPOCH + std::time::Duration::from_millis(1700000000000))
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message_id.as_deref(), Some("boundary"));
        assert_eq!(entries[0].project, None);
    }

    #[test]
    fn sqlite_v2_malformed_rows_do_not_hide_other_usage_or_legacy_fallback() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("opencode.db");
        let old = assistant_data("legacy", 1700000000000, 10, 5);
        create_test_db(&db, &[("shared", "s1", 1700000000000, &old)]);
        let invalid = v2_data(1700000000000).replace("100", "\"bad\"");
        create_v2_db(
            &db,
            &[
                ("shared", "assistant", 1700000000000, &invalid),
                ("broken", "assistant", 1700000000000, "not json"),
                ("good", "assistant", 1700000000000, &v2_data(1700000000000)),
            ],
        );
        let entries = OpenCodeParser::with_base_dir(tmp.path().into())
            .parse_all()
            .unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|e| e.model.as_deref() == Some("legacy")));
        assert!(entries
            .iter()
            .any(|e| e.model.as_deref() == Some("mimo-v2.6-flash")));
    }

    #[test]
    fn sqlite_v2_keeps_cache_only_usage_and_skips_empty_turns() {
        let tmp = TempDir::new().unwrap();
        create_v2_db(
            &tmp.path().join("opencode.db"),
            &[
                (
                    "cache",
                    "assistant",
                    1700000000000,
                    r#"{"model":{"id":"mimo-v2.6-flash"},"time":{"created":1700000000000},"tokens":{"input":0,"output":0,"cache":{"read":400,"write":0}},"cost":0}"#,
                ),
                (
                    "zero",
                    "assistant",
                    1700000000000,
                    r#"{"time":{"created":1700000000000},"tokens":{"input":0,"output":0}}"#,
                ),
                (
                    "pending",
                    "assistant",
                    1700000000000,
                    r#"{"time":{"created":1700000000000}}"#,
                ),
            ],
        );
        let entries = OpenCodeParser::with_base_dir(tmp.path().into())
            .parse_all()
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].cache_read_tokens, 400);
        assert_eq!(entries[0].cost_usd, Some(0.0));
    }

    #[test]
    fn sqlite_v2_failed_legacy_query_does_not_discard_v2_usage() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("opencode.db");
        create_v2_db(
            &db,
            &[("good", "assistant", 1700000000000, &v2_data(1700000000000))],
        );
        Connection::open(&db)
            .unwrap()
            .execute_batch("CREATE TABLE message (id TEXT)")
            .unwrap();
        let entries = OpenCodeParser::with_base_dir(tmp.path().into())
            .parse_all()
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].model.as_deref(), Some("mimo-v2.6-flash"));
    }

    #[test]
    fn sqlite_v2_failed_query_does_not_discard_legacy_usage() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("opencode.db");
        create_test_db(
            &db,
            &[
                (
                    "good",
                    "s1",
                    1700000000000,
                    &assistant_data("legacy", 1700000000000, 10, 5),
                ),
                ("bad", "s1", 1700000000000, "not json"),
            ],
        );
        Connection::open(&db)
            .unwrap()
            .execute_batch("CREATE TABLE session_message (id TEXT)")
            .unwrap();
        let entries = OpenCodeParser::with_base_dir(tmp.path().into())
            .parse_all()
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].model.as_deref(), Some("legacy"));
    }

    #[test]
    fn sqlite_v2_distinct_calls_with_equal_usage_are_not_deduplicated() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("opencode.db");
        let data = v2_data(1700000000000);
        create_v2_db(
            &db,
            &[
                ("a", "assistant", 1700000000000, &data),
                ("b", "assistant", 1700000000000, &data),
            ],
        );
        let entries = OpenCodeParser::with_base_dir(tmp.path().into())
            .parse_all()
            .unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries.iter().map(|e| e.cost_usd.unwrap()).sum::<f64>(),
            0.25
        );
    }

    #[test]
    fn sqlite_v2_counts_compaction_request_usage() {
        let tmp = TempDir::new().unwrap();
        let ts = 1700000000000;
        create_v2_db(
            &tmp.path().join("opencode.db"),
            &[
                ("turn", "assistant", ts, &v2_data(ts as u64)),
                (
                    "compacted",
                    "compaction",
                    ts,
                    r#"{"type":"compaction","status":"completed","reason":"auto",
                        "model":{"id":"claude-sonnet-4-5","providerID":"anthropic"},
                        "summary":"s","recent":"r","time":{"created":1700000000000},
                        "cost":0.4,"tokens":{"input":90000,"output":1500,"reasoning":0,
                        "cache":{"read":0,"write":0}}}"#,
                ),
                (
                    "compact-failed",
                    "compaction",
                    ts,
                    r#"{"type":"compaction","status":"failed","reason":"auto",
                        "error":{"type":"unknown","message":"boom"},
                        "time":{"created":1700000000000},
                        "cost":0.3,"tokens":{"input":80000,"output":0,"reasoning":0,
                        "cache":{"read":0,"write":0}}}"#,
                ),
                (
                    "compacting",
                    "compaction",
                    ts,
                    r#"{"type":"compaction","status":"running","reason":"manual",
                        "summary":"","recent":"","time":{"created":1700000000000}}"#,
                ),
            ],
        );
        let entries = OpenCodeParser::with_base_dir(tmp.path().into())
            .parse_all()
            .unwrap();

        let mut ids: Vec<&str> = entries
            .iter()
            .filter_map(|e| e.message_id.as_deref())
            .collect();
        ids.sort_unstable();
        assert_eq!(ids, ["compact-failed", "compacted", "turn"]);

        let completed = entries
            .iter()
            .find(|e| e.message_id.as_deref() == Some("compacted"))
            .unwrap();
        assert_eq!(completed.model.as_deref(), Some("claude-sonnet-4-5"));
        assert_eq!(completed.provider.as_deref(), Some("anthropic"));
        assert_eq!(completed.project.as_deref(), Some("/work/v2"));
        assert_eq!(
            (completed.input_tokens, completed.output_tokens),
            (90000, 1500)
        );
        assert_eq!(completed.cost_usd, Some(0.4));

        // Failed compactions carry no model in OpenCode's schema, but the
        // request was still billed.
        let failed = entries
            .iter()
            .find(|e| e.message_id.as_deref() == Some("compact-failed"))
            .unwrap();
        assert_eq!(failed.model, None);
        assert_eq!(failed.input_tokens, 80000);
        assert_eq!(failed.cost_usd, Some(0.3));
    }

    #[test]
    fn sqlite_non_text_directory_costs_only_that_rows_project() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("opencode.db");
        let ts = 1700000000000;
        create_v2_db(
            &db,
            &[
                ("in-project", "assistant", ts, &v2_data(ts as u64)),
                ("odd-session", "assistant", ts, &v2_data(ts as u64)),
            ],
        );
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "INSERT INTO session_v2 VALUES ('s2', X'00');
             UPDATE session_message SET session_id = 's2' WHERE id = 'odd-session';",
        )
        .unwrap();

        let entries = OpenCodeParser::with_base_dir(tmp.path().into())
            .parse_all()
            .unwrap();

        assert_eq!(entries.len(), 2);
        let project_of = |id: &str| {
            entries
                .iter()
                .find(|e| e.message_id.as_deref() == Some(id))
                .unwrap()
                .project
                .clone()
        };
        assert_eq!(project_of("in-project").as_deref(), Some("/work/v2"));
        assert_eq!(project_of("odd-session"), None);
    }

    #[test]
    fn sqlite_session_join_failing_mid_scan_keeps_usage() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("opencode.db");
        let ts = 1700000000000;
        create_v2_db(
            &db,
            &[
                ("first", "assistant", ts, &v2_data(ts as u64)),
                ("second", "assistant", ts, &v2_data(ts as u64)),
            ],
        );
        // Swap the session table for a view whose directory errors at read
        // time (abs of i64::MIN overflows), so the joined query prepares fine
        // and fails only once the scan reaches that session.
        Connection::open(&db)
            .unwrap()
            .execute_batch(
                "ALTER TABLE session_v2 RENAME TO session_rows;
                 CREATE VIEW session_v2 AS SELECT id,
                   CASE WHEN id = 's1' THEN abs(-9223372036854775807 - 1) ELSE directory END
                   AS directory FROM session_rows;",
            )
            .unwrap();

        let entries = OpenCodeParser::with_base_dir(tmp.path().into())
            .parse_all()
            .unwrap();

        let mut ids: Vec<&str> = entries
            .iter()
            .filter_map(|e| e.message_id.as_deref())
            .collect();
        ids.sort_unstable();
        assert_eq!(ids, ["first", "second"]);
    }
}
