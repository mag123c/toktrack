//! Update checker service for npm-installed toktrack
//!
//! Checks npm registry for newer versions and provides update functionality.

use serde::Deserialize;
use std::io::{self, Write};
use std::process::Command;
use std::time::Duration;

/// npm registry URL for toktrack
const NPM_REGISTRY_URL: &str = "https://registry.npmjs.org/toktrack/latest";

/// HTTP request timeout in seconds
const REQUEST_TIMEOUT_SECS: u64 = 3;

/// Key input timeout in seconds
const KEY_INPUT_TIMEOUT_SECS: u64 = 5;

/// Current version from Cargo.toml
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Result of checking for updates
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateCheckResult {
    /// A newer version is available
    UpdateAvailable { current: String, latest: String },
    /// Current version is up to date
    UpToDate,
    /// Check failed (network error, timeout, etc.)
    CheckFailed,
}

/// npm registry package response (minimal fields)
#[derive(Debug, Deserialize)]
struct NpmPackageInfo {
    version: String,
}

/// Check for updates from npm registry
pub fn check_for_update() -> UpdateCheckResult {
    match fetch_latest_version() {
        Ok(latest) => {
            if is_newer_version(&latest, CURRENT_VERSION) {
                UpdateCheckResult::UpdateAvailable {
                    current: CURRENT_VERSION.to_string(),
                    latest,
                }
            } else {
                UpdateCheckResult::UpToDate
            }
        }
        Err(_) => UpdateCheckResult::CheckFailed,
    }
}

/// Fetch the latest version from npm registry
fn fetch_latest_version() -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let response = client
        .get(NPM_REGISTRY_URL)
        .send()
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let info: NpmPackageInfo = response
        .json()
        .map_err(|e| format!("JSON parse error: {}", e))?;

    Ok(info.version)
}

/// Compare two semver versions
/// Returns true if `latest` is newer than `current`
pub fn is_newer_version(latest: &str, current: &str) -> bool {
    let parse_version = |s: &str| -> Option<(u32, u32, u32)> {
        let parts: Vec<&str> = s.trim_start_matches('v').split('.').collect();
        if parts.len() >= 3 {
            Some((
                parts[0].parse().ok()?,
                parts[1].parse().ok()?,
                parts[2].split('-').next()?.parse().ok()?,
            ))
        } else {
            None
        }
    };

    match (parse_version(latest), parse_version(current)) {
        (Some((l_major, l_minor, l_patch)), Some((c_major, c_minor, c_patch))) => {
            (l_major, l_minor, l_patch) > (c_major, c_minor, c_patch)
        }
        _ => false,
    }
}

/// Execute npm update command
pub fn execute_update() -> Result<(), String> {
    eprintln!("[toktrack] Running: npm update -g toktrack");

    let output = Command::new("npm")
        .args(["update", "-g", "toktrack"])
        .output()
        .map_err(|e| format!("Failed to run npm: {}", e))?;

    if output.status.success() {
        eprintln!("[toktrack] Updated successfully! Please restart toktrack.");
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "npm update failed: {}\nTry manually: npm update -g toktrack",
            stderr.trim()
        ))
    }
}

/// Check for updates and prompt user
pub fn check_and_prompt_update() {
    match check_for_update() {
        UpdateCheckResult::UpdateAvailable { current, latest } => {
            eprint!(
                "[toktrack] Update available: {} -> {} (press 'u' to update, any key to continue) ",
                current, latest
            );
            let _ = io::stderr().flush();

            if wait_for_update_key() {
                match execute_update() {
                    Ok(()) => {
                        std::process::exit(0);
                    }
                    Err(e) => {
                        eprintln!("[toktrack] {}", e);
                    }
                }
            }
        }
        UpdateCheckResult::UpToDate | UpdateCheckResult::CheckFailed => {
            // Silent: don't interrupt normal flow
        }
    }
}

/// Wait for key input with timeout
/// Returns true if 'u' was pressed
fn wait_for_update_key() -> bool {
    use crossterm::event::{self, Event, KeyCode, KeyEvent};
    use crossterm::terminal;

    if terminal::enable_raw_mode().is_err() {
        return false;
    }

    let result = if event::poll(Duration::from_secs(KEY_INPUT_TIMEOUT_SECS)).unwrap_or(false) {
        if let Ok(Event::Key(KeyEvent { code, .. })) = event::read() {
            matches!(code, KeyCode::Char('u') | KeyCode::Char('U'))
        } else {
            false
        }
    } else {
        eprintln!();
        false
    };

    let _ = terminal::disable_raw_mode();

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========== is_newer_version tests ==========

    #[test]
    fn test_is_newer_version_major() {
        assert!(is_newer_version("2.0.0", "1.0.0"));
        assert!(is_newer_version("2.0.0", "1.9.9"));
    }

    #[test]
    fn test_is_newer_version_minor() {
        assert!(is_newer_version("1.2.0", "1.1.0"));
        assert!(is_newer_version("1.2.0", "1.1.9"));
    }

    #[test]
    fn test_is_newer_version_patch() {
        assert!(is_newer_version("1.0.2", "1.0.1"));
        assert!(is_newer_version("0.1.10", "0.1.9"));
    }

    #[test]
    fn test_is_newer_version_equal() {
        assert!(!is_newer_version("1.0.0", "1.0.0"));
        assert!(!is_newer_version("0.1.10", "0.1.10"));
    }

    #[test]
    fn test_is_newer_version_older() {
        assert!(!is_newer_version("1.0.0", "2.0.0"));
        assert!(!is_newer_version("1.0.0", "1.1.0"));
        assert!(!is_newer_version("1.0.0", "1.0.1"));
    }

    #[test]
    fn test_is_newer_version_with_v_prefix() {
        assert!(is_newer_version("v2.0.0", "v1.0.0"));
        assert!(is_newer_version("v2.0.0", "1.0.0"));
        assert!(is_newer_version("2.0.0", "v1.0.0"));
    }

    #[test]
    fn test_is_newer_version_with_prerelease() {
        assert!(is_newer_version("2.0.0-beta", "1.0.0"));
        assert!(!is_newer_version("1.0.0-beta", "1.0.0"));
    }

    #[test]
    fn test_is_newer_version_invalid() {
        assert!(!is_newer_version("invalid", "1.0.0"));
        assert!(!is_newer_version("1.0.0", "invalid"));
        assert!(!is_newer_version("", "1.0.0"));
    }

    // ========== UpdateCheckResult tests ==========

    #[test]
    fn test_update_check_result_update_available() {
        let result = UpdateCheckResult::UpdateAvailable {
            current: "1.0.0".to_string(),
            latest: "2.0.0".to_string(),
        };
        assert!(matches!(result, UpdateCheckResult::UpdateAvailable { .. }));
    }

    #[test]
    fn test_update_check_result_up_to_date() {
        let result = UpdateCheckResult::UpToDate;
        assert!(matches!(result, UpdateCheckResult::UpToDate));
    }

    #[test]
    fn test_update_check_result_check_failed() {
        let result = UpdateCheckResult::CheckFailed;
        assert!(matches!(result, UpdateCheckResult::CheckFailed));
    }

    #[test]
    #[ignore] // Network required
    fn test_npm_registry_reachable() {
        let result = check_for_update();
        assert!(!matches!(result, UpdateCheckResult::CheckFailed));
    }
}
