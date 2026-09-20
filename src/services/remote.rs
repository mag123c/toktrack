//! SSH remote source support.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use directories::BaseDirs;

use crate::parsers::{CodexParser, SourceInstance, ARCHIVED_SESSIONS_DIR, SESSIONS_DIR};
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
            .join(SESSIONS_DIR)
    }

    fn sync_codex(&self, remote: &RemoteConfig, snapshot_dir: &Path) -> Result<()> {
        fs::create_dir_all(snapshot_dir)?;
        let output = run_rsync(&build_rsync_args(remote, snapshot_dir))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ToktrackError::Remote(format!(
                "failed to sync remote '{}': {}",
                remote.name,
                stderr.trim()
            )));
        }

        sync_codex_archive(remote, snapshot_dir);

        Ok(())
    }
}

/// Mirror the remote's archived sessions next to the sessions snapshot.
///
/// Best effort on purpose: the sessions snapshot is already synced and usable,
/// so a failure here warns and leaves it intact rather than failing the source.
fn sync_codex_archive(remote: &RemoteConfig, snapshot_dir: &Path) {
    let Some(dest) = archive_snapshot_dir(snapshot_dir) else {
        return;
    };
    let Some(spec) = remote_codex_archive_spec(remote) else {
        // The snapshot directory is always named `sessions`, so the parser keeps
        // scanning this sibling even after the remote is repointed at a custom
        // path. Drop the snapshot the way `--delete` drops removed sessions,
        // otherwise its usage is counted forever.
        discard_archive_snapshot(remote, &dest);
        return;
    };
    // rsync creates the destination itself, so a host that never archived a
    // session leaves no empty directory behind.
    let output = match run_rsync(&build_rsync_args_for(&spec, &dest)) {
        Ok(output) => output,
        Err(e) => {
            warn_archive_sync_failed(remote, &e.to_string());
            return;
        }
    };
    if output.status.success() {
        return;
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    settle_failed_archive_sync(remote, &dest, output.status.code(), &stderr);
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
    build_rsync_args_for(&remote_codex_spec(remote), snapshot_dir)
}

fn build_rsync_args_for(spec: &str, dest: &Path) -> Vec<OsString> {
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
        OsString::from(spec),
        dest.as_os_str().to_os_string(),
    ]
}

/// Sibling `archived_sessions` path for a remote `sessions` root.
///
/// `None` when the configured path is not a standard `sessions` directory, so a
/// custom remote layout stays isolated exactly as a custom local data directory
/// does in `CodexParser::collect_files`.
fn remote_codex_archive_path(sessions_path: &str) -> Option<String> {
    let path = sessions_path.trim_end_matches('/');
    let separator = path.rfind('/');
    let last = match separator {
        Some(index) => &path[index + 1..],
        None => path,
    };
    if last != SESSIONS_DIR {
        return None;
    }
    let prefix = match separator {
        Some(index) => &path[..=index],
        None => "",
    };
    Some(format!("{}{}", prefix, ARCHIVED_SESSIONS_DIR))
}

fn remote_codex_archive_spec(remote: &RemoteConfig) -> Option<String> {
    remote_codex_archive_path(remote.codex_sessions_path())
        .map(|path| format!("{}:{}/", remote.target, path))
}

/// Local snapshot directory for archived sessions: the sibling of the sessions
/// snapshot, which is where `CodexParser` scans for archives.
fn archive_snapshot_dir(snapshot_dir: &Path) -> Option<PathBuf> {
    snapshot_dir
        .parent()
        .map(|parent| parent.join(ARCHIVED_SESSIONS_DIR))
}

/// Whether an rsync failure means the remote has no archive directory to copy.
///
/// rsync exits 23 when it cannot read the source, which for a top-level source
/// means the directory is missing: the normal state of a host that has never
/// archived a session. The stderr text is deliberately not matched, because it
/// is the remote host's `strerror()` output and therefore locale-dependent.
fn archive_source_absent(code: Option<i32>) -> bool {
    code == Some(23)
}

/// Apply an archive-sync failure to the local snapshot.
///
/// A missing remote source drops the snapshot, mirroring what `--delete` does
/// for removed sessions; leaving it would keep counting usage the remote no
/// longer has. Any other failure keeps the snapshot, which is still the best
/// known state, and warns.
fn settle_failed_archive_sync(remote: &RemoteConfig, dest: &Path, code: Option<i32>, stderr: &str) {
    if archive_source_absent(code) {
        discard_archive_snapshot(remote, dest);
        return;
    }
    warn_archive_sync_failed(remote, stderr.trim());
}

fn run_rsync(args: &[OsString]) -> Result<std::process::Output> {
    Command::new("rsync").args(args).output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ToktrackError::Remote("rsync not found; install rsync to use remote sources".into())
        } else {
            ToktrackError::Io(e)
        }
    })
}

fn discard_archive_snapshot(remote: &RemoteConfig, dest: &Path) {
    if !dest.exists() {
        return;
    }
    if let Err(e) = fs::remove_dir_all(dest) {
        eprintln!(
            "[toktrack] Warning: stale archive snapshot for remote '{}' could not be removed ({}): {}",
            remote.name,
            dest.display(),
            e
        );
    }
}

fn warn_archive_sync_failed(remote: &RemoteConfig, reason: &str) {
    eprintln!(
        "[toktrack] Warning: archived sessions for remote '{}' were not synced: {}. Continuing with active sessions only.",
        remote.name, reason
    );
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
    use crate::parsers::CLIParser;
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

    #[test]
    fn archive_path_is_derived_from_a_standard_sessions_root() {
        assert_eq!(
            remote_codex_archive_path("~/.codex/sessions").as_deref(),
            Some("~/.codex/archived_sessions")
        );
        assert_eq!(
            remote_codex_archive_path("/home/codex/.codex/sessions").as_deref(),
            Some("/home/codex/.codex/archived_sessions")
        );
    }

    #[test]
    fn archive_path_ignores_a_trailing_slash() {
        assert_eq!(
            remote_codex_archive_path("~/.codex/sessions/").as_deref(),
            Some("~/.codex/archived_sessions")
        );
    }

    #[test]
    fn archive_path_is_none_for_a_custom_sessions_directory() {
        // Custom remote layouts stay isolated, matching how a custom local
        // data directory never picks up a sibling archive.
        assert_eq!(remote_codex_archive_path("~/logs/codex"), None);
        assert_eq!(remote_codex_archive_path("~/.codex/sessions-old"), None);
        assert_eq!(remote_codex_archive_path(""), None);
    }

    #[test]
    fn archive_path_handles_root_level_and_relative_sessions_directories() {
        assert_eq!(
            remote_codex_archive_path("/sessions").as_deref(),
            Some("/archived_sessions")
        );
        assert_eq!(
            remote_codex_archive_path("sessions").as_deref(),
            Some("archived_sessions")
        );
    }

    #[test]
    fn archive_spec_targets_the_remote_sibling_directory() {
        let remote = &config().remotes[1];
        assert_eq!(
            remote_codex_archive_spec(remote).as_deref(),
            Some("prod-alias:/home/codex/.codex/archived_sessions/")
        );
    }

    #[test]
    fn builds_rsync_args_for_codex_archived_sessions() {
        let remote = &config().remotes[1];
        let spec = remote_codex_archive_spec(remote).unwrap();
        let args = build_rsync_args_for(&spec, Path::new("/tmp/toktrack/prod/archived_sessions"));
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
                "prod-alias:/home/codex/.codex/archived_sessions/",
                "/tmp/toktrack/prod/archived_sessions"
            ]
        );
    }

    #[test]
    fn archive_snapshot_dir_is_where_the_parser_looks_for_archives() {
        // The sync destination is only useful if it is the exact directory the
        // Codex parser scans as the sibling archive, so assert against the
        // parser instead of restating its path formula.
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("codex").join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        let archive = archive_snapshot_dir(&sessions).expect("archive snapshot dir");
        std::fs::create_dir_all(&archive).unwrap();
        let synced = archive.join("archived.jsonl");
        std::fs::copy("tests/fixtures/codex/cwd-session.jsonl", &synced).unwrap();

        let collected = CodexParser::with_data_dir(sessions).collect_files();

        assert_eq!(collected, vec![synced]);
    }

    #[test]
    fn an_unreadable_archive_source_is_classified_as_absent() {
        // Keyed on the exit code alone: the stderr text comes from the remote
        // host's strerror() and changes with its locale.
        assert!(archive_source_absent(Some(23)));
    }

    #[test]
    fn transport_failures_are_not_classified_as_absent() {
        assert!(!archive_source_absent(Some(255)));
        assert!(!archive_source_absent(Some(12)));
        assert!(!archive_source_absent(Some(30)));
        assert!(!archive_source_absent(None));
    }

    #[test]
    fn an_archive_removed_on_the_remote_discards_the_local_snapshot() {
        // Same "counted forever" hazard as a repointed remote: rsync fails
        // before transferring, so --delete never clears the stale snapshot.
        let (_temp, sessions, archive) = snapshot_with_archived_session();
        let remote = standard_remote();

        settle_failed_archive_sync(&remote, &archive, Some(23), "unreadable source");

        assert!(!archive.exists());
        assert!(CodexParser::with_data_dir(sessions)
            .collect_files()
            .is_empty());
    }

    #[test]
    fn an_unreachable_remote_keeps_the_local_archive_snapshot() {
        // The snapshot is still the best known state, so a transport failure
        // must not throw away already-synced archives.
        let (_temp, sessions, archive) = snapshot_with_archived_session();
        let synced = archive.join("archived.jsonl");
        let remote = standard_remote();

        settle_failed_archive_sync(&remote, &archive, Some(255), "Connection refused");

        assert!(archive.exists());
        assert_eq!(
            CodexParser::with_data_dir(sessions).collect_files(),
            vec![synced]
        );
    }

    /// Snapshot layout with one already-synced archived session. The caller keeps
    /// the returned `TempDir` alive for as long as it uses the paths.
    fn snapshot_with_archived_session() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("codex").join(SESSIONS_DIR);
        std::fs::create_dir_all(&sessions).unwrap();
        let archive = archive_snapshot_dir(&sessions).unwrap();
        std::fs::create_dir_all(&archive).unwrap();
        std::fs::copy(
            "tests/fixtures/codex/cwd-session.jsonl",
            archive.join("archived.jsonl"),
        )
        .unwrap();
        (temp, sessions, archive)
    }

    fn standard_remote() -> RemoteConfig {
        RemoteConfig::new(
            "devbox",
            "ubuntu@devbox",
            Some("~/.codex/sessions".to_string()),
        )
    }

    #[test]
    fn a_custom_remote_path_discards_a_previously_synced_archive_snapshot() {
        // Repointing a remote at a custom path must not leave its old archive
        // snapshot behind, because the parser keeps scanning that sibling.
        let (_temp, sessions, archive) = snapshot_with_archived_session();
        let remote = RemoteConfig::new("devbox", "ubuntu@devbox", Some("~/logs/codex".to_string()));

        sync_codex_archive(&remote, &sessions);

        assert!(!archive.exists());
        assert!(CodexParser::with_data_dir(sessions)
            .collect_files()
            .is_empty());
    }
}
