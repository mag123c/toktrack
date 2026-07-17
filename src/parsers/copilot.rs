//! GitHub Copilot CLI (`~/.copilot/session-state/*/events.jsonl`) parser.
//!
//! New Copilot CLI sessions no longer write Codex `token_count` events under
//! `~/.codex/sessions`. Instead they emit `assistant.message` events with
//! per-turn `outputTokens`, and (on session end) a `session.shutdown` event
//! with authoritative `modelMetrics` totals.
//!
//! Counting policy:
//! - If a file has `session.shutdown`, reconcile it with any preceding
//!   `assistant.message` events to distribute cumulative metrics across the actual
//!   message timestamps.
//! - If the session is resumed/open (new `assistant.message` events follow the last shutdown),
//!   keep those new events as provisional entries.
//! - If no `session.shutdown` exists, emit provisional entries.

use crate::types::{Result, ToktrackError, UsageEntry};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use super::CLIParser;

#[derive(Deserialize)]
struct CopilotLine<'a> {
    #[serde(rename = "type")]
    line_type: &'a str,
    #[serde(default)]
    timestamp: Option<&'a str>,
    #[serde(default)]
    data: Option<CopilotData<'a>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CopilotData<'a> {
    #[serde(default, borrow)]
    session_id: Option<&'a str>,
    #[serde(default)]
    context: Option<CopilotContext<'a>>,
    #[serde(default, borrow)]
    message_id: Option<&'a str>,
    #[serde(default, borrow)]
    request_id: Option<&'a str>,
    #[serde(default, borrow)]
    model: Option<&'a str>,
    #[serde(default)]
    output_tokens: Option<u64>,
    #[serde(default, borrow)]
    current_model: Option<&'a str>,
    #[serde(default)]
    token_details: Option<CopilotTokenDetails>,
    #[serde(default)]
    model_metrics: Option<std::collections::HashMap<String, CopilotModelMetric>>,
}

#[derive(Deserialize)]
struct CopilotContext<'a> {
    #[serde(default, borrow)]
    cwd: Option<&'a str>,
}

#[derive(Deserialize)]
struct CopilotTokenDetails {
    #[serde(default)]
    input: Option<CopilotTokenCount>,
    #[serde(default)]
    cache_read: Option<CopilotTokenCount>,
    #[serde(default)]
    cache_write: Option<CopilotTokenCount>,
    #[serde(default)]
    output: Option<CopilotTokenCount>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CopilotTokenCount {
    #[serde(default)]
    token_count: u64,
}

#[derive(Deserialize)]
struct CopilotModelMetric {
    #[serde(default)]
    usage: Option<CopilotModelMetricUsage>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CopilotModelMetricUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    cache_read_tokens: u64,
    #[serde(default)]
    cache_write_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    reasoning_tokens: Option<u64>,
}

struct ShutdownModelEntry {
    model: Option<String>,
    input_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
    output_tokens: u64,
    reasoning_tokens: u64,
}

enum ParseResult {
    Skip,
    SessionStart {
        timestamp: DateTime<Utc>,
        session_id: Option<String>,
        project: Option<String>,
    },
    AssistantMessage {
        timestamp: DateTime<Utc>,
        message_id: Option<String>,
        request_id: Option<String>,
        model: Option<String>,
        output_tokens: u64,
    },
    SessionShutdown {
        timestamp: DateTime<Utc>,
        metrics: Vec<ShutdownModelEntry>,
    },
}

/// Parser for GitHub Copilot CLI usage logs.
pub struct CopilotParser {
    data_dir: PathBuf,
}

impl CopilotParser {
    /// Create a parser with the default data directory (`~/.copilot/session-state`).
    pub fn new() -> Self {
        let root = super::discovery::first_env_dir(&["COPILOT_HOME"]).unwrap_or_else(|| {
            directories::BaseDirs::new()
                .map(|d| d.home_dir().join(".copilot"))
                .unwrap_or_else(|| {
                    eprintln!("[toktrack] Warning: Could not determine home directory");
                    PathBuf::from(".")
                })
        });

        Self {
            data_dir: root.join("session-state"),
        }
    }

    /// Create a parser with a custom data directory (for testing).
    #[allow(dead_code)]
    pub fn with_data_dir(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    fn parse_line(&self, line: &mut [u8]) -> ParseResult {
        if line.is_empty() {
            return ParseResult::Skip;
        }

        let parsed: CopilotLine = match simd_json::from_slice(line) {
            Ok(v) => v,
            Err(_) => return ParseResult::Skip,
        };

        match parsed.line_type {
            "session.start" => {
                let data = match parsed.data {
                    Some(d) => d,
                    None => return ParseResult::Skip,
                };
                let ts = match parsed.timestamp.and_then(parse_rfc3339_utc) {
                    Some(v) => v,
                    None => return ParseResult::Skip,
                };

                let session_id = data.session_id.map(String::from);
                let project = data
                    .context
                    .and_then(|ctx| ctx.cwd)
                    .map(std::string::ToString::to_string);

                if session_id.is_none() && project.is_none() {
                    return ParseResult::Skip;
                }

                ParseResult::SessionStart {
                    timestamp: ts,
                    session_id,
                    project,
                }
            }
            "assistant.message" => {
                let data = match parsed.data {
                    Some(d) => d,
                    None => return ParseResult::Skip,
                };
                let output_tokens = match data.output_tokens {
                    Some(v) if v > 0 => v,
                    _ => return ParseResult::Skip,
                };
                let ts = match parsed.timestamp.and_then(parse_rfc3339_utc) {
                    Some(v) => v,
                    None => return ParseResult::Skip,
                };

                ParseResult::AssistantMessage {
                    timestamp: ts,
                    message_id: data.message_id.map(String::from),
                    request_id: data.request_id.map(String::from),
                    model: data.model.map(String::from),
                    output_tokens,
                }
            }
            "session.shutdown" => {
                let data = match parsed.data {
                    Some(d) => d,
                    None => return ParseResult::Skip,
                };
                let ts = match parsed.timestamp.and_then(parse_rfc3339_utc) {
                    Some(v) => v,
                    None => return ParseResult::Skip,
                };

                let mut metrics = Vec::new();

                if let Some(model_metrics) = data.model_metrics {
                    for (model_name, metric) in model_metrics {
                        if let Some(usage) = metric.usage {
                            let input_tokens = normalize_cache_tokens(
                                usage.input_tokens,
                                usage.cache_read_tokens,
                                usage.cache_write_tokens,
                            );
                            metrics.push(ShutdownModelEntry {
                                model: Some(model_name),
                                input_tokens,
                                cache_read_tokens: usage.cache_read_tokens,
                                cache_creation_tokens: usage.cache_write_tokens,
                                output_tokens: usage.output_tokens,
                                reasoning_tokens: usage.reasoning_tokens.unwrap_or(0),
                            });
                        }
                    }
                }

                // Fallback to legacy tokenDetails if no modelMetrics were parsed
                if metrics.is_empty() {
                    if let Some(details) = data.token_details {
                        let raw_input = details.input.map(|v| v.token_count).unwrap_or(0);
                        let cache_read_tokens =
                            details.cache_read.map(|v| v.token_count).unwrap_or(0);
                        let cache_creation_tokens =
                            details.cache_write.map(|v| v.token_count).unwrap_or(0);
                        let output_tokens = details.output.map(|v| v.token_count).unwrap_or(0);

                        let input_tokens = normalize_cache_tokens(
                            raw_input,
                            cache_read_tokens,
                            cache_creation_tokens,
                        );

                        metrics.push(ShutdownModelEntry {
                            model: data.current_model.map(String::from),
                            input_tokens,
                            cache_read_tokens,
                            cache_creation_tokens,
                            output_tokens,
                            reasoning_tokens: 0,
                        });
                    }
                }

                if metrics.is_empty() {
                    return ParseResult::Skip;
                }

                ParseResult::SessionShutdown {
                    timestamp: ts,
                    metrics,
                }
            }
            _ => ParseResult::Skip,
        }
    }
}

fn normalize_cache_tokens(input: u64, cache_read: u64, cache_write: u64) -> u64 {
    input.saturating_sub(cache_read).saturating_sub(cache_write)
}

fn parse_rfc3339_utc(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

impl Default for CopilotParser {
    fn default() -> Self {
        Self::new()
    }
}

impl CLIParser for CopilotParser {
    fn name(&self) -> &str {
        "copilot"
    }

    fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    fn file_pattern(&self) -> &str {
        "**/events.jsonl"
    }

    fn retroactive_reconciliation(&self) -> bool {
        true
    }

    fn parse_file(&self, path: &Path) -> Result<Vec<UsageEntry>> {
        let file = File::open(path).map_err(ToktrackError::Io)?;
        let reader = BufReader::new(file);

        let fallback_session_id = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .map(std::string::ToString::to_string);

        let mut current_session_id = fallback_session_id;
        let mut current_project: Option<String> = None;
        let mut current_model: Option<String> = None;

        let mut session_start_time: Option<DateTime<Utc>> = None;
        let mut last_shutdown: Option<(DateTime<Utc>, Vec<ShutdownModelEntry>)> = None;

        // Internal struct to keep parsed assistant messages
        struct ParsedMessage {
            timestamp: DateTime<Utc>,
            message_id: Option<String>,
            request_id: Option<String>,
            model: Option<String>,
            output_tokens: u64,
        }

        let mut messages = Vec::new();

        for line_result in reader.lines() {
            let line = match line_result {
                Ok(l) => l,
                Err(_) => continue,
            };
            if line.trim().is_empty() {
                continue;
            }

            let mut bytes = line.into_bytes();
            match self.parse_line(&mut bytes) {
                ParseResult::Skip => {}
                ParseResult::SessionStart {
                    timestamp,
                    session_id,
                    project,
                } => {
                    if session_start_time.is_none() {
                        session_start_time = Some(timestamp);
                    }
                    if session_id.is_some() {
                        current_session_id = session_id;
                    }
                    if project.is_some() {
                        current_project = project;
                    }
                }
                ParseResult::AssistantMessage {
                    timestamp,
                    message_id,
                    request_id,
                    model,
                    output_tokens,
                } => {
                    if session_start_time.is_none() {
                        session_start_time = Some(timestamp);
                    }
                    if model.is_some() {
                        current_model = model.clone();
                    }
                    messages.push(ParsedMessage {
                        timestamp,
                        message_id,
                        request_id,
                        model,
                        output_tokens,
                    });
                }
                ParseResult::SessionShutdown { timestamp, metrics } => {
                    if session_start_time.is_none() {
                        session_start_time = Some(timestamp);
                    }
                    last_shutdown = Some((timestamp, metrics));
                }
            }
        }

        let mut entries = Vec::new();

        if let Some((shutdown_ts, shutdown_metrics)) = last_shutdown {
            // Partition messages into pre-shutdown and post-shutdown
            let mut pre_shutdown = Vec::new();
            let mut post_shutdown = Vec::new();

            for msg in messages {
                if msg.timestamp <= shutdown_ts {
                    pre_shutdown.push(msg);
                } else {
                    post_shutdown.push(msg);
                }
            }

            // Reconcile each shutdown model metric
            for metric in shutdown_metrics {
                let m_model = metric.model.clone().or_else(|| current_model.clone());

                // Find matching pre-shutdown messages for this model
                let model_msgs: Vec<&ParsedMessage> = pre_shutdown
                    .iter()
                    .filter(|m| {
                        let msg_model = m.model.as_ref().or(current_model.as_ref());
                        msg_model == m_model.as_ref()
                    })
                    .collect();

                let sum_output: u64 = model_msgs.iter().map(|m| m.output_tokens).sum();

                if sum_output > 0 && !model_msgs.is_empty() {
                    // Distribute proportionally
                    let mut distributed = Vec::new();
                    let mut input_sum = 0u64;
                    let mut cache_read_sum = 0u64;
                    let mut cache_creation_sum = 0u64;
                    let mut reasoning_sum = 0u64;
                    let mut output_sum = 0u64;

                    for m in &model_msgs {
                        let ratio = m.output_tokens as f64 / sum_output as f64;

                        let input = (metric.input_tokens as f64 * ratio).round() as u64;
                        let cache_read = (metric.cache_read_tokens as f64 * ratio).round() as u64;
                        let cache_creation =
                            (metric.cache_creation_tokens as f64 * ratio).round() as u64;
                        let reasoning = (metric.reasoning_tokens as f64 * ratio).round() as u64;
                        let output = (metric.output_tokens as f64 * ratio).round() as u64;

                        input_sum += input;
                        cache_read_sum += cache_read;
                        cache_creation_sum += cache_creation;
                        reasoning_sum += reasoning;
                        output_sum += output;

                        distributed.push(UsageEntry {
                            timestamp: m.timestamp,
                            model: m_model.clone(),
                            input_tokens: input,
                            output_tokens: output,
                            cache_read_tokens: cache_read,
                            cache_creation_tokens: cache_creation,
                            reasoning_tokens: reasoning,
                            cache_creation_5m_tokens: 0,
                            cache_creation_1h_tokens: 0,
                            web_search_requests: 0,
                            web_fetch_requests: 0,
                            reported_total_tokens: None,
                            cost_usd: None,
                            message_id: m.message_id.clone(),
                            request_id: m.request_id.clone().or_else(|| current_session_id.clone()),
                            source: Some("copilot".into()),
                            provider: Some("github-copilot".into()),
                            project: current_project.clone(),
                        });
                    }

                    // Adjust for rounding discrepancies on the last entry
                    if let Some(last_entry) = distributed.last_mut() {
                        let input_diff = metric.input_tokens as i64 - input_sum as i64;
                        last_entry.input_tokens =
                            (last_entry.input_tokens as i64 + input_diff).max(0) as u64;

                        let cache_read_diff =
                            metric.cache_read_tokens as i64 - cache_read_sum as i64;
                        last_entry.cache_read_tokens =
                            (last_entry.cache_read_tokens as i64 + cache_read_diff).max(0) as u64;

                        let cache_creation_diff =
                            metric.cache_creation_tokens as i64 - cache_creation_sum as i64;
                        last_entry.cache_creation_tokens =
                            (last_entry.cache_creation_tokens as i64 + cache_creation_diff).max(0)
                                as u64;

                        let reasoning_diff = metric.reasoning_tokens as i64 - reasoning_sum as i64;
                        last_entry.reasoning_tokens =
                            (last_entry.reasoning_tokens as i64 + reasoning_diff).max(0) as u64;

                        let output_diff = metric.output_tokens as i64 - output_sum as i64;
                        last_entry.output_tokens =
                            (last_entry.output_tokens as i64 + output_diff).max(0) as u64;
                    }

                    entries.extend(distributed);
                } else {
                    // No matching messages or sum of output tokens is 0.
                    // Emit a single entry at session_start_time.
                    let entry_ts = session_start_time.unwrap_or(shutdown_ts);
                    // The model must be part of the dedup key: entries share the
                    // session id as message_id, so a multi-model shutdown reaching
                    // this branch would otherwise collide and drop all but one.
                    let request_id = Some(format!(
                        "session.shutdown::{}",
                        m_model.as_deref().unwrap_or("unknown")
                    ));
                    entries.push(UsageEntry {
                        timestamp: entry_ts,
                        model: m_model,
                        input_tokens: metric.input_tokens,
                        output_tokens: metric.output_tokens,
                        cache_read_tokens: metric.cache_read_tokens,
                        cache_creation_tokens: metric.cache_creation_tokens,
                        reasoning_tokens: metric.reasoning_tokens,
                        cache_creation_5m_tokens: 0,
                        cache_creation_1h_tokens: 0,
                        web_search_requests: 0,
                        web_fetch_requests: 0,
                        reported_total_tokens: None,
                        cost_usd: None,
                        message_id: current_session_id.clone(),
                        request_id,
                        source: Some("copilot".into()),
                        provider: Some("github-copilot".into()),
                        project: current_project.clone(),
                    });
                }
            }

            // Emit the post-shutdown messages as provisional entries
            for m in post_shutdown {
                let m_model = m.model.clone().or_else(|| current_model.clone());
                entries.push(UsageEntry {
                    timestamp: m.timestamp,
                    model: m_model,
                    input_tokens: 0,
                    output_tokens: m.output_tokens,
                    cache_read_tokens: 0,
                    cache_creation_tokens: 0,
                    reasoning_tokens: 0,
                    cache_creation_5m_tokens: 0,
                    cache_creation_1h_tokens: 0,
                    web_search_requests: 0,
                    web_fetch_requests: 0,
                    reported_total_tokens: None,
                    cost_usd: None,
                    message_id: m.message_id,
                    request_id: m.request_id.or_else(|| current_session_id.clone()),
                    source: Some("copilot".into()),
                    provider: Some("github-copilot".into()),
                    project: current_project.clone(),
                });
            }
        } else {
            // No shutdown event at all. Emit all messages as provisional.
            for m in messages {
                let m_model = m.model.clone().or_else(|| current_model.clone());
                entries.push(UsageEntry {
                    timestamp: m.timestamp,
                    model: m_model,
                    input_tokens: 0,
                    output_tokens: m.output_tokens,
                    cache_read_tokens: 0,
                    cache_creation_tokens: 0,
                    reasoning_tokens: 0,
                    cache_creation_5m_tokens: 0,
                    cache_creation_1h_tokens: 0,
                    web_search_requests: 0,
                    web_fetch_requests: 0,
                    reported_total_tokens: None,
                    cost_usd: None,
                    message_id: m.message_id,
                    request_id: m.request_id.or_else(|| current_session_id.clone()),
                    source: Some("copilot".into()),
                    provider: Some("github-copilot".into()),
                    project: current_project.clone(),
                });
            }
        }

        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("copilot")
            .join(name)
    }

    #[test]
    fn test_parse_open_session_uses_assistant_output_tokens() {
        let parser = CopilotParser::with_data_dir(PathBuf::from("tests/fixtures/copilot"));
        let entries = parser
            .parse_file(&fixture_path("open-session.log"))
            .unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].model.as_deref(), Some("gpt-5.3-codex"));
        assert_eq!(entries[0].input_tokens, 0);
        assert_eq!(entries[0].output_tokens, 120);
        assert_eq!(entries[0].provider.as_deref(), Some("github-copilot"));
        assert_eq!(entries[0].project.as_deref(), Some("/work/demo-repo"));
        assert_eq!(entries[0].message_id.as_deref(), Some("msg-1"));
        assert_eq!(entries[0].request_id.as_deref(), Some("req-1"));

        assert_eq!(entries[1].output_tokens, 80);
        assert_eq!(entries[1].message_id.as_deref(), Some("msg-2"));
        assert_eq!(entries[1].request_id.as_deref(), Some("req-2"));
    }

    #[test]
    fn test_parse_shutdown_token_details_as_authoritative_totals() {
        let parser = CopilotParser::with_data_dir(PathBuf::from("tests/fixtures/copilot"));
        let entries = parser
            .parse_file(&fixture_path("closed-session.log"))
            .unwrap();

        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.model.as_deref(), Some("gpt-5.3-codex"));
        // 92078 input - 1043840 cache_read - 0 cache_write = saturating to 0
        assert_eq!(entry.input_tokens, 0);
        assert_eq!(entry.cache_read_tokens, 1_043_840);
        assert_eq!(entry.cache_creation_tokens, 0);
        assert_eq!(entry.output_tokens, 9227);
        assert_eq!(entry.provider.as_deref(), Some("github-copilot"));
        assert_eq!(entry.message_id.as_deref(), Some("msg-a"));
        assert_eq!(entry.request_id.as_deref(), Some("req-a"));
        assert_eq!(entry.project.as_deref(), Some("/work/demo-repo"));
    }

    #[test]
    fn test_parse_real_session_metrics() {
        let parser = CopilotParser::with_data_dir(PathBuf::from("tests/fixtures/copilot"));
        let entries = parser
            .parse_file(&fixture_path("real-session-metrics.log"))
            .unwrap();

        assert_eq!(entries.len(), 2);

        let gpt = entries
            .iter()
            .find(|e| e.model.as_deref() == Some("gpt-5.3-codex"))
            .unwrap();
        assert_eq!(gpt.input_tokens, 92078);
        assert_eq!(gpt.cache_read_tokens, 1_043_840);
        assert_eq!(gpt.cache_creation_tokens, 0);
        assert_eq!(gpt.output_tokens, 9227);
        assert_eq!(gpt.reasoning_tokens, 321);
        assert_eq!(gpt.message_id.as_deref(), Some("msg-a1"));
        assert_eq!(gpt.request_id.as_deref(), Some("req-a1"));

        let claude = entries
            .iter()
            .find(|e| e.model.as_deref() == Some("claude-3-5-sonnet"))
            .unwrap();
        assert_eq!(claude.input_tokens, 3500);
        assert_eq!(claude.cache_read_tokens, 1000);
        assert_eq!(claude.cache_creation_tokens, 500);
        assert_eq!(claude.output_tokens, 120);
        assert_eq!(claude.reasoning_tokens, 0);
        assert_eq!(claude.message_id.as_deref(), Some("msg-a2"));
        assert_eq!(claude.request_id.as_deref(), Some("req-a2"));
    }

    #[test]
    fn test_parse_cross_midnight_distribution() {
        let parser = CopilotParser::with_data_dir(PathBuf::from("tests/fixtures/copilot"));
        let entries = parser
            .parse_file(&fixture_path("cross-midnight.log"))
            .unwrap();

        assert_eq!(entries.len(), 2);

        // Message 1 on 2026-07-14 gets 100 / 400 = 25% of the total tokens
        let m1 = entries
            .iter()
            .find(|e| e.message_id.as_deref() == Some("msg-m1"))
            .unwrap();
        assert_eq!(m1.timestamp.to_rfc3339(), "2026-07-14T23:30:00+00:00");
        assert_eq!(m1.input_tokens, 1000); // 25% of (8000 - 4000) = 25% of 4000 = 1000
        assert_eq!(m1.cache_read_tokens, 1000); // 25% of 4000 = 1000
        assert_eq!(m1.output_tokens, 100); // 25% of 400 = 100

        // Message 2 on 2026-07-15 gets 300 / 400 = 75% of the total tokens
        let m2 = entries
            .iter()
            .find(|e| e.message_id.as_deref() == Some("msg-m2"))
            .unwrap();
        assert_eq!(m2.timestamp.to_rfc3339(), "2026-07-15T00:30:00+00:00");
        assert_eq!(m2.input_tokens, 3000); // 75% of 4000 = 3000
        assert_eq!(m2.cache_read_tokens, 3000); // 75% of 4000 = 3000
        assert_eq!(m2.output_tokens, 300); // 75% of 400 = 300
    }

    #[test]
    fn test_parse_resumed_session() {
        let parser = CopilotParser::with_data_dir(PathBuf::from("tests/fixtures/copilot"));
        let entries = parser
            .parse_file(&fixture_path("resumed-session.log"))
            .unwrap();

        // Should return 2 entries: the reconciled shutdown entry and the new provisional entry
        assert_eq!(entries.len(), 2);

        let m1 = entries
            .iter()
            .find(|e| e.message_id.as_deref() == Some("msg-r1"))
            .unwrap();
        assert_eq!(m1.input_tokens, 10000);
        assert_eq!(m1.output_tokens, 200);

        let m2 = entries
            .iter()
            .find(|e| e.message_id.as_deref() == Some("msg-r2"))
            .unwrap();
        assert_eq!(m2.input_tokens, 0); // Provisional (has output only)
        assert_eq!(m2.output_tokens, 150);
    }

    #[test]
    fn test_parser_name_and_pattern() {
        let parser = CopilotParser::new();
        assert_eq!(parser.name(), "copilot");
        assert_eq!(parser.file_pattern(), "**/events.jsonl");
    }

    #[test]
    fn test_parse_nonexistent_file() {
        let parser = CopilotParser::new();
        let result = parser.parse_file(Path::new("/nonexistent/file.jsonl"));
        assert!(result.is_err());
    }

    #[test]
    fn test_multimodel_no_messages_emits_distinct_dedup_hashes() {
        // A multi-model shutdown with no matching assistant messages emits one
        // entry per model via the else-branch. `parse_and_dedup` keys on
        // `message_id:request_id`, so the entries must hash distinctly or a
        // model's usage is silently dropped.
        let parser = CopilotParser::with_data_dir(PathBuf::from("tests/fixtures/copilot"));
        let entries = parser
            .parse_file(&fixture_path("multimodel-no-messages.log"))
            .unwrap();

        assert_eq!(entries.len(), 2);

        let mut seen = std::collections::HashSet::new();
        for e in &entries {
            assert!(
                seen.insert(e.dedup_hash()),
                "colliding dedup_hash drops a model: {:?}",
                e.dedup_hash()
            );
        }

        let gpt = entries
            .iter()
            .find(|e| e.model.as_deref() == Some("gpt-5.3-codex"))
            .unwrap();
        assert_eq!(gpt.input_tokens, 10000);
        assert_eq!(gpt.output_tokens, 500);

        let claude = entries
            .iter()
            .find(|e| e.model.as_deref() == Some("claude-3-5-sonnet"))
            .unwrap();
        assert_eq!(claude.input_tokens, 8000);
        assert_eq!(claude.output_tokens, 300);
    }

    #[test]
    fn test_copilot_reconciles_retroactively() {
        // Copilot re-dates cumulative shutdown totals onto earlier messages, so
        // the warm path must treat its recent files as able to touch old days.
        assert!(CopilotParser::new().retroactive_reconciliation());
        // Sources whose entries are final when written must not (perf).
        assert!(!crate::parsers::ClaudeCodeParser::new().retroactive_reconciliation());
    }
}
