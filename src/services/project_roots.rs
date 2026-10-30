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

/// Project keys mapped to the path they fold by: a worktree key maps to the
/// same folder in its main repository. The paths are only compared against
/// roots, never shown.
pub type ProjectAliases = HashMap<String, String>;

/// Folder T3 Code keeps its worktrees in, as `<this>/<repo name>/<branch>`.
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
pub fn fold_target<'a>(
    key: &str,
    aliases: &ProjectAliases,
    roots: &'a [String],
) -> Option<&'a str> {
    match aliases.get(key) {
        Some(repo_path) => root_for(repo_path, roots).or_else(|| root_for(key, roots)),
        None => root_for(key, roots).or_else(|| root_for_t3_worktree(key, roots)),
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

/// The only root named like the repository of the T3 worktree `key` lies in,
/// ignoring case and `_` vs `-` (T3 may rename `foo_bar` to `foo-bar`).
fn root_for_t3_worktree<'a>(key: &str, roots: &'a [String]) -> Option<&'a str> {
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
    matches.next().is_none().then_some(root.as_str())
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
pub fn display_key<'a>(key: &'a str, aliases: &ProjectAliases, roots: &'a [String]) -> &'a str {
    fold_target(key, aliases, roots).unwrap_or(key)
}

/// Re-key a rolled-up project map onto roots, merging folded projects.
pub fn collapse_usage(
    map: &HashMap<String, ProjectUsage>,
    aliases: &ProjectAliases,
    roots: &[String],
) -> HashMap<String, ProjectUsage> {
    if roots.is_empty() {
        return map.clone();
    }
    let mut out: HashMap<String, ProjectUsage> = HashMap::new();
    for (key, usage) in map {
        out.entry(display_key(key, aliases, roots).to_string())
            .or_default()
            .merge(usage);
    }
    out
}

/// Re-key per-project daily summaries onto roots, merging same-date days.
pub fn collapse_daily(
    map: &HashMap<String, Vec<DailySummary>>,
    aliases: &ProjectAliases,
    roots: &[String],
) -> HashMap<String, Vec<DailySummary>> {
    if roots.is_empty() {
        return map.clone();
    }
    let mut grouped: HashMap<String, Vec<DailySummary>> = HashMap::new();
    for (key, days) in map {
        grouped
            .entry(display_key(key, aliases, roots).to_string())
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

    fn aliases(pairs: &[(&str, &str)]) -> ProjectAliases {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
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
            fold_target("/w/repo/a", &HashMap::new(), &r),
            Some("/w/repo")
        );
        assert_eq!(fold_target("/w/other", &HashMap::new(), &r), None);
    }

    #[test]
    fn fold_target_t3_worktree_name_ignores_case_and_underscore_vs_hyphen() {
        let r = roots(&[r"G:\Scripts\rngdle_solver", r"G:\Scripts\annex"]);
        let none = HashMap::new();
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
            fold_target("/home/u/.t3/worktrees/annex/x", &HashMap::new(), &r),
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
            fold_target("/home/u/.t3/worktrees/annex/x", &HashMap::new(), &r),
            Some("/home/u/.t3")
        );
    }

    #[test]
    fn fold_target_ignores_paths_outside_t3_worktrees() {
        let r = roots(&["/w/annex"]);
        let none = HashMap::new();
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

        let out = collapse_usage(&map, &HashMap::new(), &roots(&["/w/repo"]));
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

        let out = collapse_daily(&map, &HashMap::new(), &roots(&["/w/repo"]));
        let models = &out.get("/w/repo").unwrap()[0].models;
        assert_eq!(models.get("opus").unwrap().input_tokens, 13);
        assert_eq!(models.get("sonnet").unwrap().input_tokens, 2);
    }

    #[test]
    fn collapse_usage_without_roots_is_identity() {
        let mut map = HashMap::new();
        map.insert("/w/a".to_string(), usage(10, 0.1));
        let out = collapse_usage(&map, &HashMap::new(), &[]);
        assert_eq!(out.len(), 1);
        assert_eq!(out.get("/w/a").unwrap().input_tokens, 10);
    }

    #[test]
    fn collapse_daily_merges_same_date_and_sorts() {
        let mut map = HashMap::new();
        map.insert("/w/repo/a".to_string(), vec![day(1, 10), day(3, 30)]);
        map.insert("/w/repo/b".to_string(), vec![day(1, 5), day(2, 20)]);
        map.insert("/w/other".to_string(), vec![day(1, 1)]);

        let out = collapse_daily(&map, &HashMap::new(), &roots(&["/w/repo"]));
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
