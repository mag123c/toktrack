//! Git worktree links for the Projects tab: a project whose folder is a linked
//! worktree folds by its main repository's path rather than its own (T3 Code
//! keeps worktrees outside the repository). Runs on the loader thread, and
//! only once a root is starred, since nothing folds before that.
//!
//! Links describe the data, not the user's choices, so they are saved next to
//! the cache (`~/.toktrack/cache/worktree_links.json`) rather than in
//! config.toml. They outlive the worktree folder, which is deleted after
//! merging while its cached usage lives on, and are re-checked while the
//! folder exists, so a reused path follows its current repository.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use directories::BaseDirs;

use crate::services::project_roots::{normalize, path_rest, ProjectLinks};
use crate::types::NO_PROJECT;

/// Where git keeps a linked worktree's admin folder:
/// `<repository>/.git/worktrees/<worktree name>`. Worktrees of a bare or
/// separate-git-dir repository have no such path and are not linked.
const GIT_WORKTREES: &str = "/.git/worktrees/";

/// Worktree folder → main repository path, as saved between runs.
type Links = BTreeMap<String, String>;

/// What probing a project folder found.
#[derive(Debug, PartialEq)]
enum Probe {
    /// A linked worktree of this repository.
    Worktree(String),
    /// An existing folder that is not a linked worktree.
    Other,
    /// No such folder, e.g. a worktree deleted after merging.
    Missing,
}

/// The saved links file, `~/.toktrack/cache/worktree_links.json`.
pub fn default_links_path() -> Option<PathBuf> {
    Some(
        BaseDirs::new()?
            .home_dir()
            .join(".toktrack")
            .join("cache")
            .join("worktree_links.json"),
    )
}

/// Worktree links for the project `keys` under the starred `roots`. Without
/// a root nothing folds, so nothing is read or probed. Otherwise the links
/// saved at `links_path` are checked against the keys' `.git` files and saved
/// back if they changed (best effort: an unsaved link is just found again).
/// Folder names are indexed separately, on every load (see `ProjectLinks`).
pub fn link_projects<'a>(
    keys: impl IntoIterator<Item = &'a str>,
    roots: &[String],
    links_path: Option<&Path>,
) -> ProjectLinks {
    let mut links = ProjectLinks::default();
    if roots.is_empty() {
        return links;
    }
    let keys: Vec<&str> = keys.into_iter().collect();
    let saved = links_path.map(load_links).unwrap_or_default();
    let (aliases, updated) = resolve(&keys, &saved, probe_worktree);
    if let Some(path) = links_path.filter(|_| updated != saved) {
        let _ = save_links(path, &updated);
    }
    links.aliases = aliases;
    links.resolved_roots = resolve_roots(roots);
    links.probed = true;
    links
}

/// Aliases for `keys` from the `saved` links, re-checked (via `probe`) for
/// the folders that still exist. Returns the aliases and the updated links.
fn resolve(
    keys: &[&str],
    saved: &Links,
    probe: impl Fn(&Path) -> Probe,
) -> (HashMap<String, String>, Links) {
    let keys: Vec<(&str, String)> = keys
        .iter()
        .filter(|key| **key != NO_PROJECT)
        .map(|key| (*key, normalize(key)))
        .collect();
    let saved_paths = normalized(saved);
    let mut links = saved.clone();
    for (key, normalized) in &keys {
        // A subfolder of a linked worktree has no `.git` of its own.
        let subfolder = saved_paths.iter().any(|(worktree, _)| {
            path_rest(normalized, worktree).is_some_and(|rest| !rest.is_empty())
        });
        if subfolder || !is_local_dir(key) {
            continue;
        }
        match probe(Path::new(key)) {
            Probe::Worktree(repo) => {
                links.insert(key.to_string(), repo);
            }
            Probe::Other => {
                links.remove(*key);
            }
            // Keep a deleted worktree's link for its cached history.
            Probe::Missing => {}
        }
    }

    let paths = normalized(&links);
    let aliases = keys
        .iter()
        .filter_map(|(key, normalized)| Some((key.to_string(), repo_path(normalized, &paths)?)))
        .collect();
    (aliases, links)
}

/// `links` with normalized worktree paths, for prefix matching.
fn normalized(links: &Links) -> Vec<(String, &str)> {
    links
        .iter()
        .map(|(worktree, repo)| (normalize(worktree), repo.as_str()))
        .collect()
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

/// Whether `dir` is a linked worktree, i.e. its `.git` is a file pointing
/// into `<repository>/.git/worktrees/`.
fn probe_worktree(dir: &Path) -> Probe {
    let git = dir.join(".git");
    match fs::metadata(&git) {
        Ok(meta) if meta.is_file() => fs::read_to_string(&git)
            .ok()
            .and_then(|content| repo_from_gitfile(dir, &content))
            .map_or(Probe::Other, Probe::Worktree),
        // A main checkout's `.git` is a folder: one stat, nothing read.
        Ok(_) => Probe::Other,
        Err(_) if dir.is_dir() => Probe::Other,
        Err(_) => Probe::Missing,
    }
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

/// Starred roots whose symlink-resolved path differs from the starred one.
/// git records a worktree's repository by its resolved path, so a root
/// starred through a symlink (`/tmp` → `/private/tmp`) is matched by both.
fn resolve_roots(roots: &[String]) -> HashMap<String, String> {
    roots
        .iter()
        .filter(|root| is_local_dir(root))
        .filter_map(|root| {
            let resolved = strip_verbatim(&fs::canonicalize(root).ok()?);
            (normalize(&resolved) != normalize(root)).then(|| (root.clone(), resolved))
        })
        .collect()
}

/// `path` as a string without the verbatim prefix Windows `canonicalize`
/// adds (`\\?\C:\...`, `\\?\UNC\server\...`), so it compares with plain paths.
fn strip_verbatim(path: &Path) -> String {
    let path = path.to_string_lossy();
    match path.strip_prefix(r"\\?\") {
        Some(rest) => match rest.strip_prefix(r"UNC\") {
            Some(share) => format!(r"\\{share}"),
            None => rest.to_string(),
        },
        None => path.into_owned(),
    }
}

/// The links saved at `path`; a missing or unreadable file reads as none.
fn load_links(path: &Path) -> Links {
    fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

/// Save `links` to `path` through a sibling temp file, so a crash mid-write
/// cannot leave a truncated file behind.
fn save_links(path: &Path, links: &Links) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_vec_pretty(links).map_err(std::io::Error::other)?;
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    let write = || -> std::io::Result<()> {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(&content)?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    };
    write().inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn links(pairs: &[(&str, &str)]) -> Links {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn path_str(path: &Path) -> String {
        path.to_str().unwrap().to_string()
    }

    fn roots(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|p| p.to_string()).collect()
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
    fn probe_worktree_tells_worktrees_other_folders_and_missing_ones_apart() {
        let base = tempfile::tempdir().unwrap();
        let repo = base.path().join("repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        let worktree = base.path().join("wt");
        make_worktree(&worktree, &repo);

        match probe_worktree(&worktree) {
            Probe::Worktree(found) => assert_eq!(normalize(&found), normalize(&path_str(&repo))),
            other => panic!("expected a worktree, got {other:?}"),
        }
        // A main checkout has a `.git` folder; a plain folder has none.
        assert_eq!(probe_worktree(&repo), Probe::Other);
        assert_eq!(probe_worktree(base.path()), Probe::Other);
        assert_eq!(probe_worktree(&base.path().join("gone")), Probe::Missing);
    }

    #[test]
    fn resolve_maps_saved_links_and_their_subfolders() {
        let base = tempfile::tempdir().unwrap();
        let wt = path_str(&base.path().join("wt"));
        let sub = path_str(&base.path().join("wt").join("src"));
        let saved = links(&[(wt.as_str(), "/w/repo")]);
        let probed = RefCell::new(Vec::new());
        let (aliases, updated) = resolve(&[wt.as_str(), sub.as_str()], &saved, |dir| {
            probed.borrow_mut().push(path_str(dir));
            Probe::Missing
        });

        // Only the worktree folder itself is probed, not its subfolder.
        assert_eq!(probed.into_inner(), vec![wt.clone()]);
        assert_eq!(updated, saved);
        assert_eq!(aliases.get(&wt).map(String::as_str), Some("/w/repo"));
        assert_eq!(aliases.get(&sub).map(String::as_str), Some("/w/repo/src"));
    }

    #[test]
    fn resolve_rechecks_saved_links_while_the_folder_exists() {
        let base = tempfile::tempdir().unwrap();
        let moved = path_str(&base.path().join("moved"));
        let plain = path_str(&base.path().join("plain"));
        let saved = links(&[(moved.as_str(), "/w/a"), (plain.as_str(), "/w/a")]);
        let (aliases, updated) = resolve(&[moved.as_str(), plain.as_str()], &saved, |dir| {
            if path_str(dir) == moved {
                Probe::Worktree("/w/b".to_string())
            } else {
                Probe::Other
            }
        });

        // A reused path follows its current repository; a path that is no
        // longer a worktree loses its link.
        assert_eq!(updated, links(&[(moved.as_str(), "/w/b")]));
        assert_eq!(aliases.get(&moved).map(String::as_str), Some("/w/b"));
        assert!(!aliases.contains_key(&plain));
    }

    #[test]
    fn resolve_finds_new_worktrees_and_skips_unprobeable_keys() {
        let base = tempfile::tempdir().unwrap();
        let wt = path_str(&base.path().join("wt"));
        let sub = path_str(&base.path().join("wt").join("src"));
        let probed = RefCell::new(Vec::new());
        let keys = [
            sub.as_str(),
            wt.as_str(),
            "relative/dir",
            r"\\wsl.localhost\Ubuntu\home\me\x",
            "//server/share/x",
            NO_PROJECT,
        ];
        let (aliases, updated) = resolve(&keys, &Links::new(), |dir| {
            probed.borrow_mut().push(path_str(dir));
            if path_str(dir) == wt {
                Probe::Worktree("/w/repo".to_string())
            } else {
                Probe::Other
            }
        });

        let mut probed = probed.into_inner();
        probed.sort();
        let mut expected = vec![sub.clone(), wt.clone()];
        expected.sort();
        assert_eq!(probed, expected);
        assert_eq!(updated, links(&[(wt.as_str(), "/w/repo")]));
        assert_eq!(aliases.get(&wt).map(String::as_str), Some("/w/repo"));
        // A subfolder seen before its worktree still folds through it.
        assert_eq!(aliases.get(&sub).map(String::as_str), Some("/w/repo/src"));
        assert_eq!(aliases.len(), 2);
    }

    #[test]
    fn link_projects_without_a_root_reads_and_probes_nothing() {
        let base = tempfile::tempdir().unwrap();
        let saved = base.path().join("cache").join("worktree_links.json");
        let worktree = base.path().join("wt");
        make_worktree(&worktree, &base.path().join("repo"));
        let key = path_str(&worktree);

        let links = link_projects([key.as_str()], &[], Some(&saved));

        assert!(!links.probed);
        assert!(links.aliases.is_empty());
        assert!(!saved.exists());
    }

    #[test]
    fn link_projects_saves_links_that_outlive_the_worktree() {
        let base = tempfile::tempdir().unwrap();
        let saved = base.path().join("cache").join("worktree_links.json");
        let repo = base.path().join("repo");
        let worktree = base.path().join("wt");
        make_worktree(&worktree, &repo);
        let key = path_str(&worktree);
        let r = roots(&["/w/root"]);

        let first = link_projects([key.as_str()], &r, Some(&saved));
        fs::remove_dir_all(&worktree).unwrap();
        let second = link_projects([key.as_str()], &r, Some(&saved));

        let expected = Some(normalize(&path_str(&repo)));
        assert!(first.probed);
        assert_eq!(first.aliases.get(&key).map(|r| normalize(r)), expected);
        assert_eq!(second.aliases.get(&key).map(|r| normalize(r)), expected);
        assert!(load_links(&saved).contains_key(&key));
    }

    #[test]
    fn link_projects_rebuilds_from_an_unreadable_links_file() {
        let base = tempfile::tempdir().unwrap();
        let saved = base.path().join("worktree_links.json");
        fs::write(&saved, "{ not json").unwrap();
        let worktree = base.path().join("wt");
        make_worktree(&worktree, &base.path().join("repo"));
        let key = path_str(&worktree);

        let links = link_projects([key.as_str()], &roots(&["/w/root"]), Some(&saved));

        assert!(links.aliases.contains_key(&key));
        assert!(load_links(&saved).contains_key(&key));
    }

    #[test]
    fn link_projects_without_changes_does_not_write() {
        let base = tempfile::tempdir().unwrap();
        let saved = base.path().join("worktree_links.json");

        let links = link_projects(
            [path_str(base.path()).as_str()],
            &roots(&["/w"]),
            Some(&saved),
        );

        assert!(links.aliases.is_empty());
        assert!(!saved.exists());
    }

    #[cfg(unix)]
    #[test]
    fn resolve_roots_maps_a_symlinked_root_to_its_target() {
        let base = tempfile::tempdir().unwrap();
        let real = base.path().join("real");
        fs::create_dir_all(&real).unwrap();
        let link = base.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let resolved = resolve_roots(&roots(&[&path_str(&link), &path_str(&real)]));

        let real = path_str(&fs::canonicalize(&real).unwrap());
        assert_eq!(resolved.get(&path_str(&link)), Some(&real));
        // A root that is already resolved needs no entry (unless the temp
        // dir itself sits behind a link, as on macOS).
        assert!(resolved.len() <= 2);
    }

    #[test]
    fn strip_verbatim_drops_the_windows_prefix() {
        assert_eq!(strip_verbatim(Path::new(r"\\?\C:\w\repo")), r"C:\w\repo");
        assert_eq!(
            strip_verbatim(Path::new(r"\\?\UNC\srv\share\repo")),
            r"\\srv\share\repo"
        );
        assert_eq!(strip_verbatim(Path::new("/w/repo")), "/w/repo");
    }

    #[test]
    fn resolve_roots_skips_plain_roots_and_missing_ones() {
        let base = tempfile::tempdir().unwrap();
        let real = fs::canonicalize(base.path()).unwrap();
        let real = strip_verbatim(&real);
        let gone = path_str(&base.path().join("gone"));

        assert!(resolve_roots(&roots(&[&real, &gone, "relative"])).is_empty());
    }
}
