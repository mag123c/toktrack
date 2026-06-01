//! Shared data-directory discovery for parsers.
//!
//! Centralizes environment-variable overrides so each parser doesn't reimplement
//! the "first non-empty env var wins, empty means unset" rule. Confirmed upstream
//! override variables:
//! - Codex: `CODEX_HOME` (root; sessions live under `$CODEX_HOME/sessions`)
//! - Claude: `CLAUDE_CONFIG_DIR` (root; projects under `$CLAUDE_CONFIG_DIR/projects`)
//! - Gemini: `GEMINI_CLI_HOME` (home root; data under `$GEMINI_CLI_HOME/.gemini/tmp`)
//! - OpenCode: `OPENCODE_DATA_DIR` (full data dir) / `XDG_DATA_HOME` (base)
//! - PI Agent: `PI_CODING_AGENT_SESSION_DIR` (== `--session-dir`) then `PI_AGENT_DIR`

use std::path::PathBuf;

/// First usable directory among `env_vars`: the first variable that is set and
/// non-empty. `CODEX_HOME`/`CLAUDE_CONFIG_DIR` may be comma-separated lists, so
/// the first entry is taken. Empty/whitespace values are treated as unset.
pub fn first_env_dir(env_vars: &[&str]) -> Option<PathBuf> {
    for name in env_vars {
        if let Ok(val) = std::env::var(name) {
            let first = val.split(',').next().unwrap_or("").trim();
            if !first.is_empty() {
                return Some(PathBuf::from(first));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // Each test uses a uniquely-named env var to avoid cross-test races
    // (cargo runs tests in parallel; process env is global).
    #[test]
    fn test_returns_none_when_unset() {
        assert_eq!(first_env_dir(&["TOKTRACK_DISCOVERY_UNSET_XYZ"]), None);
    }

    #[test]
    fn test_returns_first_set_var() {
        std::env::set_var("TOKTRACK_DISCOVERY_A1", "/tmp/a1");
        let got = first_env_dir(&["TOKTRACK_DISCOVERY_A1"]);
        std::env::remove_var("TOKTRACK_DISCOVERY_A1");
        assert_eq!(got, Some(PathBuf::from("/tmp/a1")));
    }

    #[test]
    fn test_empty_value_treated_as_unset() {
        std::env::set_var("TOKTRACK_DISCOVERY_EMPTY", "   ");
        let got = first_env_dir(&["TOKTRACK_DISCOVERY_EMPTY"]);
        std::env::remove_var("TOKTRACK_DISCOVERY_EMPTY");
        assert_eq!(got, None);
    }

    #[test]
    fn test_precedence_first_non_empty_wins() {
        std::env::set_var("TOKTRACK_DISCOVERY_P1", "");
        std::env::set_var("TOKTRACK_DISCOVERY_P2", "/tmp/p2");
        let got = first_env_dir(&["TOKTRACK_DISCOVERY_P1", "TOKTRACK_DISCOVERY_P2"]);
        std::env::remove_var("TOKTRACK_DISCOVERY_P1");
        std::env::remove_var("TOKTRACK_DISCOVERY_P2");
        assert_eq!(got, Some(PathBuf::from("/tmp/p2")));
    }

    #[test]
    fn test_comma_list_takes_first_entry() {
        std::env::set_var("TOKTRACK_DISCOVERY_LIST", "/tmp/first,/tmp/second");
        let got = first_env_dir(&["TOKTRACK_DISCOVERY_LIST"]);
        std::env::remove_var("TOKTRACK_DISCOVERY_LIST");
        assert_eq!(got, Some(PathBuf::from("/tmp/first")));
    }
}
