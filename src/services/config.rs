//! Configuration loading for toktrack.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

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
    /// Project folders starred as roots in the Projects tab; projects under a
    /// root are folded into it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub project_roots: Vec<String>,
    /// Git worktree folders mapped to the main repository they belong to, so
    /// a worktree's projects fold into the repo's root even after the
    /// worktree folder is deleted. Filled in by the data loader.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub worktree_repos: BTreeMap<String, String>,
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

    /// Star `path` as a project root, or unstar it if already starred.
    /// Returns whether it is a root afterwards.
    pub fn toggle_project_root(&mut self, path: &str) -> bool {
        crate::services::project_roots::toggle(&mut self.project_roots, path)
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

    /// Load the config at `path` (or a default one), let `edit` change it and
    /// save it if `edit` reports a change. Returns the resulting config. The
    /// whole round trip holds a process-wide lock, so the data loader and the
    /// TUI cannot drop each other's edits.
    pub fn update(
        path: &Path,
        edit: impl FnOnce(&mut ToktrackConfig) -> bool,
    ) -> Result<ToktrackConfig> {
        let _guard = UPDATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut config = Self::load_or_default(path)?;
        if edit(&mut config) {
            Self::save(path, &config)?;
        }
        Ok(config)
    }

    pub fn save(path: &Path, config: &ToktrackConfig) -> Result<()> {
        config.validate()?;
        // Replace the real file behind a symlinked config (dotfile managers),
        // not the link itself.
        let target = resolve_save_target(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let fresh = toml::to_string_pretty(config)
            .map_err(|e| ToktrackError::Config(format!("failed to serialize config: {}", e)))?;
        // Edit the existing file rather than rewriting it, so comments,
        // formatting and keys this version doesn't know about survive. A file
        // that can't be read or parsed here (callers load it first, so only a
        // concurrent edit gets this far) is replaced by the fresh output.
        let content = fs::read_to_string(&target)
            .ok()
            .and_then(|existing| merge_owned_keys(&existing, &fresh))
            .unwrap_or(fresh);
        // Write a sibling temp file and rename it over the target, so a full
        // disk or a crash mid-write cannot leave a truncated config behind.
        let tmp = target.with_extension(format!("toml.{}.tmp", std::process::id()));
        let write = || -> std::io::Result<()> {
            // A temp file left by an earlier crashed save would block create_new.
            let _ = fs::remove_file(&tmp);
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)?;
            // Keep the existing mode (e.g. 0600) instead of the umask default,
            // and apply it before any content lands in the file.
            if let Ok(meta) = fs::metadata(&target) {
                file.set_permissions(meta.permissions())?;
            }
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
            fs::rename(&tmp, &target)
        };
        if let Err(e) = write() {
            let _ = fs::remove_file(&tmp);
            return Err(e.into());
        }
        Ok(())
    }
}

/// Top-level keys written by [`ToktrackConfig`]. Every other key in the file
/// belongs to the user (or a newer toktrack) and is left alone on save.
const OWNED_KEYS: [&str; 4] = [
    "default_remotes",
    "remotes",
    "project_roots",
    "worktree_repos",
];

/// Serializes [`ConfigService::update`] round trips within this process.
static UPDATE_LOCK: Mutex<()> = Mutex::new(());

/// Symlink hops followed when resolving a dangling config link.
const MAX_SYMLINK_HOPS: usize = 40;

/// The file a save should replace: `path` with its symlinks resolved.
/// `canonicalize` needs the target to exist, so a dangling link is followed by
/// hand; otherwise the first save would replace the link with a regular file.
fn resolve_save_target(path: &Path) -> PathBuf {
    if let Ok(real) = fs::canonicalize(path) {
        return real;
    }
    let mut target = path.to_path_buf();
    for _ in 0..MAX_SYMLINK_HOPS {
        match fs::read_link(&target) {
            Ok(next) => {
                target = match target.parent() {
                    Some(parent) => parent.join(next),
                    None => next,
                }
            }
            Err(_) => return target,
        }
    }
    // Symlink loop: fall back to the path as given.
    path.to_path_buf()
}

/// Apply the owned keys of `fresh` to `existing`, replacing only the keys
/// whose value changed. A changed key is rewritten whole, so comments and
/// unknown fields inside it (e.g. in one `[[remotes]]` entry) are not kept;
/// everything outside it is. Returns `None` if either side fails to parse, so
/// the caller falls back to writing `fresh` as is.
fn merge_owned_keys(existing: &str, fresh: &str) -> Option<String> {
    let mut doc: toml_edit::DocumentMut = existing.parse().ok()?;
    let new_doc: toml_edit::DocumentMut = fresh.parse().ok()?;
    // Compare what this version understands, so a key holding fields it
    // doesn't know about isn't seen as changed (and rewritten without them).
    let old_config: ToktrackConfig = toml::from_str(existing).ok()?;
    let old_values: toml::Table =
        toml::from_str(&toml::to_string_pretty(&old_config).ok()?).ok()?;
    let new_values: toml::Table = toml::from_str(fresh).ok()?;

    for key in OWNED_KEYS {
        // Unchanged keys keep their formatting and inner comments.
        if old_values.get(key) == new_values.get(key) {
            continue;
        }
        // Edit existing items in place: comments above a key belong to the
        // key, so replacing or removing the key would drop them.
        match (doc.get_mut(key), new_doc.get(key)) {
            (Some(old_item), Some(new_item)) => {
                let mut new_item = new_item.clone();
                // Keep a trailing comment on a plain `key = value` line.
                if let (Some(old), Some(new)) = (old_item.as_value(), new_item.as_value_mut()) {
                    *new.decor_mut() = old.decor().clone();
                }
                *old_item = new_item;
            }
            // An emptied `key = [...]` stays as `key = []`, keeping its
            // comments. Emptied `[[tables]]` have no line to keep.
            (Some(old_item), None) => match old_item.as_value_mut() {
                Some(old) => {
                    let mut empty = toml_edit::Value::Array(toml_edit::Array::new());
                    *empty.decor_mut() = old.decor().clone();
                    *old = empty;
                }
                None => {
                    doc.remove(key);
                }
            },
            (None, Some(new_item)) => {
                doc.insert(key, new_item.clone());
            }
            (None, None) => {}
        }
    }
    // toml_edit writes `\n`; keep a CRLF file CRLF.
    let merged = doc.to_string();
    if existing.contains("\r\n") {
        return Some(merged.replace("\r\n", "\n").replace('\n', "\r\n"));
    }
    Some(merged)
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
    fn config_without_project_roots_still_loads() {
        let config = load_config(r#"default_remotes = []"#).unwrap();
        assert!(config.project_roots.is_empty());
    }

    #[test]
    fn toggle_project_root_adds_then_removes() {
        let mut config = ToktrackConfig::default();
        assert!(config.toggle_project_root("/w/repo"));
        assert_eq!(config.project_roots, vec!["/w/repo"]);
        assert!(!config.toggle_project_root("/w/repo"));
        assert!(config.project_roots.is_empty());
    }

    #[test]
    fn project_roots_round_trip() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("config.toml");
        let mut config = ToktrackConfig::default();
        config.toggle_project_root(r"G:\work\repo");

        ConfigService::save(&path, &config).unwrap();
        let loaded = ConfigService::load_optional(&path).unwrap().unwrap();

        assert_eq!(loaded.project_roots, vec![r"G:\work\repo"]);
    }

    #[test]
    fn save_replaces_existing_config_without_leaving_temp_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("config.toml");
        let mut config = ToktrackConfig::default();
        config.toggle_project_root("/w/a");
        ConfigService::save(&path, &config).unwrap();
        config.toggle_project_root("/w/b");
        ConfigService::save(&path, &config).unwrap();

        let loaded = ConfigService::load_optional(&path).unwrap().unwrap();
        assert_eq!(loaded.project_roots, vec!["/w/a", "/w/b"]);
        let entries: Vec<_> = fs::read_dir(temp_dir.path()).unwrap().collect();
        assert_eq!(entries.len(), 1);
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

    #[cfg(unix)]
    #[test]
    fn save_writes_through_a_symlinked_config() {
        let temp_dir = tempfile::tempdir().unwrap();
        let real = temp_dir.path().join("dotfiles").join("config.toml");
        fs::create_dir_all(real.parent().unwrap()).unwrap();
        fs::write(&real, "").unwrap();
        let link = temp_dir.path().join("config.toml");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let mut config = ToktrackConfig::default();
        config.toggle_project_root("/w/a");

        ConfigService::save(&link, &config).unwrap();

        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        let loaded = ConfigService::load_optional(&real).unwrap().unwrap();
        assert_eq!(loaded.project_roots, vec!["/w/a"]);
    }

    #[test]
    fn save_keeps_comments_and_unknown_keys() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("config.toml");
        fs::write(
            &path,
            r#"# my toktrack config
future_option = true # keep me

# remotes
[[remotes]]
name = "devbox" # the box
target = "ubuntu@devbox"
"#,
        )
        .unwrap();
        let mut config = ConfigService::load_or_default(&path).unwrap();
        config.toggle_project_root("/w/a");

        ConfigService::save(&path, &config).unwrap();

        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains("# my toktrack config"));
        assert!(saved.contains("future_option = true # keep me"));
        assert!(saved.contains("# remotes"));
        assert!(saved.contains(r#"name = "devbox" # the box"#));
        let loaded = ConfigService::load_optional(&path).unwrap().unwrap();
        assert_eq!(loaded.project_roots, vec!["/w/a"]);
        assert_eq!(loaded.remote("devbox").unwrap().target, "ubuntu@devbox");
    }

    #[test]
    fn worktree_repos_round_trip_with_windows_paths_and_comments() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("config.toml");
        fs::write(&path, "# starred\nproject_roots = ['G:\\Scripts\\annex']\n").unwrap();

        ConfigService::update(&path, |config| {
            config.worktree_repos.insert(
                r"C:\Users\me\.t3\worktrees\annex\x".to_string(),
                "G:/Scripts/annex".to_string(),
            );
            true
        })
        .unwrap();

        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains("# starred"));
        assert!(saved.contains(r"project_roots = ['G:\Scripts\annex']"));
        let loaded = ConfigService::load_optional(&path).unwrap().unwrap();
        assert_eq!(
            loaded
                .worktree_repos
                .get(r"C:\Users\me\.t3\worktrees\annex\x")
                .map(String::as_str),
            Some("G:/Scripts/annex")
        );
    }

    #[test]
    fn update_returns_the_saved_config() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("config.toml");

        let config = ConfigService::update(&path, |config| {
            config.toggle_project_root("/w/a");
            true
        })
        .unwrap();

        assert_eq!(config.project_roots, vec!["/w/a"]);
        let loaded = ConfigService::load_optional(&path).unwrap().unwrap();
        assert_eq!(loaded.project_roots, vec!["/w/a"]);
    }

    #[test]
    fn update_without_a_change_does_not_write() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("config.toml");

        ConfigService::update(&path, |_| false).unwrap();

        assert!(!path.exists());
    }

    #[test]
    fn update_does_not_overwrite_an_unparseable_config() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("config.toml");
        fs::write(&path, "project_roots = [").unwrap();

        assert!(ConfigService::update(&path, |_| true).is_err());

        assert_eq!(fs::read_to_string(&path).unwrap(), "project_roots = [");
    }

    #[test]
    fn save_keeps_comments_around_a_changed_key() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("config.toml");
        fs::write(
            &path,
            "# starred folders\nproject_roots = [\"/w/a\"] # mine\n",
        )
        .unwrap();
        let mut config = ConfigService::load_or_default(&path).unwrap();
        config.toggle_project_root("/w/b");

        ConfigService::save(&path, &config).unwrap();

        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains("# starred folders"));
        assert!(saved.contains("# mine"));
        let loaded = ConfigService::load_optional(&path).unwrap().unwrap();
        assert_eq!(loaded.project_roots, vec!["/w/a", "/w/b"]);
    }

    #[test]
    fn save_empties_a_cleared_list_without_dropping_its_comments() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("config.toml");
        fs::write(&path, "# header\nproject_roots = [\"/w/a\"] # mine\n").unwrap();
        let mut config = ConfigService::load_or_default(&path).unwrap();
        config.toggle_project_root("/w/a");

        ConfigService::save(&path, &config).unwrap();

        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains("# header"));
        assert!(saved.contains("project_roots = [] # mine"));
        let loaded = ConfigService::load_optional(&path).unwrap().unwrap();
        assert!(loaded.project_roots.is_empty());
    }

    #[test]
    fn save_removes_the_last_remote_table() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("config.toml");
        fs::write(
            &path,
            "future_option = 1\n\n[[remotes]]\nname = \"devbox\"\ntarget = \"ubuntu@devbox\"\n",
        )
        .unwrap();
        let mut config = ConfigService::load_or_default(&path).unwrap();
        config.remove_remote("devbox").unwrap();

        ConfigService::save(&path, &config).unwrap();

        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains("future_option = 1"));
        assert!(!saved.contains("remotes"));
    }

    #[test]
    fn save_keeps_unknown_fields_in_unchanged_remotes() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("config.toml");
        fs::write(
            &path,
            r#"[[remotes]]
name = "devbox"
target = "ubuntu@devbox"
future = "x" # keep

[remotes.paths]
codex = "~/s"
claude = "~/c"
"#,
        )
        .unwrap();
        let mut config = ConfigService::load_or_default(&path).unwrap();
        config.toggle_project_root("/w/a");

        ConfigService::save(&path, &config).unwrap();

        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains(r#"future = "x" # keep"#));
        assert!(saved.contains(r#"claude = "~/c""#));
        let loaded = ConfigService::load_optional(&path).unwrap().unwrap();
        assert_eq!(loaded.project_roots, vec!["/w/a"]);
    }

    #[test]
    fn save_keeps_crlf_line_endings() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("config.toml");
        fs::write(&path, "# header\r\nproject_roots = [\"/w/a\"]\r\n").unwrap();
        let mut config = ConfigService::load_or_default(&path).unwrap();
        config.toggle_project_root("/w/b");

        ConfigService::save(&path, &config).unwrap();

        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains("# header\r\n"));
        assert!(!saved.replace("\r\n", "").contains('\n'));
        let loaded = ConfigService::load_optional(&path).unwrap().unwrap();
        assert_eq!(loaded.project_roots, vec!["/w/a", "/w/b"]);
    }

    #[test]
    fn owned_keys_cover_every_config_field() {
        let mut config = ToktrackConfig::default();
        config
            .add_remote(
                RemoteConfig::new("devbox", "ubuntu@devbox", Some("~/s".into())),
                true,
            )
            .unwrap();
        config.toggle_project_root("/w/a");
        config
            .worktree_repos
            .insert("/wt/repo/x".to_string(), "/w/repo".to_string());

        let table: toml::Table = toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
        let mut keys: Vec<&str> = table.keys().map(String::as_str).collect();
        keys.sort_unstable();
        let mut owned = OWNED_KEYS.to_vec();
        owned.sort_unstable();
        assert_eq!(keys, owned);
    }

    #[cfg(unix)]
    #[test]
    fn save_through_a_dangling_symlink_keeps_the_link() {
        let temp_dir = tempfile::tempdir().unwrap();
        let real = temp_dir.path().join("dotfiles").join("config.toml");
        let link = temp_dir.path().join("config.toml");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let mut config = ToktrackConfig::default();
        config.toggle_project_root("/w/a");

        ConfigService::save(&link, &config).unwrap();

        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        let loaded = ConfigService::load_optional(&real).unwrap().unwrap();
        assert_eq!(loaded.project_roots, vec!["/w/a"]);
    }

    #[cfg(unix)]
    #[test]
    fn save_keeps_existing_file_mode() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("config.toml");
        fs::write(&path, "").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

        ConfigService::save(&path, &ToktrackConfig::default()).unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
