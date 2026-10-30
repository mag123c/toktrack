//! Git worktree links for the Projects tab: a project whose folder is a linked
//! worktree folds by its main repository's path rather than its own (T3 Code
//! keeps worktrees outside the repository). Runs on the loader thread.
//!
//! Links are persisted as `worktree_repos` in config.toml the first time they
//! are seen, because worktree folders are deleted after merging while their
//! cached usage lives on.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::services::config::ConfigService;
use crate::services::project_roots::{normalize, path_rest, ProjectAliases};
use crate::types::NO_PROJECT;

/// Where git keeps a linked worktree's admin folder:
/// `<repository>/.git/worktrees/<worktree name>`. Worktrees of a bare or
/// separate-git-dir repository have no such path and are not linked.
const GIT_WORKTREES: &str = "/.git/worktrees/";

/// Aliases for the project `keys`, from the links saved in the config at
/// `config_path` plus any found by reading the keys' `.git` files. Newly found
/// links are saved (best effort: an unsaved one is just found again).
pub fn link_projects<'a>(
    keys: impl IntoIterator<Item = &'a str>,
    config_path: Option<&Path>,
) -> ProjectAliases {
    let known = config_path
        .and_then(|path| ConfigService::load_optional(path).ok().flatten())
        .map(|config| config.worktree_repos)
        .unwrap_or_default();
    let (aliases, found) = resolve(keys, &known, probe_worktree);
    if let Some(path) = config_path.filter(|_| !found.is_empty()) {
        // An unreadable config is left alone: `update` fails before saving.
        let _ = ConfigService::update(path, |config| {
            let before = config.worktree_repos.len();
            for (worktree, repo) in found {
                config.worktree_repos.entry(worktree).or_insert(repo);
            }
            config.worktree_repos.len() != before
        });
    }
    aliases
}

/// Aliases for `keys` from the `known` links, probing (via `probe`) the
/// absolute keys no link covers. Returns the aliases and the newly found links.
fn resolve<'a>(
    keys: impl IntoIterator<Item = &'a str>,
    known: &BTreeMap<String, String>,
    probe: impl Fn(&Path) -> Option<String>,
) -> (ProjectAliases, BTreeMap<String, String>) {
    let keys: Vec<(&str, String)> = keys
        .into_iter()
        .filter(|key| *key != NO_PROJECT)
        .map(|key| (key, normalize(key)))
        .collect();
    let mut links: Vec<(String, &str)> = known
        .iter()
        .map(|(worktree, repo)| (normalize(worktree), repo.as_str()))
        .collect();

    // Only a worktree's own folder holds its `.git` file; a subfolder key is
    // covered once its worktree's link is known (see the second pass).
    let mut found = BTreeMap::new();
    for (key, normalized) in &keys {
        if repo_path(normalized, &links).is_some() || !is_local_dir(key) {
            continue;
        }
        if let Some(repo) = probe(Path::new(key)) {
            found.insert(key.to_string(), repo);
        }
    }
    links.extend(
        found
            .iter()
            .map(|(worktree, repo)| (normalize(worktree), repo.as_str())),
    );

    let aliases = keys
        .iter()
        .filter_map(|(key, normalized)| Some((key.to_string(), repo_path(normalized, &links)?)))
        .collect();
    (aliases, found)
}

/// Whether `key` is worth probing: an absolute path that is not on a network
/// share (`\\server\share`, `\\wsl$\...`), where a stat can stall the load.
fn is_local_dir(key: &str) -> bool {
    !key.starts_with(r"\\") && !key.starts_with("//") && Path::new(key).is_absolute()
}

/// The path of normalized `key` within the main repository of the deepest
/// worktree link it lies in.
fn repo_path(key: &str, links: &[(String, &str)]) -> Option<String> {
    links
        .iter()
        .filter_map(|(worktree, repo)| Some((worktree.len(), path_rest(key, worktree)?, *repo)))
        .max_by_key(|(len, _, _)| *len)
        .map(|(_, rest, repo)| format!("{}{rest}", repo.trim_end_matches(['/', '\\'])))
}

/// Main repository of `dir` if it is a linked worktree, i.e. its `.git` is a
/// file pointing into `<repository>/.git/worktrees/`.
fn probe_worktree(dir: &Path) -> Option<String> {
    let git = dir.join(".git");
    // A main checkout's `.git` is a folder: one stat, nothing read.
    if !fs::metadata(&git).ok()?.is_file() {
        return None;
    }
    repo_from_gitfile(dir, &fs::read_to_string(git).ok()?)
}

/// Main repository named by the `.git` file content of the worktree at `dir`:
/// `gitdir: <repository>/.git/worktrees/<name>`, absolute or relative to `dir`.
fn repo_from_gitfile(dir: &Path, content: &str) -> Option<String> {
    let gitdir = content.lines().next()?.strip_prefix("gitdir:")?.trim();
    let gitdir = clean(&dir.join(gitdir));
    let gitdir = gitdir.to_str()?.replace('\\', "/");
    let at = gitdir.rfind(GIT_WORKTREES)?;
    let name = &gitdir[at + GIT_WORKTREES.len()..];
    if at == 0 || name.is_empty() || name.contains('/') {
        return None;
    }
    Some(gitdir[..at].to_string())
}

/// `path` with `.` and `..` resolved lexically; the worktree may be gone, so
/// the filesystem is not consulted.
fn clean(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn links(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn path_str(path: &Path) -> String {
        path.to_str().unwrap().to_string()
    }

    /// A worktree folder at `dir` whose `.git` file points at `repo`.
    fn make_worktree(dir: &Path, repo: &Path) {
        fs::create_dir_all(dir).unwrap();
        let name = dir.file_name().unwrap().to_str().unwrap();
        let gitdir = repo.join(".git").join("worktrees").join(name);
        fs::write(dir.join(".git"), format!("gitdir: {}\n", gitdir.display())).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn repo_from_gitfile_reads_a_t3_worktree() {
        let dir = Path::new(r"C:\Users\me\.t3\worktrees\annex\chore-x");
        assert_eq!(
            repo_from_gitfile(dir, "gitdir: G:/Scripts/annex/.git/worktrees/chore-x\r\n"),
            Some("G:/Scripts/annex".to_string())
        );
    }

    #[cfg(unix)]
    #[test]
    fn repo_from_gitfile_reads_an_absolute_gitdir() {
        let dir = Path::new("/home/me/.t3/worktrees/annex/chore-x");
        assert_eq!(
            repo_from_gitfile(dir, "gitdir: /w/annex/.git/worktrees/chore-x\n"),
            Some("/w/annex".to_string())
        );
    }

    #[test]
    fn repo_from_gitfile_resolves_a_relative_gitdir() {
        let base = tempfile::tempdir().unwrap();
        let dir = base.path().join("worktrees").join("x");
        let repo = repo_from_gitfile(&dir, "gitdir: ../../repo/.git/worktrees/x").unwrap();
        assert_eq!(
            normalize(&repo),
            normalize(&path_str(&base.path().join("repo")))
        );
    }

    #[test]
    fn repo_from_gitfile_accepts_crlf_and_padding() {
        let base = tempfile::tempdir().unwrap();
        let dir = base.path().join("worktrees").join("x");
        let repo = repo_from_gitfile(&dir, "gitdir:   ../../repo/.git/worktrees/x  \r\n").unwrap();
        assert_eq!(
            normalize(&repo),
            normalize(&path_str(&base.path().join("repo")))
        );
    }

    #[test]
    fn resolve_skips_network_shares() {
        let probed = RefCell::new(0);
        let (aliases, _) = resolve(
            [r"\\wsl.localhost\Ubuntu\home\me\x", "//server/share/x"],
            &BTreeMap::new(),
            |_| {
                *probed.borrow_mut() += 1;
                Some("/w/repo".to_string())
            },
        );
        assert_eq!(*probed.borrow(), 0);
        assert!(aliases.is_empty());
    }

    #[test]
    fn link_projects_leaves_an_unparseable_config_untouched() {
        let base = tempfile::tempdir().unwrap();
        let config = base.path().join("config.toml");
        fs::write(&config, "project_roots = [").unwrap();
        let repo = base.path().join("repo");
        let worktree = base.path().join("wt");
        make_worktree(&worktree, &repo);
        let key = path_str(&worktree);

        let aliases = link_projects([key.as_str()], Some(&config));

        assert!(aliases.contains_key(&key));
        assert_eq!(fs::read_to_string(&config).unwrap(), "project_roots = [");
    }

    #[test]
    fn repo_from_gitfile_rejects_other_git_files() {
        let base = tempfile::tempdir().unwrap();
        let repo = path_str(&base.path().join("repo"));
        let dir = base.path().join("x");
        // Submodule checkouts point into `.git/modules`.
        let submodule = format!("gitdir: {repo}/.git/modules/x");
        assert_eq!(repo_from_gitfile(&dir, &submodule), None);
        let nested = format!("gitdir: {repo}/.git/worktrees/a/b");
        assert_eq!(repo_from_gitfile(&dir, &nested), None);
        let unnamed = format!("gitdir: {repo}/.git/worktrees/");
        assert_eq!(repo_from_gitfile(&dir, &unnamed), None);
        assert_eq!(repo_from_gitfile(&dir, "not a git file"), None);
        assert_eq!(repo_from_gitfile(&dir, ""), None);
    }

    #[test]
    fn probe_worktree_reads_only_a_git_file() {
        let base = tempfile::tempdir().unwrap();
        let repo = base.path().join("repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        let worktree = base.path().join("wt");
        make_worktree(&worktree, &repo);

        assert_eq!(
            probe_worktree(&worktree).map(|r| normalize(&r)),
            Some(normalize(&path_str(&repo)))
        );
        // A main checkout has a `.git` folder; a plain folder has none.
        assert_eq!(probe_worktree(&repo), None);
        assert_eq!(probe_worktree(base.path()), None);
        assert_eq!(probe_worktree(&base.path().join("gone")), None);
    }

    #[test]
    fn resolve_uses_known_links_without_probing() {
        let known = links(&[("/wt/x", "/w/repo")]);
        let probed = RefCell::new(Vec::new());
        let (aliases, found) = resolve(["/wt/x", "/wt/x/src"], &known, |dir| {
            probed.borrow_mut().push(dir.to_path_buf());
            None
        });

        assert!(probed.borrow().is_empty());
        assert!(found.is_empty());
        assert_eq!(aliases.get("/wt/x").map(String::as_str), Some("/w/repo"));
        assert_eq!(
            aliases.get("/wt/x/src").map(String::as_str),
            Some("/w/repo/src")
        );
    }

    #[test]
    fn resolve_probes_unknown_absolute_keys_once() {
        let base = tempfile::tempdir().unwrap();
        let wt = path_str(&base.path().join("wt"));
        let sub = path_str(&base.path().join("wt").join("src"));
        let other = path_str(&base.path().join("other"));
        let probed = RefCell::new(Vec::new());
        let (aliases, found) = resolve(
            [
                sub.as_str(),
                wt.as_str(),
                other.as_str(),
                "relative/dir",
                NO_PROJECT,
            ],
            &BTreeMap::new(),
            |dir| {
                probed.borrow_mut().push(path_str(dir));
                (path_str(dir) == wt).then(|| "/w/repo".to_string())
            },
        );

        let mut probed = probed.into_inner();
        probed.sort();
        let mut expected = vec![sub.clone(), wt.clone(), other.clone()];
        expected.sort();
        assert_eq!(probed, expected);
        assert_eq!(found, links(&[(wt.as_str(), "/w/repo")]));
        assert_eq!(aliases.get(&wt).map(String::as_str), Some("/w/repo"));
        // A subfolder seen before its worktree still folds through it.
        assert_eq!(aliases.get(&sub).map(String::as_str), Some("/w/repo/src"));
        assert!(!aliases.contains_key(&other));
        assert!(!aliases.contains_key("relative/dir"));
    }

    #[test]
    fn link_projects_persists_links_that_outlive_the_worktree() {
        let base = tempfile::tempdir().unwrap();
        let config = base.path().join("config.toml");
        fs::write(&config, "# mine\nproject_roots = [\"/w/a\"]\n").unwrap();
        let repo = base.path().join("repo");
        let worktree = base.path().join("wt");
        make_worktree(&worktree, &repo);
        let key = path_str(&worktree);

        let first = link_projects([key.as_str()], Some(&config));
        fs::remove_dir_all(&worktree).unwrap();
        let second = link_projects([key.as_str()], Some(&config));

        let expected = Some(normalize(&path_str(&repo)));
        assert_eq!(first.get(&key).map(|r| normalize(r)), expected);
        assert_eq!(second.get(&key).map(|r| normalize(r)), expected);
        let saved = fs::read_to_string(&config).unwrap();
        assert!(saved.contains("# mine"));
        let loaded = ConfigService::load_optional(&config).unwrap().unwrap();
        assert_eq!(loaded.project_roots, vec!["/w/a"]);
        assert!(loaded.worktree_repos.contains_key(&key));
    }

    #[test]
    fn link_projects_without_new_links_leaves_the_config_alone() {
        let base = tempfile::tempdir().unwrap();
        let config = base.path().join("config.toml");

        let aliases = link_projects([path_str(base.path()).as_str()], Some(&config));

        assert!(aliases.is_empty());
        assert!(!config.exists());
    }

    #[test]
    fn link_projects_works_without_a_config() {
        let base = tempfile::tempdir().unwrap();
        let repo = base.path().join("repo");
        let worktree = base.path().join("wt");
        make_worktree(&worktree, &repo);
        let key = path_str(&worktree);

        let aliases = link_projects([key.as_str()], None);

        assert!(aliases.contains_key(&key));
    }
}
