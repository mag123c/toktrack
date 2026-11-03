//! User-starred project "root" folders: projects whose path lies under a root
//! are folded into it for the Projects tab. A display preference only — the
//! cached per-project data is never rewritten.
//!
//! A git worktree project folds by its main repository's path instead (see
//! `services::worktrees`), and a T3 Code worktree with no known repository
//! folds by name into the root its folder is named after.

use std::collections::HashMap;

use crate::services::Aggregator;
use crate::types::{DailySummary, ProjectUsage, NO_PROJECT};

/// What the loader found out about the project keys for folding (see
/// `services::worktrees::link_projects`). Built off the UI thread, so
/// refolding never touches the filesystem.
#[derive(Debug, Default, Clone)]
pub struct ProjectLinks {
    /// Worktree keys mapped to the same folder in their main repository.
    /// Only compared against roots, never shown.
    pub aliases: HashMap<String, String>,
    /// Starred roots mapped to their symlink-resolved path, where it differs:
    /// git records a worktree's repository by its resolved path.
    pub resolved_roots: HashMap<String, String>,
    /// Normalized project keys outside T3's worktree folder, by folder name
    /// (see `name_key`), so the name fallback can spot same-named projects.
    /// Pure string work, so it is built on every load, probed or not.
    pub names: HashMap<String, Vec<String>>,
    /// Whether worktrees were probed. Without a starred root nothing folds,
    /// so the loader skips probing until a root is starred.
    pub probed: bool,
}

impl ProjectLinks {
    /// Index the folder names of `keys` for the T3 name fallback.
    pub fn index_names<'a>(&mut self, keys: impl IntoIterator<Item = &'a str>) {
        for key in keys.into_iter().filter(|key| *key != NO_PROJECT) {
            let key = normalize(key);
            if key.contains(T3_WORKTREES) {
                continue;
            }
            if let Some(name) = key.rsplit('/').next().filter(|name| !name.is_empty()) {
                self.names.entry(name_key(name)).or_default().push(key);
            }
        }
    }
}

/// Folder T3 Code keeps its worktrees in, as `<this>/<repo name>/<branch>`.
/// The only tool-specific name rule: other tools' worktrees should fold
/// through their `.git` link (`services::worktrees`), not more name matching.
const T3_WORKTREES: &str = "/.t3/worktrees/";

/// Star `path` as a root, or unstar it if already starred. Returns whether it
/// is a root afterwards.
pub fn toggle(roots: &mut Vec<String>, path: &str) -> bool {
    let original_len = roots.len();
    roots.retain(|root| root != path);
    if roots.len() != original_len {
        return false;
    }
    roots.push(path.to_string());
    true
}

/// The root project `key` folds into. A worktree (a key with an alias) folds
/// by its repository's path and then its own; any other key by its own path
/// and then, inside T3's worktree folder, by the repository name.
pub fn fold_target<'a>(key: &str, links: &ProjectLinks, roots: &'a [String]) -> Option<&'a str> {
    match links.aliases.get(key) {
        Some(repo_path) => root_for_repo(repo_path, links, roots).or_else(|| root_for(key, roots)),
        None => root_for(key, roots).or_else(|| root_for_t3_worktree(key, links, roots)),
    }
}

/// The deepest root that `key` equals or lies under, if any.
pub fn root_for<'a>(key: &str, roots: &'a [String]) -> Option<&'a str> {
    if roots.is_empty() || key == NO_PROJECT {
        return None;
    }
    let key = normalize(key);
    roots
        .iter()
        .map(|root| (root, normalize(root)))
        .filter(|(_, root)| path_rest(&key, root).is_some())
        .max_by_key(|(_, root)| root.len())
        .map(|(root, _)| root.as_str())
}

/// The root a worktree's repository path lies under, by the root's path as
/// starred or, failing that, as symlinks resolve it.
fn root_for_repo<'a>(repo: &str, links: &ProjectLinks, roots: &'a [String]) -> Option<&'a str> {
    root_for(repo, roots).or_else(|| {
        let repo = normalize(repo);
        roots
            .iter()
            .filter_map(|root| Some((root, normalize(links.resolved_roots.get(root)?))))
            .filter(|(_, resolved)| path_rest(&repo, resolved).is_some())
            .max_by_key(|(_, resolved)| resolved.len())
            .map(|(root, _)| root.as_str())
    })
}

/// The only root named like the repository of the T3 worktree `key` lies in,
/// ignoring case and `_` vs `-` (T3 may rename `foo_bar` to `foo-bar`). A
/// same-named project outside that root, starred or not, makes the name
/// ambiguous, so the key is left unfolded.
fn root_for_t3_worktree<'a>(
    key: &str,
    links: &ProjectLinks,
    roots: &'a [String],
) -> Option<&'a str> {
    if roots.is_empty() || key == NO_PROJECT {
        return None;
    }
    let key = normalize(key);
    let start = key.find(T3_WORKTREES)? + T3_WORKTREES.len();
    let repo = key[start..]
        .split('/')
        .next()
        .filter(|name| !name.is_empty())?;
    let repo = name_key(repo);
    let mut matches = roots.iter().filter(|root| {
        normalize(root)
            .rsplit('/')
            .next()
            .is_some_and(|name| name_key(name) == repo)
    });
    let root = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    let root_path = normalize(root);
    let clash = links.names.get(&repo).is_some_and(|keys| {
        keys.iter()
            .any(|other| path_rest(other, &root_path).is_none())
    });
    (!clash).then_some(root.as_str())
}

/// Folder name in the form T3 worktree names are compared in.
fn name_key(name: &str) -> String {
    name.to_lowercase().replace('_', "-")
}

/// The part of normalized `path` below normalized `base` (empty, or starting
/// with `/`), if `path` equals or lies under `base`.
pub(crate) fn path_rest<'a>(path: &'a str, base: &str) -> Option<&'a str> {
    path.strip_prefix(base)
        .filter(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// Comparable form of a path: `/` separators, no trailing separator, and
/// lowercased for Windows-style paths (drive letter or backslash), whose
/// filesystems are case-insensitive.
pub(crate) fn normalize(path: &str) -> String {
    let windows = path.contains('\\') || path.as_bytes().get(1) == Some(&b':');
    let unified = path.replace('\\', "/");
    let trimmed = unified.trim_end_matches('/');
    if windows {
        trimmed.to_lowercase()
    } else {
        trimmed.to_string()
    }
}

/// Key a project is shown under: its root, or itself when not under one.
pub fn display_key<'a>(key: &'a str, links: &ProjectLinks, roots: &'a [String]) -> &'a str {
    fold_target(key, links, roots).unwrap_or(key)
}

/// Re-key a rolled-up project map onto roots, merging folded projects.
pub fn collapse_usage(
    map: &HashMap<String, ProjectUsage>,
    links: &ProjectLinks,
    roots: &[String],
) -> HashMap<String, ProjectUsage> {
    if roots.is_empty() {
        return map.clone();
    }
    let mut out: HashMap<String, ProjectUsage> = HashMap::new();
    for (key, usage) in map {
        out.entry(display_key(key, links, roots).to_string())
            .or_default()
            .merge(usage);
    }
    out
}

/// Re-key per-project daily summaries onto roots, merging same-date days.
pub fn collapse_daily(
    map: &HashMap<String, Vec<DailySummary>>,
    links: &ProjectLinks,
    roots: &[String],
) -> HashMap<String, Vec<DailySummary>> {
    if roots.is_empty() {
        return map.clone();
    }
    let mut grouped: HashMap<String, Vec<DailySummary>> = HashMap::new();
    for (key, days) in map {
        grouped
            .entry(display_key(key, links, roots).to_string())
            .or_default()
            .extend(days.iter().cloned());
    }
    grouped
        .into_iter()
        .map(|(key, days)| (key, Aggregator::merge_by_date(days)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn roots(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|p| p.to_string()).collect()
    }

    fn usage(input: u64, cost: f64) -> ProjectUsage {
        ProjectUsage {
            input_tokens: input,
            cost_usd: cost,
            count: 1,
            ..Default::default()
        }
    }

    fn day(d: u32, input: u64) -> DailySummary {
        DailySummary {
            date: NaiveDate::from_ymd_opt(2026, 1, d).unwrap(),
            total_input_tokens: input,
            total_output_tokens: 0,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            total_reasoning_tokens: 0,
            total_cache_creation_5m_tokens: 0,
            total_cache_creation_1h_tokens: 0,
            total_web_search_requests: 0,
            total_cost_usd: 0.0,
            models: HashMap::new(),
            projects: HashMap::new(),
        }
    }

    #[test]
    fn root_for_matches_itself_and_descendants() {
        let r = roots(&["/w/repo"]);
        assert_eq!(root_for("/w/repo", &r), Some("/w/repo"));
        assert_eq!(root_for("/w/repo/a", &r), Some("/w/repo"));
        assert_eq!(root_for("/w/repo/a/b", &r), Some("/w/repo"));
        assert_eq!(root_for("/w/other", &r), None);
    }

    #[test]
    fn root_for_respects_component_boundary() {
        let r = roots(&["/w/repo"]);
        assert_eq!(root_for("/w/repo-two", &r), None);
        assert_eq!(root_for("/w/rep", &r), None);
    }

    #[test]
    fn root_for_deepest_root_wins() {
        let r = roots(&["/w", "/w/big"]);
        assert_eq!(root_for("/w/big/sub", &r), Some("/w/big"));
        assert_eq!(root_for("/w/small", &r), Some("/w"));
    }

    #[test]
    fn root_for_ignores_trailing_separator() {
        let r = roots(&["/w/repo/"]);
        assert_eq!(root_for("/w/repo", &r), Some("/w/repo/"));
        assert_eq!(root_for("/w/repo/a", &r), Some("/w/repo/"));
    }

    #[test]
    fn root_for_windows_paths_are_case_and_separator_insensitive() {
        let r = roots(&["G:\\Work\\Repo"]);
        assert_eq!(root_for("g:\\work\\repo\\sub", &r), Some("G:\\Work\\Repo"));
        assert_eq!(root_for("G:/Work/Repo/sub", &r), Some("G:\\Work\\Repo"));
    }

    #[test]
    fn root_for_unix_paths_stay_case_sensitive() {
        let r = roots(&["/w/Repo"]);
        assert_eq!(root_for("/w/repo/sub", &r), None);
    }

    #[test]
    fn root_for_filesystem_root() {
        let r = roots(&["/"]);
        assert_eq!(root_for("/w/repo", &r), Some("/"));
    }

    #[test]
    fn root_for_windows_drive_root_unc_and_trailing_backslash() {
        let r = roots(&[r"C:\"]);
        assert_eq!(root_for(r"C:\work\repo", &r), Some(r"C:\"));

        let r = roots(&[r"\\srv\share"]);
        assert_eq!(root_for(r"\\srv\share\x", &r), Some(r"\\srv\share"));
        assert_eq!(root_for(r"\\srv\shared", &r), None);

        let r = roots(&[r"G:\work\"]);
        assert_eq!(root_for("G:/work/a", &r), Some(r"G:\work\"));
    }

    #[test]
    fn root_for_never_matches_no_project() {
        let r = roots(&[NO_PROJECT]);
        assert_eq!(root_for(NO_PROJECT, &r), None);
    }

    fn aliases(pairs: &[(&str, &str)]) -> ProjectLinks {
        ProjectLinks {
            aliases: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            ..Default::default()
        }
    }

    fn named(keys: &[&str]) -> ProjectLinks {
        let mut links = ProjectLinks::default();
        links.index_names(keys.iter().copied());
        links
    }

    #[test]
    fn fold_target_follows_a_worktree_alias_into_its_repo_root() {
        let r = roots(&[r"G:\Scripts\annex"]);
        let a = aliases(&[(r"C:\u\.t3\worktrees\annex\x", "G:/Scripts/annex")]);
        assert_eq!(
            fold_target(r"C:\u\.t3\worktrees\annex\x", &a, &r),
            Some(r"G:\Scripts\annex")
        );
    }

    #[test]
    fn fold_target_alias_beats_a_path_root() {
        let r = roots(&["/home/u", "/w/repo"]);
        let a = aliases(&[("/home/u/wt/x", "/w/repo")]);
        assert_eq!(fold_target("/home/u/wt/x", &a, &r), Some("/w/repo"));
    }

    #[test]
    fn fold_target_alias_to_an_unstarred_repo_falls_back_to_the_path() {
        let a = aliases(&[("/home/u/wt/x", "/w/repo")]);
        assert_eq!(fold_target("/home/u/wt/x", &a, &roots(&["/w/other"])), None);
        assert_eq!(
            fold_target("/home/u/wt/x", &a, &roots(&["/home/u/wt/x"])),
            Some("/home/u/wt/x")
        );
    }

    #[test]
    fn fold_target_without_alias_matches_by_path() {
        let r = roots(&["/w/repo"]);
        assert_eq!(
            fold_target("/w/repo/a", &ProjectLinks::default(), &r),
            Some("/w/repo")
        );
        assert_eq!(fold_target("/w/other", &ProjectLinks::default(), &r), None);
    }

    #[test]
    fn fold_target_t3_worktree_name_ignores_case_and_underscore_vs_hyphen() {
        let r = roots(&[r"G:\Scripts\rngdle_solver", r"G:\Scripts\annex"]);
        let none = ProjectLinks::default();
        assert_eq!(
            fold_target(r"C:\u\.t3\worktrees\rngdle-solver\fix-x", &none, &r),
            Some(r"G:\Scripts\rngdle_solver")
        );
        assert_eq!(
            fold_target(r"C:\u\.t3\worktrees\Annex\x\src", &none, &r),
            Some(r"G:\Scripts\annex")
        );
        let r = roots(&["/w/Raven"]);
        assert_eq!(
            fold_target("/home/u/.t3/worktrees/raven/x", &none, &r),
            Some("/w/Raven")
        );
    }

    #[test]
    fn fold_target_t3_worktree_name_needs_a_unique_match() {
        let r = roots(&["/w/annex", "/old/annex"]);
        assert_eq!(
            fold_target(
                "/home/u/.t3/worktrees/annex/x",
                &ProjectLinks::default(),
                &r
            ),
            None
        );
    }

    #[test]
    fn fold_target_t3_worktree_name_is_only_a_fallback() {
        // A known link wins, even when its repo is not starred.
        let r = roots(&["/w/annex"]);
        let a = aliases(&[("/home/u/.t3/worktrees/annex/x", "/elsewhere/annex")]);
        assert_eq!(fold_target("/home/u/.t3/worktrees/annex/x", &a, &r), None);
        // A starred path match wins over the name.
        let r = roots(&["/w/annex", "/home/u/.t3"]);
        assert_eq!(
            fold_target(
                "/home/u/.t3/worktrees/annex/x",
                &ProjectLinks::default(),
                &r
            ),
            Some("/home/u/.t3")
        );
    }

    #[test]
    fn fold_target_t3_worktree_name_refuses_a_same_named_project_elsewhere() {
        // ~/personal/web is not starred, but its deleted worktrees must not
        // be credited to the starred /work/web.
        let r = roots(&["/work/web"]);
        let links = named(&[
            "/work/web",
            "/home/u/personal/web",
            "/home/u/.t3/worktrees/web/x",
        ]);
        assert_eq!(fold_target("/home/u/.t3/worktrees/web/x", &links, &r), None);
        // `_` vs `-` counts as the same name here too.
        let links = named(&["/home/u/personal/my_app"]);
        assert_eq!(
            fold_target(
                "/home/u/.t3/worktrees/my-app/x",
                &links,
                &roots(&["/work/my-app"])
            ),
            None
        );
    }

    #[test]
    fn fold_target_t3_worktree_name_allows_same_named_folders_inside_the_root() {
        let r = roots(&["/work/web"]);
        let links = named(&["/work/web", "/work/web/packages/web"]);
        assert_eq!(
            fold_target("/home/u/.t3/worktrees/web/x", &links, &r),
            Some("/work/web")
        );
    }

    #[test]
    fn fold_target_matches_a_symlinked_root_by_its_resolved_path() {
        // git writes the resolved repo path; the root was starred via a link.
        let r = roots(&["/tmp/repo"]);
        let mut links = aliases(&[("/home/u/.t3/worktrees/repo/x", "/private/tmp/repo")]);
        links
            .resolved_roots
            .insert("/tmp/repo".to_string(), "/private/tmp/repo".to_string());
        assert_eq!(
            fold_target("/home/u/.t3/worktrees/repo/x", &links, &r),
            Some("/tmp/repo")
        );
    }

    #[test]
    fn fold_target_ignores_paths_outside_t3_worktrees() {
        let r = roots(&["/w/annex"]);
        let none = ProjectLinks::default();
        assert_eq!(fold_target("/home/u/worktrees/annex/x", &none, &r), None);
        assert_eq!(fold_target("/home/u/.t3/worktrees/", &none, &r), None);
    }

    #[test]
    fn collapse_folds_worktree_aliases_into_the_repo_root() {
        let mut map = HashMap::new();
        map.insert("/w/repo".to_string(), usage(10, 0.1));
        map.insert("/wt/repo-x".to_string(), usage(5, 0.05));
        let r = roots(&["/w/repo"]);
        let a = aliases(&[("/wt/repo-x", "/w/repo")]);

        let out = collapse_usage(&map, &a, &r);
        assert_eq!(out.len(), 1);
        assert_eq!(out.get("/w/repo").unwrap().input_tokens, 15);

        let mut days = HashMap::new();
        days.insert("/w/repo".to_string(), vec![day(1, 10)]);
        days.insert("/wt/repo-x".to_string(), vec![day(1, 5)]);
        let out = collapse_daily(&days, &a, &r);
        assert_eq!(out.get("/w/repo").unwrap()[0].total_input_tokens, 15);
    }

    #[test]
    fn collapse_usage_merges_descendants_into_root() {
        let mut map = HashMap::new();
        map.insert("/w/repo/a".to_string(), usage(10, 0.1));
        map.insert("/w/repo/b".to_string(), usage(20, 0.2));
        map.insert("/w/other".to_string(), usage(5, 0.05));

        let out = collapse_usage(&map, &ProjectLinks::default(), &roots(&["/w/repo"]));
        assert_eq!(out.len(), 2);
        let root = out.get("/w/repo").unwrap();
        assert_eq!(root.input_tokens, 30);
        assert_eq!(root.count, 2);
        assert!((root.cost_usd - 0.3).abs() < 1e-9);
        assert_eq!(out.get("/w/other").unwrap().input_tokens, 5);
    }

    #[test]
    fn collapse_daily_keeps_per_model_breakdown() {
        let model = |input| crate::types::ModelUsage {
            input_tokens: input,
            ..Default::default()
        };
        let mut a = day(1, 10);
        a.models.insert("opus".to_string(), model(10));
        let mut b = day(1, 5);
        b.models.insert("opus".to_string(), model(3));
        b.models.insert("sonnet".to_string(), model(2));
        let mut map = HashMap::new();
        map.insert("/w/repo/a".to_string(), vec![a]);
        map.insert("/w/repo/b".to_string(), vec![b]);

        let out = collapse_daily(&map, &ProjectLinks::default(), &roots(&["/w/repo"]));
        let models = &out.get("/w/repo").unwrap()[0].models;
        assert_eq!(models.get("opus").unwrap().input_tokens, 13);
        assert_eq!(models.get("sonnet").unwrap().input_tokens, 2);
    }

    #[test]
    fn collapse_usage_without_roots_is_identity() {
        let mut map = HashMap::new();
        map.insert("/w/a".to_string(), usage(10, 0.1));
        let out = collapse_usage(&map, &ProjectLinks::default(), &[]);
        assert_eq!(out.len(), 1);
        assert_eq!(out.get("/w/a").unwrap().input_tokens, 10);
    }

    #[test]
    fn collapse_daily_merges_same_date_and_sorts() {
        let mut map = HashMap::new();
        map.insert("/w/repo/a".to_string(), vec![day(1, 10), day(3, 30)]);
        map.insert("/w/repo/b".to_string(), vec![day(1, 5), day(2, 20)]);
        map.insert("/w/other".to_string(), vec![day(1, 1)]);

        let out = collapse_daily(&map, &ProjectLinks::default(), &roots(&["/w/repo"]));
        assert_eq!(out.len(), 2);
        let days: Vec<(u32, u64)> = out
            .get("/w/repo")
            .unwrap()
            .iter()
            .map(|d| (chrono::Datelike::day(&d.date), d.total_input_tokens))
            .collect();
        assert_eq!(days, vec![(1, 15), (2, 20), (3, 30)]);
        assert_eq!(out.get("/w/other").unwrap().len(), 1);
    }
}
