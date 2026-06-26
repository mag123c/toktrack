//! Antigravity IDE / CLI parser.
//!
//! Antigravity (Google's agentic IDE, built on Codeium/Windsurf tech) persists
//! agent token usage locally in **per-conversation SQLite databases**, as
//! **protobuf blobs** — not JSON. One DB per trajectory:
//! - IDE: `~/.gemini/antigravity-ide/conversations/<trajectory_id>.db`
//! - CLI: `~/.gemini/antigravity-cli/conversations/<trajectory_id>.db`
//!
//! Token usage lives in the `gen_metadata` table (one row per model
//! generation). Each `data` blob is a wrapper message:
//! - field 1  `metadata` → `ChatModelMetadata`
//! - field 4  `generation_id` (uuid string; not used)
//!
//! The `ChatModelMetadata` (field 1) carries:
//! - field 4  `usage` → `ModelUsageStats`
//!     - 2 `input_tokens`            (non-cached input)
//!     - 3 `output_tokens`          (= thinking + response; not used directly)
//!     - 4 `cache_write_tokens`
//!     - 5 `cache_read_tokens`
//!     - 9 `thinking_output_tokens` (reasoning)
//!     - 10 `response_output_tokens` (visible output)
//!     - 11 `response_id`           (string; unique per generation)
//! - field 9  `chat_start_metadata` → field 4 `Timestamp {1 seconds, 2 nanos}`
//! - field 19 `response_model`      (string model id, e.g. `gemini-3-flash-a`)
//!
//! The session's working directory (used as the project key) is stored once per
//! DB in `trajectory_metadata_blob` (field 7, or field 1→1) as a `file://` URI.
//!
//! The field numbers were reverse-engineered (2026-06-24) from the Antigravity
//! IDE language-server binary's embedded proto descriptors
//! (`resources/app/extensions/antigravity/bin/language_server_*`) and validated
//! against real sessions (`output_tokens == thinking + response`). The binary
//! exposes no semantic Antigravity version, so this layout is pinned to that
//! extraction; field numbers may drift across versions, so decoding is defensive:
//! any malformed/under-specified blob is skipped rather than panicking.

use crate::types::{Result, UsageEntry};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OpenFlags};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use super::CLIParser;

/// Minimal protobuf wire-format reader. Only what we need: iterate top-level
/// fields and pluck the first varint / length-delimited value by field number.
/// No schema, no allocation — slices borrow the input buffer.
mod proto {
    /// A single decoded wire value. Fixed-width variants are retained only so
    /// iteration stays in sync; the getters ignore them.
    enum Wire<'a> {
        Varint(u64),
        Bytes(&'a [u8]),
        /// 32/64-bit fixed fields. We never read the value — the variant exists
        /// only so iteration stays byte-aligned past such fields.
        Fixed,
    }

    fn read_varint(buf: &[u8], pos: &mut usize) -> Option<u64> {
        let mut result = 0u64;
        let mut shift = 0u32;
        loop {
            let byte = *buf.get(*pos)?;
            *pos += 1;
            result |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(result);
            }
            shift += 7;
            if shift >= 64 {
                return None;
            }
        }
    }

    struct Reader<'a> {
        buf: &'a [u8],
        pos: usize,
    }

    impl<'a> Iterator for Reader<'a> {
        type Item = (u64, Wire<'a>);

        fn next(&mut self) -> Option<Self::Item> {
            if self.pos >= self.buf.len() {
                return None;
            }
            let tag = read_varint(self.buf, &mut self.pos)?;
            let field = tag >> 3;
            let value = match tag & 7 {
                0 => Wire::Varint(read_varint(self.buf, &mut self.pos)?),
                1 => {
                    let end = self.pos.checked_add(8)?;
                    if end > self.buf.len() {
                        return None;
                    }
                    self.pos = end;
                    Wire::Fixed
                }
                2 => {
                    let len = read_varint(self.buf, &mut self.pos)? as usize;
                    let end = self.pos.checked_add(len)?;
                    let slice = self.buf.get(self.pos..end)?;
                    self.pos = end;
                    Wire::Bytes(slice)
                }
                5 => {
                    let end = self.pos.checked_add(4)?;
                    if end > self.buf.len() {
                        return None;
                    }
                    self.pos = end;
                    Wire::Fixed
                }
                _ => return None, // unknown wire type (groups 3/4 or reserved 6/7): no schema to skip it, so stop
            };
            Some((field, value))
        }
    }

    fn fields(buf: &[u8]) -> Reader<'_> {
        Reader { buf, pos: 0 }
    }

    /// First varint value for `field` (0 if absent — callers treat missing
    /// token counts as zero). First-match is safe here: each field appears at
    /// most once per blob, so protobuf's last-wins rule for singular scalars
    /// never matters.
    pub fn varint(buf: &[u8], field: u64) -> Option<u64> {
        fields(buf).find_map(|(f, v)| match v {
            Wire::Varint(n) if f == field => Some(n),
            _ => None,
        })
    }

    /// First length-delimited value (sub-message or string bytes) for `field`.
    /// Like `varint`, first-match is safe: each field occurs at most once per blob.
    pub fn bytes(buf: &[u8], field: u64) -> Option<&[u8]> {
        fields(buf).find_map(|(f, v)| match v {
            Wire::Bytes(b) if f == field => Some(b),
            _ => None,
        })
    }

    /// First length-delimited value for `field` decoded as UTF-8.
    pub fn string(buf: &[u8], field: u64) -> Option<&str> {
        bytes(buf, field).and_then(|b| std::str::from_utf8(b).ok())
    }
}

/// Parser for Antigravity IDE/CLI usage data.
pub struct AntigravityParser {
    /// `~/.gemini` root (honors `GEMINI_CLI_HOME`). Conversation DBs live under
    /// `antigravity-ide/conversations/` and `antigravity-cli/conversations/`.
    data_dir: PathBuf,
    source: &'static str,
}

impl AntigravityParser {
    /// Create a parser with the default data directory (`~/.gemini`, or
    /// `$GEMINI_CLI_HOME/.gemini`).
    pub fn new() -> Self {
        let base = super::discovery::first_env_dir(&["GEMINI_CLI_HOME"])
            .or_else(|| directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf()))
            .unwrap_or_else(|| {
                eprintln!("[toktrack] Warning: Could not determine home directory");
                PathBuf::from(".")
            });
        Self::with_data_dir(base.join(".gemini"))
    }

    /// Create a parser rooted at a custom `.gemini`-style directory (the dir that
    /// directly contains `antigravity-ide/` and/or `antigravity-cli/`).
    pub fn with_data_dir(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            source: "antigravity",
        }
    }

    /// Read every `gen_metadata` blob from one conversation DB and decode it into
    /// a `UsageEntry`. Open failures and undecodable rows yield nothing (warned,
    /// never fatal) so one corrupt DB can't sink the whole scan.
    fn parse_db(&self, path: &Path) -> Vec<UsageEntry> {
        let conn = match Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "[toktrack] Warning: failed to open Antigravity db {:?}: {}",
                    path, e
                );
                return Vec::new();
            }
        };

        let project = read_project(&conn);
        let trajectory_id = read_trajectory_id(&conn);
        let fallback_ts = file_mtime(path);

        read_gen_metadata(&conn)
            .into_iter()
            .filter_map(|data| {
                decode_entry(
                    &data,
                    project.as_deref(),
                    trajectory_id.as_deref(),
                    fallback_ts,
                    self.source,
                )
            })
            .collect()
    }
}

/// Decode one `gen_metadata` blob (`ChatModelMetadata`) into a `UsageEntry`.
/// Returns `None` for blobs missing the usage sub-message or carrying no tokens.
fn decode_entry(
    data: &[u8],
    project: Option<&str>,
    trajectory_id: Option<&str>,
    fallback_ts: DateTime<Utc>,
    source: &'static str,
) -> Option<UsageEntry> {
    let cmm = proto::bytes(data, 1)?; // ChatModelMetadata
    let usage = proto::bytes(cmm, 4)?; // ModelUsageStats

    let input_tokens = proto::varint(usage, 2).unwrap_or(0);
    let cache_creation_tokens = proto::varint(usage, 4).unwrap_or(0);
    let cache_read_tokens = proto::varint(usage, 5).unwrap_or(0);
    let reasoning_tokens = proto::varint(usage, 9).unwrap_or(0);
    let output_tokens = proto::varint(usage, 10).unwrap_or(0);

    // Skip rows with no usage (e.g. interrupted/empty generations). Sum with
    // `saturating_add`: the counts are raw varints from an untrusted blob, so a
    // corrupt DB could carry oversized values that would overflow a plain `+`
    // (panic in debug, wrap in release) — the module contract is skip, never panic.
    let total = [
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_creation_tokens,
        reasoning_tokens,
    ]
    .into_iter()
    .fold(0u64, u64::saturating_add);
    if total == 0 {
        return None;
    }

    let message_id = proto::string(usage, 11).map(String::from);
    let model = proto::string(cmm, 19).map(String::from);
    let timestamp = decode_timestamp(cmm).unwrap_or(fallback_ts);

    Some(UsageEntry {
        timestamp,
        model,
        // `input_tokens` is already the non-cached input (cache reads are a
        // separate field), matching the v2 contract — no subtraction needed.
        input_tokens,
        // `response_output_tokens` is the visible output; `thinking_output_tokens`
        // is reasoning. Their sum equals ModelUsageStats `output_tokens` (f3), so
        // splitting here keeps the contract (output excludes reasoning) without
        // double-counting.
        output_tokens,
        cache_read_tokens,
        cache_creation_tokens,
        reasoning_tokens,
        cache_creation_5m_tokens: 0,
        cache_creation_1h_tokens: 0,
        web_search_requests: 0,
        web_fetch_requests: 0,
        reported_total_tokens: None,
        cost_usd: None,
        message_id,
        request_id: trajectory_id.map(String::from),
        source: Some(source.into()),
        provider: None,
        project: project.map(String::from),
    })
}

/// `ChatModelMetadata.chat_start_metadata(9).timestamp(4) = {seconds(1), nanos(2)}`.
fn decode_timestamp(cmm: &[u8]) -> Option<DateTime<Utc>> {
    let start = proto::bytes(cmm, 9)?;
    let ts = proto::bytes(start, 4)?;
    let seconds = i64::try_from(proto::varint(ts, 1)?).ok()?;
    let nanos = u32::try_from(proto::varint(ts, 2).unwrap_or(0)).unwrap_or(0);
    DateTime::from_timestamp(seconds, nanos)
}

/// Workspace folder URI from `trajectory_metadata_blob` (field 7, or field 1→1),
/// normalized to a plain path for use as the project key.
fn read_project(conn: &Connection) -> Option<String> {
    let blob: Vec<u8> = conn
        .query_row(
            "SELECT data FROM trajectory_metadata_blob LIMIT 1",
            [],
            |r| r.get(0),
        )
        .ok()?;
    let uri = proto::string(&blob, 7)
        .or_else(|| proto::bytes(&blob, 1).and_then(|m| proto::string(m, 1)))?;
    Some(workspace_path(uri))
}

/// Trajectory id (used as `request_id` for dedup), if the table is present.
fn read_trajectory_id(conn: &Connection) -> Option<String> {
    conn.query_row(
        "SELECT trajectory_id FROM trajectory_meta LIMIT 1",
        [],
        |r| r.get(0),
    )
    .ok()
}

/// All `gen_metadata` blobs in insertion order. Missing table → empty.
fn read_gen_metadata(conn: &Connection) -> Vec<Vec<u8>> {
    let mut stmt = match conn.prepare("SELECT data FROM gen_metadata ORDER BY idx") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = match stmt.query_map([], |r| r.get::<_, Vec<u8>>(0)) {
        Ok(rows) => rows,
        Err(_) => return Vec::new(),
    };
    rows.filter_map(std::result::Result::ok).collect()
}

/// Strip the `file://` scheme and a leading slash before a Windows drive letter,
/// e.g. `file:///g:/Scripts/x` → `g:/Scripts/x`; `file:///home/x` → `/home/x`.
fn workspace_path(uri: &str) -> String {
    let p = uri.strip_prefix("file://").unwrap_or(uri);
    let bytes = p.as_bytes();
    if bytes.len() >= 3 && bytes[0] == b'/' && bytes[2] == b':' {
        p[1..].to_string()
    } else {
        p.to_string()
    }
}

/// DB file mtime as a UTC datetime; falls back to `now` if unavailable. Used only
/// when a row's embedded timestamp is missing.
fn file_mtime(path: &Path) -> DateTime<Utc> {
    path.metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .and_then(|d| DateTime::from_timestamp(d.as_secs() as i64, 0))
        .unwrap_or_else(Utc::now)
}

impl Default for AntigravityParser {
    fn default() -> Self {
        Self::new()
    }
}

impl CLIParser for AntigravityParser {
    fn name(&self) -> &str {
        self.source
    }

    fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Representative pattern. Actual collection globs both the IDE and CLI
    /// `conversations/` directories via `collect_files`.
    fn file_pattern(&self) -> &str {
        "antigravity-ide/conversations/*.db"
    }

    fn collect_files(&self) -> Vec<PathBuf> {
        let patterns = [
            "antigravity-ide/conversations/*.db",
            "antigravity-cli/conversations/*.db",
        ];
        let mut seen: HashSet<PathBuf> = HashSet::new();
        let mut out = Vec::new();
        for pat in patterns {
            let full = self.data_dir.join(pat);
            if let Ok(paths) = glob::glob(&full.to_string_lossy()) {
                for p in paths.filter_map(std::result::Result::ok) {
                    if seen.insert(p.clone()) {
                        out.push(p);
                    }
                }
            }
        }
        out
    }

    fn parse_file(&self, path: &Path) -> Result<Vec<UsageEntry>> {
        Ok(self.parse_db(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use tempfile::TempDir;

    // ===== protobuf encode helpers (build fixture blobs) =====

    fn enc_varint(out: &mut Vec<u8>, mut n: u64) {
        loop {
            let mut byte = (n & 0x7f) as u8;
            n >>= 7;
            if n != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if n == 0 {
                break;
            }
        }
    }

    fn field_varint(out: &mut Vec<u8>, field: u64, n: u64) {
        enc_varint(out, field << 3);
        enc_varint(out, n);
    }

    fn field_bytes(out: &mut Vec<u8>, field: u64, b: &[u8]) {
        enc_varint(out, (field << 3) | 2);
        enc_varint(out, b.len() as u64);
        out.extend_from_slice(b);
    }

    #[allow(clippy::too_many_arguments)]
    fn model_usage_stats(
        input: u64,
        response_out: u64,
        thinking: u64,
        cache_read: u64,
        cache_write: u64,
        response_id: &str,
    ) -> Vec<u8> {
        let mut u = Vec::new();
        field_varint(&mut u, 2, input);
        field_varint(&mut u, 3, thinking + response_out); // output_tokens (= think+resp)
        if cache_write > 0 {
            field_varint(&mut u, 4, cache_write);
        }
        if cache_read > 0 {
            field_varint(&mut u, 5, cache_read);
        }
        field_varint(&mut u, 9, thinking);
        field_varint(&mut u, 10, response_out);
        field_bytes(&mut u, 11, response_id.as_bytes());
        u
    }

    fn chat_start(seconds: u64, nanos: u64) -> Vec<u8> {
        let mut ts = Vec::new();
        field_varint(&mut ts, 1, seconds);
        field_varint(&mut ts, 2, nanos);
        let mut start = Vec::new();
        field_bytes(&mut start, 4, &ts);
        start
    }

    fn chat_model_metadata(usage: &[u8], start: &[u8], model: &str) -> Vec<u8> {
        let mut m = Vec::new();
        field_bytes(&mut m, 4, usage);
        field_bytes(&mut m, 9, start);
        field_bytes(&mut m, 19, model.as_bytes());
        m
    }

    /// Top-level `gen_metadata.data`: field 1 = ChatModelMetadata, field 4 = uuid.
    fn gen_blob(cmm: &[u8], uuid: &str) -> Vec<u8> {
        let mut g = Vec::new();
        field_bytes(&mut g, 1, cmm);
        field_bytes(&mut g, 4, uuid.as_bytes());
        g
    }

    fn traj_meta_blob(uri: &str) -> Vec<u8> {
        let mut inner = Vec::new();
        field_bytes(&mut inner, 1, uri.as_bytes());
        let mut t = Vec::new();
        field_bytes(&mut t, 1, &inner);
        field_bytes(&mut t, 7, uri.as_bytes());
        t
    }

    // ===== fixture DB builder =====

    struct Row {
        idx: i64,
        data: Vec<u8>,
    }

    fn seed_db(root: &Path, variant: &str, db_name: &str, uri: &str, traj_id: &str, rows: &[Row]) {
        seed_db_with_traj(root, variant, db_name, &traj_meta_blob(uri), traj_id, rows);
    }

    fn seed_db_with_traj(
        root: &Path,
        variant: &str,
        db_name: &str,
        traj_blob: &[u8],
        traj_id: &str,
        rows: &[Row],
    ) {
        let dir = root.join(variant).join("conversations");
        std::fs::create_dir_all(&dir).unwrap();
        let conn = Connection::open(dir.join(db_name)).unwrap();
        conn.execute_batch(
            "CREATE TABLE gen_metadata (idx INTEGER PRIMARY KEY, data BLOB, size INTEGER);
             CREATE TABLE trajectory_metadata_blob (id TEXT PRIMARY KEY, data BLOB);
             CREATE TABLE trajectory_meta (trajectory_id TEXT, cascade_id TEXT, \
               trajectory_type INTEGER, source INTEGER);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO trajectory_metadata_blob (id, data) VALUES ('main', ?)",
            params![traj_blob],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO trajectory_meta (trajectory_id, cascade_id, trajectory_type, source) \
             VALUES (?, 'c', 4, 1)",
            params![traj_id],
        )
        .unwrap();
        for r in rows {
            conn.execute(
                "INSERT INTO gen_metadata (idx, data, size) VALUES (?, ?, ?)",
                params![r.idx, r.data, r.data.len() as i64],
            )
            .unwrap();
        }
    }

    /// Like [`seed_db`] but omits the `trajectory_meta` table entirely, mirroring
    /// older/partial DBs where `read_trajectory_id` returns `None` so `request_id`
    /// is `None` and dedup falls back to the content arm.
    fn seed_db_no_traj_meta(root: &Path, variant: &str, db_name: &str, uri: &str, rows: &[Row]) {
        let dir = root.join(variant).join("conversations");
        std::fs::create_dir_all(&dir).unwrap();
        let conn = Connection::open(dir.join(db_name)).unwrap();
        conn.execute_batch(
            "CREATE TABLE gen_metadata (idx INTEGER PRIMARY KEY, data BLOB, size INTEGER);
             CREATE TABLE trajectory_metadata_blob (id TEXT PRIMARY KEY, data BLOB);",
        )
        .unwrap();
        let blob = traj_meta_blob(uri);
        conn.execute(
            "INSERT INTO trajectory_metadata_blob (id, data) VALUES ('main', ?)",
            params![blob],
        )
        .unwrap();
        for r in rows {
            conn.execute(
                "INSERT INTO gen_metadata (idx, data, size) VALUES (?, ?, ?)",
                params![r.idx, r.data, r.data.len() as i64],
            )
            .unwrap();
        }
    }

    /// One realistic assistant generation, mirroring real-session field layout.
    fn usage_row(idx: i64, input: u64, resp: u64, think: u64, cr: u64, cw: u64, id: &str) -> Row {
        let usage = model_usage_stats(input, resp, think, cr, cw, id);
        let start = chat_start(1_782_298_884 + idx as u64, 0);
        let cmm = chat_model_metadata(&usage, &start, "gemini-3-flash-a");
        Row {
            idx,
            data: gen_blob(&cmm, &format!("uuid-{idx}")),
        }
    }

    /// ChatModelMetadata with optionally-omitted usage (f4) / start (f9) — used to
    /// exercise the missing-usage skip and the timestamp→mtime fallback branches.
    fn cmm_parts(usage: Option<&[u8]>, start: Option<&[u8]>, model: &str) -> Vec<u8> {
        let mut m = Vec::new();
        if let Some(u) = usage {
            field_bytes(&mut m, 4, u);
        }
        if let Some(s) = start {
            field_bytes(&mut m, 9, s);
        }
        field_bytes(&mut m, 19, model.as_bytes());
        m
    }

    /// trajectory_metadata_blob carrying ONLY the nested field 1→1 URI (older
    /// layout, no field 7) — exercises the fallback path in `read_project`.
    fn traj_meta_blob_field1_only(uri: &str) -> Vec<u8> {
        let mut inner = Vec::new();
        field_bytes(&mut inner, 1, uri.as_bytes());
        let mut t = Vec::new();
        field_bytes(&mut t, 1, &inner);
        t
    }

    // ===== tests =====

    #[test]
    fn test_name_and_data_dir() {
        // Do NOT mutate the process-global `GEMINI_CLI_HOME` here: it races with
        // gemini.rs::test_gemini_cli_home_env_var (cargo runs both in one test
        // binary in parallel). The override path is covered by gemini.rs and
        // discovery.rs, since `new()` resolves it through the shared
        // `discovery::first_env_dir(&["GEMINI_CLI_HOME"])`.
        let parser = AntigravityParser::with_data_dir(PathBuf::from("/tmp/toktrack-ag/.gemini"));
        assert_eq!(parser.name(), "antigravity");
        assert_eq!(parser.data_dir(), Path::new("/tmp/toktrack-ag/.gemini"));

        // `new()` only reads env (never writes), and the joined `.gemini` suffix
        // holds regardless of `GEMINI_CLI_HOME`'s value, so this is race-safe.
        assert!(AntigravityParser::new().data_dir().ends_with(".gemini"));
    }

    #[test]
    fn test_token_mapping_and_invariant() {
        let tmp = TempDir::new().unwrap();
        seed_db(
            tmp.path(),
            "antigravity-ide",
            "t.db",
            "file:///g:/Scripts/proj",
            "traj-1",
            &[usage_row(1, 4626, 1474, 577, 12208, 0, "resp-a")],
        );
        let parser = AntigravityParser::with_data_dir(tmp.path().to_path_buf());
        let entries = parser.parse_all().unwrap();

        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.model.as_deref(), Some("gemini-3-flash-a"));
        assert_eq!(e.input_tokens, 4626);
        assert_eq!(e.output_tokens, 1474); // response_output_tokens (visible only)
        assert_eq!(e.reasoning_tokens, 577); // thinking_output_tokens
        assert_eq!(e.cache_read_tokens, 12208);
        assert_eq!(e.cache_creation_tokens, 0);
        // total = 4626 + 1474 + 12208 + 0 + 577
        assert_eq!(e.total_tokens(), 18885);
        assert_eq!(e.source.as_deref(), Some("antigravity"));
        assert_eq!(e.message_id.as_deref(), Some("resp-a"));
        assert_eq!(e.request_id.as_deref(), Some("traj-1"));
        // file:///g:/... → g:/... (drive-letter slash stripped)
        assert_eq!(e.project.as_deref(), Some("g:/Scripts/proj"));
        // timestamp decoded from chat_start_metadata (1_782_298_884 + idx)
        assert_eq!(e.timestamp.timestamp(), 1_782_298_885);

        // Invariant the test name claims: ModelUsageStats.output_tokens (f3) ==
        // thinking_output_tokens (f9) + response_output_tokens (f10). We expose
        // those two as reasoning_tokens and output_tokens, so their sum must
        // reconcile with the upstream aggregate — guards a future split that
        // could double-count or drop reasoning. (Asserted here, not via a
        // debug_assert in decode_entry, to preserve the skip-never-panic
        // contract on untrusted blobs.)
        let usage = model_usage_stats(4626, 1474, 577, 12208, 0, "resp-a");
        assert_eq!(
            proto::varint(&usage, 3),
            Some(e.output_tokens + e.reasoning_tokens)
        );
    }

    #[test]
    fn test_cache_write_mapped() {
        let tmp = TempDir::new().unwrap();
        seed_db(
            tmp.path(),
            "antigravity-ide",
            "t.db",
            "file:///work/x",
            "traj-1",
            &[usage_row(1, 100, 50, 10, 0, 77, "r")],
        );
        let parser = AntigravityParser::with_data_dir(tmp.path().to_path_buf());
        let entries = parser.parse_all().unwrap();
        assert_eq!(entries[0].cache_creation_tokens, 77);
        // unix-style path keeps its leading slash
        assert_eq!(entries[0].project.as_deref(), Some("/work/x"));
    }

    #[test]
    fn test_zero_token_rows_skipped() {
        let tmp = TempDir::new().unwrap();
        seed_db(
            tmp.path(),
            "antigravity-ide",
            "t.db",
            "file:///work/x",
            "traj-1",
            &[
                usage_row(1, 0, 0, 0, 0, 0, "zero"),
                usage_row(2, 10, 5, 0, 0, 0, "real"),
            ],
        );
        let parser = AntigravityParser::with_data_dir(tmp.path().to_path_buf());
        let entries = parser.parse_all().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message_id.as_deref(), Some("real"));
    }

    #[test]
    fn test_malformed_blob_skipped_good_kept() {
        let tmp = TempDir::new().unwrap();
        seed_db(
            tmp.path(),
            "antigravity-ide",
            "t.db",
            "file:///work/x",
            "traj-1",
            &[
                Row {
                    idx: 1,
                    data: vec![0xff, 0xff, 0xff], // not valid protobuf
                },
                usage_row(2, 10, 5, 0, 0, 0, "good"),
            ],
        );
        let parser = AntigravityParser::with_data_dir(tmp.path().to_path_buf());
        let entries = parser.parse_all().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message_id.as_deref(), Some("good"));
    }

    #[test]
    fn test_open_failure_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("antigravity-ide").join("conversations");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("bad.db"), b"not a sqlite file").unwrap();
        let parser = AntigravityParser::with_data_dir(tmp.path().to_path_buf());
        let entries = parser.parse_all().unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_collect_files_finds_ide_and_cli() {
        let tmp = TempDir::new().unwrap();
        seed_db(
            tmp.path(),
            "antigravity-ide",
            "ide.db",
            "file:///work/x",
            "traj-ide",
            &[usage_row(1, 10, 5, 0, 0, 0, "i")],
        );
        seed_db(
            tmp.path(),
            "antigravity-cli",
            "cli.db",
            "file:///work/y",
            "traj-cli",
            &[usage_row(1, 10, 5, 0, 0, 0, "c")],
        );
        let parser = AntigravityParser::with_data_dir(tmp.path().to_path_buf());
        let files = parser.collect_files();
        assert_eq!(files.len(), 2, "got: {files:?}");
        assert!(files.iter().any(|f| f.to_string_lossy().contains("ide")));
        assert!(files.iter().any(|f| f.to_string_lossy().contains("cli")));
    }

    #[test]
    fn test_parse_all_aggregates_and_dedups() {
        let tmp = TempDir::new().unwrap();
        // Two distinct generations in IDE + one in CLI = 3 entries.
        seed_db(
            tmp.path(),
            "antigravity-ide",
            "ide.db",
            "file:///work/x",
            "traj-ide",
            &[
                usage_row(1, 10, 5, 0, 0, 0, "a"),
                usage_row(2, 20, 7, 0, 0, 0, "b"),
            ],
        );
        seed_db(
            tmp.path(),
            "antigravity-cli",
            "cli.db",
            "file:///work/y",
            "traj-cli",
            &[usage_row(1, 30, 9, 0, 0, 0, "c")],
        );
        let parser = AntigravityParser::with_data_dir(tmp.path().to_path_buf());
        let entries = parser.parse_all().unwrap();
        assert_eq!(entries.len(), 3);
        assert!(entries
            .iter()
            .all(|e| e.source.as_deref() == Some("antigravity")));
    }

    #[test]
    fn test_dedup_same_response_id_within_db_collapses() {
        // Two rows in ONE DB sharing a response_id have the same dedup key
        // (response_id:trajectory_id, since trajectory_id is constant per DB),
        // so they collapse to a single entry.
        let tmp = TempDir::new().unwrap();
        seed_db(
            tmp.path(),
            "antigravity-ide",
            "t.db",
            "file:///work/x",
            "traj-1",
            &[
                usage_row(1, 10, 5, 0, 0, 0, "dup"),
                usage_row(2, 20, 7, 0, 0, 0, "dup"),
            ],
        );
        let parser = AntigravityParser::with_data_dir(tmp.path().to_path_buf());
        assert_eq!(parser.parse_all().unwrap().len(), 1);
    }

    #[test]
    fn test_same_response_id_across_ide_and_cli_both_survive() {
        // The same response_id in two DBs with DIFFERENT trajectory_ids yields
        // distinct dedup keys, so both are kept. Pins the intended cross-source
        // behavior so a future trajectory_id change can't silently collapse them.
        let tmp = TempDir::new().unwrap();
        seed_db(
            tmp.path(),
            "antigravity-ide",
            "ide.db",
            "file:///work/x",
            "traj-ide",
            &[usage_row(1, 10, 5, 0, 0, 0, "same")],
        );
        seed_db(
            tmp.path(),
            "antigravity-cli",
            "cli.db",
            "file:///work/y",
            "traj-cli",
            &[usage_row(1, 10, 5, 0, 0, 0, "same")],
        );
        let parser = AntigravityParser::with_data_dir(tmp.path().to_path_buf());
        assert_eq!(parser.parse_all().unwrap().len(), 2);
    }

    #[test]
    fn test_missing_trajectory_meta_falls_back_to_content_dedup() {
        // No `trajectory_meta` table => `request_id` is `None` for every row, so
        // dedup uses the content arm (`response_id:model:input:output`). Two such
        // DBs with distinct response_ids both survive. (Two trajectory_meta-less
        // DBs sharing a response_id AND identical counts would collapse cross-DB,
        // but that is a non-issue in practice: response_ids are server-issued UUIDs.)
        let tmp = TempDir::new().unwrap();
        seed_db_no_traj_meta(
            tmp.path(),
            "antigravity-ide",
            "ide.db",
            "file:///work/x",
            &[usage_row(1, 10, 5, 0, 0, 0, "r1")],
        );
        seed_db_no_traj_meta(
            tmp.path(),
            "antigravity-cli",
            "cli.db",
            "file:///work/y",
            &[usage_row(1, 10, 5, 0, 0, 0, "r2")],
        );
        let parser = AntigravityParser::with_data_dir(tmp.path().to_path_buf());
        assert_eq!(parser.parse_all().unwrap().len(), 2);
    }

    #[test]
    fn test_missing_usage_submessage_skipped() {
        // A valid ChatModelMetadata with model + timestamp but NO usage (f4) is
        // skipped — distinct from the malformed-blob and zero-token paths.
        let tmp = TempDir::new().unwrap();
        let start = chat_start(1_782_298_900, 0);
        let no_usage = cmm_parts(None, Some(&start), "gemini-3-flash-a");
        seed_db(
            tmp.path(),
            "antigravity-ide",
            "t.db",
            "file:///work/x",
            "traj-1",
            &[
                Row {
                    idx: 1,
                    data: gen_blob(&no_usage, "uuid-1"),
                },
                usage_row(2, 10, 5, 0, 0, 0, "good"),
            ],
        );
        let parser = AntigravityParser::with_data_dir(tmp.path().to_path_buf());
        let entries = parser.parse_all().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message_id.as_deref(), Some("good"));
    }

    #[test]
    fn test_missing_timestamp_falls_back_to_file_mtime() {
        // No chat_start (f9) → decode_timestamp returns None → timestamp falls back
        // to the DB file's mtime.
        let tmp = TempDir::new().unwrap();
        let usage = model_usage_stats(10, 5, 0, 0, 0, "r");
        let no_start = cmm_parts(Some(&usage), None, "gemini-3-flash-a");
        seed_db(
            tmp.path(),
            "antigravity-ide",
            "t.db",
            "file:///work/x",
            "traj-1",
            &[Row {
                idx: 1,
                data: gen_blob(&no_start, "uuid-1"),
            }],
        );
        let db = tmp
            .path()
            .join("antigravity-ide")
            .join("conversations")
            .join("t.db");
        let mtime_secs = std::fs::metadata(&db)
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let parser = AntigravityParser::with_data_dir(tmp.path().to_path_buf());
        let entries = parser.parse_all().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].timestamp.timestamp(), mtime_secs);
    }

    #[test]
    fn test_project_resolves_via_field1_fallback() {
        // Older layout: trajectory_metadata_blob has ONLY the nested field 1→1 URI
        // (no field 7). The project must still resolve.
        let tmp = TempDir::new().unwrap();
        seed_db_with_traj(
            tmp.path(),
            "antigravity-ide",
            "t.db",
            &traj_meta_blob_field1_only("file:///g:/Scripts/proj"),
            "traj-1",
            &[usage_row(1, 10, 5, 0, 0, 0, "r")],
        );
        let parser = AntigravityParser::with_data_dir(tmp.path().to_path_buf());
        let entries = parser.parse_all().unwrap();
        assert_eq!(entries[0].project.as_deref(), Some("g:/Scripts/proj"));
    }

    #[test]
    fn test_oversized_token_varints_do_not_panic() {
        // Corrupt/adversarial usage counts near u64::MAX must not overflow-panic
        // the zero-check sum; the row is still produced.
        let tmp = TempDir::new().unwrap();
        // Build the usage blob directly (model_usage_stats would overflow its own
        // f3 = thinking + response computation with MAX inputs).
        let mut usage = Vec::new();
        field_varint(&mut usage, 2, u64::MAX); // input
        field_varint(&mut usage, 10, u64::MAX); // response output
        field_varint(&mut usage, 5, u64::MAX); // cache read
        field_bytes(&mut usage, 11, b"big");
        let cmm = chat_model_metadata(&usage, &chat_start(1_782_298_900, 0), "gemini-3-flash-a");
        seed_db(
            tmp.path(),
            "antigravity-ide",
            "t.db",
            "file:///work/x",
            "traj-1",
            &[Row {
                idx: 1,
                data: gen_blob(&cmm, "uuid-1"),
            }],
        );
        let parser = AntigravityParser::with_data_dir(tmp.path().to_path_buf());
        let entries = parser.parse_all().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].input_tokens, u64::MAX);
    }
}
