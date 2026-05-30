//! Configuration loading for toktrack.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use directories::BaseDirs;
use serde::{Deserialize, Serialize};

use crate::types::{Result, ToktrackError};

const DEFAULT_CODEX_SESSIONS_PATH: &str = "~/.codex/sessions";

/// User configuration loaded from `~/.toktrack/config.toml`.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ToktrackConfig {
    /// Remote names that are included unless `--local-only` is used.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_remotes: Vec<String>,
    /// Configured remote machines.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remotes: Vec<RemoteConfig>,
}

/// One configured SSH remote.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RemoteConfig {
    pub name: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "RemotePaths::is_empty")]
    pub paths: RemotePaths,
}

/// Per-tool remote data paths.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct RemotePaths {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex: Option<String>,
}

impl RemoteConfig {
    pub fn new(name: impl Into<String>, target: impl Into<String>, codex: Option<String>) -> Self {
        Self {
            name: name.into(),
            target: target.into(),
            paths: RemotePaths { codex },
        }
    }

    pub fn codex_sessions_path(&self) -> &str {
        self.paths
            .codex
            .as_deref()
            .filter(|path| !path.trim().is_empty())
            .unwrap_or(DEFAULT_CODEX_SESSIONS_PATH)
    }
}

impl RemotePaths {
    fn is_empty(&self) -> bool {
        self.codex.is_none()
    }
}

impl ToktrackConfig {
    pub fn remote(&self, name: &str) -> Option<&RemoteConfig> {
        self.remotes.iter().find(|remote| remote.name == name)
    }

    pub fn remote_names(&self) -> Vec<String> {
        self.remotes
            .iter()
            .map(|remote| remote.name.clone())
            .collect()
    }

    pub fn add_remote(&mut self, remote: RemoteConfig, make_default: bool) -> Result<()> {
        validate_remote_name(&remote.name)?;
        if remote.target.trim().is_empty() {
            return Err(ToktrackError::Config(format!(
                "remote '{}' has an empty target",
                remote.name
            )));
        }
        if self.remote(&remote.name).is_some() {
            return Err(ToktrackError::Config(format!(
                "remote '{}' is already configured",
                remote.name
            )));
        }

        let name = remote.name.clone();
        self.remotes.push(remote);
        if make_default {
            self.add_default_remote(&name)?;
        }
        self.validate()
    }

    pub fn remove_remote(&mut self, name: &str) -> Result<()> {
        let original_len = self.remotes.len();
        self.remotes.retain(|remote| remote.name != name);
        if self.remotes.len() == original_len {
            return Err(ToktrackError::Config(format!(
                "remote '{}' is not configured",
                name
            )));
        }

        self.default_remotes
            .retain(|default_name| default_name != name);
        self.validate()
    }

    pub fn add_default_remote(&mut self, name: &str) -> Result<bool> {
        validate_remote_name(name)?;
        if self.remote(name).is_none() {
            return Err(ToktrackError::Config(format!(
                "remote '{}' is not configured",
                name
            )));
        }
        if self
            .default_remotes
            .iter()
            .any(|default_name| default_name == name)
        {
            return Ok(false);
        }

        self.default_remotes.push(name.to_string());
        self.validate()?;
        Ok(true)
    }

    pub fn remove_default_remote(&mut self, name: &str) -> Result<bool> {
        validate_remote_name(name)?;
        let original_len = self.default_remotes.len();
        self.default_remotes
            .retain(|default_name| default_name != name);
        self.validate()?;
        Ok(self.default_remotes.len() != original_len)
    }

    fn validate(&self) -> Result<()> {
        let mut names = HashSet::new();

        for remote in &self.remotes {
            validate_remote_name(&remote.name)?;
            if remote.target.trim().is_empty() {
                return Err(ToktrackError::Config(format!(
                    "remote '{}' has an empty target",
                    remote.name
                )));
            }
            if !names.insert(remote.name.clone()) {
                return Err(ToktrackError::Config(format!(
                    "duplicate remote '{}'",
                    remote.name
                )));
            }
        }

        for name in &self.default_remotes {
            if self.remote(name).is_none() {
                return Err(ToktrackError::Config(format!(
                    "default remote '{}' is not configured",
                    name
                )));
            }
        }

        Ok(())
    }
}

/// Load toktrack configuration from disk.
pub struct ConfigService;

impl ConfigService {
    pub fn default_config_path() -> Result<PathBuf> {
        let base_dirs = BaseDirs::new()
            .ok_or_else(|| ToktrackError::Config("cannot determine home directory".into()))?;
        Ok(base_dirs.home_dir().join(".toktrack").join("config.toml"))
    }

    pub fn load_default() -> Result<Option<ToktrackConfig>> {
        let path = Self::default_config_path()?;
        Self::load_optional(&path)
    }

    pub fn load_optional(path: &Path) -> Result<Option<ToktrackConfig>> {
        if !path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(path)?;
        let config: ToktrackConfig = toml::from_str(&content)
            .map_err(|e| ToktrackError::Config(format!("failed to parse config: {}", e)))?;
        config.validate()?;
        Ok(Some(config))
    }

    pub fn load_or_default(path: &Path) -> Result<ToktrackConfig> {
        Ok(Self::load_optional(path)?.unwrap_or_default())
    }

    pub fn save(path: &Path, config: &ToktrackConfig) -> Result<()> {
        config.validate()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(config)
            .map_err(|e| ToktrackError::Config(format!("failed to serialize config: {}", e)))?;
        fs::write(path, content)?;
        Ok(())
    }
}

fn validate_remote_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(ToktrackError::Config("remote name cannot be empty".into()));
    }

    if !name
        .bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphanumeric())
    {
        return Err(ToktrackError::Config(format!(
            "remote name '{}' must start with a letter or number and may only contain letters, numbers, '.', '_' and '-'",
            name
        )));
    }

    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Err(ToktrackError::Config(format!(
            "remote name '{}' may only contain letters, numbers, '.', '_' and '-'",
            name
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_config(content: &str) -> Result<ToktrackConfig> {
        let config: ToktrackConfig = toml::from_str(content)
            .map_err(|e| ToktrackError::Config(format!("failed to parse config: {}", e)))?;
        config.validate()?;
        Ok(config)
    }

    #[test]
    fn parses_remote_config() {
        let config = load_config(
            r#"
default_remotes = ["devbox"]

[[remotes]]
name = "devbox"
target = "ubuntu@devbox"

[remotes.paths]
codex = "/home/ubuntu/.codex/sessions"
"#,
        )
        .unwrap();

        let remote = config.remote("devbox").unwrap();
        assert_eq!(config.default_remotes, vec!["devbox"]);
        assert_eq!(remote.target, "ubuntu@devbox");
        assert_eq!(remote.codex_sessions_path(), "/home/ubuntu/.codex/sessions");
    }

    #[test]
    fn codex_path_defaults_to_standard_location() {
        let config = load_config(
            r#"
[[remotes]]
name = "devbox"
target = "ubuntu@devbox"
"#,
        )
        .unwrap();

        assert_eq!(
            config.remote("devbox").unwrap().codex_sessions_path(),
            DEFAULT_CODEX_SESSIONS_PATH
        );
    }

    #[test]
    fn rejects_duplicate_remote_names() {
        let error = load_config(
            r#"
[[remotes]]
name = "devbox"
target = "ubuntu@devbox"

[[remotes]]
name = "devbox"
target = "root@devbox"
"#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("duplicate remote"));
    }

    #[test]
    fn rejects_unknown_default_remote() {
        let error = load_config(r#"default_remotes = ["devbox"]"#).unwrap_err();
        assert!(error.to_string().contains("default remote 'devbox'"));
    }

    #[test]
    fn rejects_remote_name_with_path_separator() {
        let error = load_config(
            r#"
[[remotes]]
name = "../devbox"
target = "ubuntu@devbox"
"#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("may only contain"));
    }

    #[test]
    fn rejects_dot_remote_name() {
        let error = load_config(
            r#"
[[remotes]]
name = "."
target = "ubuntu@devbox"
"#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("must start"));
    }

    #[test]
    fn rejects_dot_dot_remote_name() {
        let error = load_config(
            r#"
[[remotes]]
name = ".."
target = "ubuntu@devbox"
"#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("must start"));
    }

    #[test]
    fn add_remote_can_mark_it_default() {
        let mut config = ToktrackConfig::default();
        config
            .add_remote(
                RemoteConfig::new("devbox", "ubuntu@devbox", Some("~/.codex/sessions".into())),
                true,
            )
            .unwrap();

        assert_eq!(config.remote_names(), vec!["devbox"]);
        assert_eq!(config.default_remotes, vec!["devbox"]);
    }

    #[test]
    fn add_remote_rejects_duplicate_name() {
        let mut config = ToktrackConfig::default();
        config
            .add_remote(RemoteConfig::new("devbox", "ubuntu@devbox", None), false)
            .unwrap();
        let error = config
            .add_remote(RemoteConfig::new("devbox", "root@devbox", None), false)
            .unwrap_err();

        assert!(error.to_string().contains("already configured"));
    }

    #[test]
    fn remove_remote_also_removes_it_from_defaults() {
        let mut config = ToktrackConfig::default();
        config
            .add_remote(RemoteConfig::new("devbox", "ubuntu@devbox", None), true)
            .unwrap();

        config.remove_remote("devbox").unwrap();

        assert!(config.remotes.is_empty());
        assert!(config.default_remotes.is_empty());
    }

    #[test]
    fn add_default_remote_is_idempotent() {
        let mut config = ToktrackConfig::default();
        config
            .add_remote(RemoteConfig::new("devbox", "ubuntu@devbox", None), false)
            .unwrap();

        assert!(config.add_default_remote("devbox").unwrap());
        assert!(!config.add_default_remote("devbox").unwrap());
        assert_eq!(config.default_remotes, vec!["devbox"]);
    }

    #[test]
    fn remove_default_remote_reports_whether_it_changed_config() {
        let mut config = ToktrackConfig::default();
        config
            .add_remote(RemoteConfig::new("devbox", "ubuntu@devbox", None), true)
            .unwrap();

        assert!(config.remove_default_remote("devbox").unwrap());
        assert!(!config.remove_default_remote("devbox").unwrap());
        assert!(config.default_remotes.is_empty());
    }

    #[test]
    fn save_creates_parent_dirs_and_round_trips() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join(".toktrack").join("config.toml");
        let mut config = ToktrackConfig::default();
        config
            .add_remote(RemoteConfig::new("devbox", "ubuntu@devbox", None), true)
            .unwrap();

        ConfigService::save(&path, &config).unwrap();
        let loaded = ConfigService::load_optional(&path).unwrap().unwrap();

        assert_eq!(loaded.default_remotes, vec!["devbox"]);
        assert_eq!(loaded.remote("devbox").unwrap().target, "ubuntu@devbox");
    }
}
