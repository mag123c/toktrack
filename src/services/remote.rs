//! SSH remote source support.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use directories::BaseDirs;

use crate::parsers::{CodexParser, SourceInstance};
use crate::services::config::{ConfigService, RemoteConfig, ToktrackConfig};
use crate::types::{Result, ToktrackError};

const RSYNC_SSH_COMMAND: &str = "ssh -o BatchMode=yes -o ConnectTimeout=5";

/// CLI-selected remote source options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteOptions {
    pub remote_names: Vec<String>,
    pub all_remotes: bool,
    pub local_only: bool,
}

/// Builds remote `SourceInstance`s by syncing configured Codex session files.
pub struct RemoteSourceService {
    snapshot_root: PathBuf,
}

impl RemoteSourceService {
    pub fn new() -> Result<Self> {
        let base_dirs = BaseDirs::new()
            .ok_or_else(|| ToktrackError::Config("cannot determine home directory".into()))?;
        Ok(Self {
            snapshot_root: base_dirs.home_dir().join(".toktrack").join("remotes"),
        })
    }

    pub fn sync_and_build_sources(options: &RemoteOptions) -> Result<Vec<SourceInstance>> {
        Self::new()?.sync_and_build_sources_with_options(options)
    }

    fn sync_and_build_sources_with_options(
        &self,
        options: &RemoteOptions,
    ) -> Result<Vec<SourceInstance>> {
        let config = ConfigService::load_default()?;
        let remote_names = resolve_remote_names(config.as_ref(), options)?;
        let Some(config) = config else {
            return Ok(Vec::new());
        };

        let mut sources = Vec::with_capacity(remote_names.len());
        for name in remote_names {
            let remote = config.remote(&name).ok_or_else(|| {
                ToktrackError::Config(format!("remote '{}' is not configured", name))
            })?;
            let snapshot_dir = self.snapshot_dir(remote);
            let sync_result = self.sync_codex(remote, &snapshot_dir);
            sources.push(build_source_after_sync_attempt(
                remote,
                snapshot_dir,
                sync_result,
            ));
        }

        Ok(sources)
    }

    fn snapshot_dir(&self, remote: &RemoteConfig) -> PathBuf {
        self.snapshot_root
            .join(&remote.name)
            .join("codex")
            .join("sessions")
    }

    fn sync_codex(&self, remote: &RemoteConfig, snapshot_dir: &Path) -> Result<()> {
        fs::create_dir_all(snapshot_dir)?;
        let args = build_rsync_args(remote, snapshot_dir);
        let output = Command::new("rsync").args(&args).output().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ToktrackError::Remote("rsync not found; install rsync to use remote sources".into())
            } else {
                ToktrackError::Io(e)
            }
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ToktrackError::Remote(format!(
                "failed to sync remote '{}': {}",
                remote.name,
                stderr.trim()
            )));
        }

        Ok(())
    }
}

pub fn resolve_remote_names(
    config: Option<&ToktrackConfig>,
    options: &RemoteOptions,
) -> Result<Vec<String>> {
    if options.local_only && (!options.remote_names.is_empty() || options.all_remotes) {
        return Err(ToktrackError::Config(
            "--local-only cannot be combined with --remote or --all-remotes".into(),
        ));
    }

    if options.local_only {
        return Ok(Vec::new());
    }

    let Some(config) = config else {
        if options.remote_names.is_empty() && !options.all_remotes {
            return Ok(Vec::new());
        }
        return Err(ToktrackError::Config(
            "remote sources require ~/.toktrack/config.toml".into(),
        ));
    };

    if options.all_remotes {
        return Ok(config.remote_names());
    }

    let mut names = Vec::new();
    for name in config
        .default_remotes
        .iter()
        .chain(options.remote_names.iter())
    {
        if config.remote(name).is_none() {
            let available = config.remote_names().join(", ");
            return Err(ToktrackError::Config(format!(
                "remote '{}' is not configured{}",
                name,
                if available.is_empty() {
                    String::new()
                } else {
                    format!("; available remotes: {}", available)
                }
            )));
        }
        if !names.contains(name) {
            names.push(name.clone());
        }
    }

    Ok(names)
}

fn build_codex_source(remote: &RemoteConfig, snapshot_dir: PathBuf) -> SourceInstance {
    SourceInstance::new(
        format!("codex@{}", remote.name),
        format!("codex ({})", remote.name),
        "codex",
        Box::new(CodexParser::with_data_dir(snapshot_dir)),
    )
}

fn build_source_after_sync_attempt(
    remote: &RemoteConfig,
    snapshot_dir: PathBuf,
    sync_result: Result<()>,
) -> SourceInstance {
    if let Err(error) = sync_result {
        warn_sync_failed(remote, &snapshot_dir, &error);
    }
    build_codex_source(remote, snapshot_dir)
}

pub fn build_rsync_args(remote: &RemoteConfig, snapshot_dir: &Path) -> Vec<OsString> {
    vec![
        OsString::from("-az"),
        OsString::from("--delete"),
        OsString::from("-e"),
        OsString::from(RSYNC_SSH_COMMAND),
        OsString::from("--include"),
        OsString::from("*/"),
        OsString::from("--include"),
        OsString::from("*.jsonl"),
        OsString::from("--exclude"),
        OsString::from("*"),
        OsString::from("--"),
        OsString::from(remote_codex_spec(remote)),
        snapshot_dir.as_os_str().to_os_string(),
    ]
}

fn warn_sync_failed(remote: &RemoteConfig, snapshot_dir: &Path, error: &ToktrackError) {
    eprintln!(
        "[toktrack] Warning: {}. Using existing snapshot/cache for remote '{}' if available (snapshot: {}).",
        error,
        remote.name,
        snapshot_dir.display()
    );
}

fn remote_codex_spec(remote: &RemoteConfig) -> String {
    format!(
        "{}:{}/",
        remote.target,
        remote.codex_sessions_path().trim_end_matches('/')
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::config::RemotePaths;

    fn config() -> ToktrackConfig {
        ToktrackConfig {
            default_remotes: vec!["devbox".to_string()],
            remotes: vec![
                RemoteConfig {
                    name: "devbox".to_string(),
                    target: "ubuntu@devbox".to_string(),
                    paths: RemotePaths {
                        codex: Some("~/.codex/sessions".to_string()),
                    },
                },
                RemoteConfig {
                    name: "prod".to_string(),
                    target: "prod-alias".to_string(),
                    paths: RemotePaths {
                        codex: Some("/home/codex/.codex/sessions".to_string()),
                    },
                },
            ],
        }
    }

    #[test]
    fn default_remotes_are_used_without_cli_overrides() {
        let options = RemoteOptions::default();
        let names = resolve_remote_names(Some(&config()), &options).unwrap();
        assert_eq!(names, vec!["devbox"]);
    }

    #[test]
    fn explicit_remotes_are_appended_after_defaults() {
        let options = RemoteOptions {
            remote_names: vec!["prod".to_string(), "devbox".to_string()],
            all_remotes: false,
            local_only: false,
        };
        let names = resolve_remote_names(Some(&config()), &options).unwrap();
        assert_eq!(names, vec!["devbox", "prod"]);
    }

    #[test]
    fn local_only_ignores_default_remotes() {
        let options = RemoteOptions {
            local_only: true,
            ..RemoteOptions::default()
        };
        let names = resolve_remote_names(Some(&config()), &options).unwrap();
        assert!(names.is_empty());
    }

    #[test]
    fn all_remotes_uses_config_order() {
        let options = RemoteOptions {
            all_remotes: true,
            ..RemoteOptions::default()
        };
        let names = resolve_remote_names(Some(&config()), &options).unwrap();
        assert_eq!(names, vec!["devbox", "prod"]);
    }

    #[test]
    fn local_only_conflicts_with_explicit_remote() {
        let options = RemoteOptions {
            remote_names: vec!["devbox".to_string()],
            local_only: true,
            all_remotes: false,
        };
        let error = resolve_remote_names(Some(&config()), &options).unwrap_err();
        assert!(error.to_string().contains("cannot be combined"));
    }

    #[test]
    fn local_only_conflicts_with_all_remotes() {
        let options = RemoteOptions {
            all_remotes: true,
            local_only: true,
            ..RemoteOptions::default()
        };
        let error = resolve_remote_names(Some(&config()), &options).unwrap_err();
        assert!(error.to_string().contains("cannot be combined"));
    }

    #[test]
    fn unknown_remote_returns_available_names() {
        let options = RemoteOptions {
            remote_names: vec!["unknown".to_string()],
            ..RemoteOptions::default()
        };
        let error = resolve_remote_names(Some(&config()), &options).unwrap_err();
        assert!(error
            .to_string()
            .contains("available remotes: devbox, prod"));
    }

    #[test]
    fn missing_config_without_remote_options_is_local_only() {
        let options = RemoteOptions::default();
        let names = resolve_remote_names(None, &options).unwrap();
        assert!(names.is_empty());
    }

    #[test]
    fn missing_config_with_explicit_remote_is_error() {
        let options = RemoteOptions {
            remote_names: vec!["devbox".to_string()],
            ..RemoteOptions::default()
        };
        let error = resolve_remote_names(None, &options).unwrap_err();
        assert!(error.to_string().contains("config.toml"));
    }

    #[test]
    fn builds_remote_codex_source_identity() {
        let remote = &config().remotes[0];
        let source = build_codex_source(remote, PathBuf::from("/tmp/devbox/codex/sessions"));

        assert_eq!(source.id, "codex@devbox");
        assert_eq!(source.label, "codex (devbox)");
        assert_eq!(source.kind, "codex");
    }

    #[test]
    fn sync_failure_still_returns_remote_source_for_cache_fallback() {
        let remote = &config().remotes[0];
        let source = build_source_after_sync_attempt(
            remote,
            PathBuf::from("/tmp/devbox/codex/sessions"),
            Err(ToktrackError::Remote("boom".into())),
        );

        assert_eq!(source.id, "codex@devbox");
        assert_eq!(source.kind, "codex");
    }

    #[test]
    fn builds_rsync_args_for_codex_sessions() {
        let remote = &config().remotes[1];
        let args = build_rsync_args(remote, Path::new("/tmp/toktrack/prod"));
        let args: Vec<String> = args
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();

        assert_eq!(
            args,
            vec![
                "-az",
                "--delete",
                "-e",
                "ssh -o BatchMode=yes -o ConnectTimeout=5",
                "--include",
                "*/",
                "--include",
                "*.jsonl",
                "--exclude",
                "*",
                "--",
                "prod-alias:/home/codex/.codex/sessions/",
                "/tmp/toktrack/prod"
            ]
        );
    }
}
