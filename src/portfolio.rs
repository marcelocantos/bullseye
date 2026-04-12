// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use std::path::{Path, PathBuf};

use crate::graph;
use crate::schema::{CrossEdge, TargetsFile};
use crate::store;

/// A single cross-repo edge as it appears in portfolio output — the
/// edge itself plus the source target that owns it. Collected during
/// scanning and surfaced in [`format_portfolio`].
#[derive(Debug, Clone)]
pub struct CrossRepoEdgeRef {
    /// ID of the target in this repo that owns the edge (e.g. `"T2.2"`).
    pub source_target: String,
    /// The cross-repo edge payload (target repo + target/capability + note).
    pub edge: CrossEdge,
}

/// A frontier target within a portfolio summary. Carries the bits
/// `format_portfolio` needs to render with priority boosts: value,
/// cross-enables count, and the target name.
#[derive(Debug, Clone)]
pub struct PortfolioFrontierTarget {
    /// Target ID (e.g. `"T1"`, `"T1.2"`).
    pub id: String,
    /// Target name.
    pub name: String,
    /// Target value (for ordering).
    pub value: f64,
    /// Number of cross-repo enablers attached to the target. A
    /// non-zero count boosts the target's rank in the portfolio view:
    /// finishing it unblocks work in other repos, so it deserves
    /// visibility ahead of targets that only pay out locally.
    pub cross_enables_count: usize,
}

/// A discovered repo with its targets summary.
#[derive(Debug, Clone)]
pub struct RepoSummary {
    /// Repo identifier derived from the path (e.g., "marcelocantos/bullseye").
    pub repo: String,
    /// Filesystem path to the repo root.
    pub path: PathBuf,
    /// Number of active (non-achieved) targets.
    pub active: usize,
    /// Number of frontier (unblocked) targets.
    pub frontier: usize,
    /// Number of achieved targets.
    pub achieved: usize,
    /// Frontier targets, ordered by portfolio priority: targets with
    /// `cross_enables` first (value propagates across repos), then by
    /// plain value desc, then by ID for stable tiebreaks.
    pub frontier_targets: Vec<PortfolioFrontierTarget>,
    /// All `cross_depends` edges from any active target in this repo.
    pub cross_depends: Vec<CrossRepoEdgeRef>,
    /// All `cross_enables` edges from any active target in this repo.
    pub cross_enables: Vec<CrossRepoEdgeRef>,
}

/// A repo that was discovered during a portfolio scan but could not
/// be summarized — the bullseye.yaml was either unreadable, unparsable,
/// or declared a schema version the current bullseye doesn't support.
///
/// Warnings must be surfaced in the portfolio output rather than
/// silently dropping the affected repo, because the user explicitly
/// asked for a portfolio-wide view and a silent skip would hide
/// important information — most critically, a `VersionTooNew` error
/// (which tells the user their bullseye is out of date and should
/// be upgraded). Hiding that would defeat the entire purpose of the
/// schema version check.
#[derive(Debug, Clone)]
pub struct RepoWarning {
    /// Repo identifier derived from the path.
    pub repo: String,
    /// Filesystem path to the repo root.
    pub path: PathBuf,
    /// Classification of the failure. `VersionMismatch` is separated
    /// out so the output can flag it prominently.
    pub kind: RepoWarningKind,
    /// Rendered error string for display.
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoWarningKind {
    /// Targets file declares a `schema_version` newer than this binary
    /// supports. The user should upgrade bullseye.
    VersionMismatch,
    /// Any other load failure (parse error, I/O error).
    LoadError,
}

/// Result of scanning a workspace root for repos with targets.
#[derive(Debug, Clone, Default)]
pub struct PortfolioScan {
    /// Repos whose bullseye.yaml loaded successfully.
    pub repos: Vec<RepoSummary>,
    /// Repos whose bullseye.yaml was found but couldn't be summarized.
    /// Must be surfaced in the output, not silently dropped.
    pub warnings: Vec<RepoWarning>,
}

/// Scan a workspace root for repos containing `bullseye.yaml`.
///
/// Walks up to `max_depth` levels deep under `root`, looking for
/// `bullseye.yaml` files. For each found, attempts to load and
/// summarize. Failures become [`RepoWarning`] entries on the returned
/// [`PortfolioScan`] — critically, this means a repo whose targets
/// file declares a newer `schema_version` than this bullseye supports
/// is visible in the scan output (flagged for upgrade) rather than
/// silently disappearing.
pub fn discover_repos(root: &Path, max_depth: usize) -> PortfolioScan {
    let mut scan = PortfolioScan::default();
    let mut dirs = vec![(root.to_path_buf(), 0usize)];

    while let Some((dir, depth)) = dirs.pop() {
        // Check for bullseye.yaml at this level.
        let targets_path = dir.join("bullseye.yaml");
        if targets_path.is_file() {
            match summarize_repo(&dir, &targets_path) {
                Ok(summary) => scan.repos.push(summary),
                Err(warning) => scan.warnings.push(warning),
            }
            // Don't recurse into repos that have targets — they're leaf repos.
            continue;
        }

        // Recurse if within depth limit.
        if depth < max_depth
            && let Ok(entries) = std::fs::read_dir(&dir)
        {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    // Skip hidden directories and common non-repo dirs.
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if !name_str.starts_with('.')
                        && name_str != "node_modules"
                        && name_str != "target"
                        && name_str != "vendor"
                    {
                        dirs.push((path, depth + 1));
                    }
                }
            }
        }
    }

    // Sort both lists by repo name for stable output.
    scan.repos.sort_by(|a, b| a.repo.cmp(&b.repo));
    scan.warnings.sort_by(|a, b| a.repo.cmp(&b.repo));
    scan
}

/// Derive a repo identifier from a filesystem path.
///
/// Given a path like `/Users/x/work/github.com/org/repo`, extracts `org/repo`.
/// Falls back to the last two path components if the pattern doesn't match.
pub fn repo_name_from_path(path: &Path) -> String {
    let components: Vec<&str> = path
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();

    // Look for a hosting provider component (github.com, bitbucket.com, etc.).
    if let Some(host_idx) = components
        .iter()
        .position(|c| c.contains(".com") || c.contains(".org") || c.contains(".io"))
    {
        // Take everything after the host.
        let after_host = &components[host_idx + 1..];
        if after_host.len() >= 2 {
            return format!("{}/{}", after_host[0], after_host[1]);
        }
        if !after_host.is_empty() {
            return after_host[0].to_string();
        }
    }

    // Fallback: last two components.
    let len = components.len();
    if len >= 2 {
        format!("{}/{}", components[len - 2], components[len - 1])
    } else {
        components.last().unwrap_or(&"unknown").to_string()
    }
}

fn summarize_repo(repo_root: &Path, targets_path: &Path) -> Result<RepoSummary, RepoWarning> {
    let file: TargetsFile = match store::load(targets_path) {
        Ok(f) => f,
        Err(store::LoadError::VersionTooNew {
            found, supported, ..
        }) => {
            return Err(RepoWarning {
                repo: repo_name_from_path(repo_root),
                path: repo_root.to_path_buf(),
                kind: RepoWarningKind::VersionMismatch,
                message: format!(
                    "schema_version {found} > supported {supported} — upgrade bullseye"
                ),
            });
        }
        Err(e) => {
            return Err(RepoWarning {
                repo: repo_name_from_path(repo_root),
                path: repo_root.to_path_buf(),
                kind: RepoWarningKind::LoadError,
                message: e.to_string(),
            });
        }
    };

    let active = file.active();
    let active_count = active.len();
    let achieved_count = file.achieved().len();

    // Collect cross-repo edges across every active target so the
    // portfolio view can surface them. Inactive (achieved) targets
    // are skipped — their cross-refs no longer represent live work.
    let mut cross_depends: Vec<CrossRepoEdgeRef> = Vec::new();
    let mut cross_enables: Vec<CrossRepoEdgeRef> = Vec::new();
    for (id, t) in &active {
        for edge in &t.cross_depends {
            cross_depends.push(CrossRepoEdgeRef {
                source_target: id.to_string(),
                edge: edge.clone(),
            });
        }
        for edge in &t.cross_enables {
            cross_enables.push(CrossRepoEdgeRef {
                source_target: id.to_string(),
                edge: edge.clone(),
            });
        }
    }

    // Only compute frontier if validation passes.
    let errors = graph::validate(&file);
    let frontier_targets = if errors.is_empty() {
        graph::frontier(&file)
    } else {
        Vec::new()
    };

    // Turn frontier entries into PortfolioFrontierTarget and order
    // them: targets with `cross_enables` first (their value
    // propagates across repos and so gets priority in the portfolio
    // view), then by target value desc, then by ID for stability.
    let mut frontier_named: Vec<PortfolioFrontierTarget> = frontier_targets
        .iter()
        .map(|ft| {
            let (value, cross_enables_count) = file
                .targets
                .get(&ft.id)
                .map(|t| (t.value, t.cross_enables.len()))
                .unwrap_or((0.0, 0));
            PortfolioFrontierTarget {
                id: ft.id.clone(),
                name: ft.name.clone(),
                value,
                cross_enables_count,
            }
        })
        .collect();
    frontier_named.sort_by(|a, b| {
        let a_has = a.cross_enables_count > 0;
        let b_has = b.cross_enables_count > 0;
        // Cross-enablers first.
        b_has
            .cmp(&a_has)
            // Then higher value first.
            .then_with(|| {
                b.value
                    .partial_cmp(&a.value)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            // Stable tiebreak on ID.
            .then_with(|| a.id.cmp(&b.id))
    });

    Ok(RepoSummary {
        repo: repo_name_from_path(repo_root),
        path: repo_root.to_path_buf(),
        active: active_count,
        frontier: frontier_named.len(),
        achieved: achieved_count,
        frontier_targets: frontier_named,
        cross_depends,
        cross_enables,
    })
}

/// Format a portfolio summary for agent consumption.
pub fn format_portfolio(scan: &PortfolioScan) -> String {
    let repos = &scan.repos;
    let warnings = &scan.warnings;

    if repos.is_empty() && warnings.is_empty() {
        return "No repos with targets found.\n".to_string();
    }

    let total_active: usize = repos.iter().map(|r| r.active).sum();
    let total_frontier: usize = repos.iter().map(|r| r.frontier).sum();
    let total_achieved: usize = repos.iter().map(|r| r.achieved).sum();
    let repos_with_active: usize = repos.iter().filter(|r| r.active > 0).count();

    let mut out = format!(
        "# Portfolio\n\
         Repos scanned: {}, with active targets: {}\n\
         Total: {} active, {} frontier, {} achieved\n\n",
        repos.len(),
        repos_with_active,
        total_active,
        total_frontier,
        total_achieved,
    );

    // Warnings first — the user must see these, especially the
    // VersionMismatch ones which tell them to upgrade bullseye.
    // Silently dropping such repos would hide the whole point of
    // the schema_version check.
    if !warnings.is_empty() {
        let version_mismatches: Vec<&RepoWarning> = warnings
            .iter()
            .filter(|w| w.kind == RepoWarningKind::VersionMismatch)
            .collect();
        let load_errors: Vec<&RepoWarning> = warnings
            .iter()
            .filter(|w| w.kind == RepoWarningKind::LoadError)
            .collect();

        out.push_str("## ⚠ Warnings\n\n");
        if !version_mismatches.is_empty() {
            out.push_str("**Schema version mismatch — upgrade bullseye to read these repos:**\n\n");
            for w in &version_mismatches {
                out.push_str(&format!("- **{}** — {}\n", w.repo, w.message));
            }
            out.push('\n');
        }
        if !load_errors.is_empty() {
            out.push_str("**Targets file could not be loaded:**\n\n");
            for w in &load_errors {
                out.push_str(&format!("- **{}** — {}\n", w.repo, w.message));
            }
            out.push('\n');
        }
    }

    // Repos with frontier targets first (sorted by frontier count desc).
    let mut with_frontier: Vec<&RepoSummary> = repos.iter().filter(|r| r.frontier > 0).collect();
    with_frontier.sort_by(|a, b| b.frontier.cmp(&a.frontier));

    if !with_frontier.is_empty() {
        out.push_str("## Ready for work\n\n");
        for repo in &with_frontier {
            out.push_str(&format!(
                "**{}** — {} active, {} frontier\n",
                repo.repo, repo.active, repo.frontier,
            ));
            for ft in &repo.frontier_targets {
                // Annotate cross-enabler targets so the caller can see
                // at a glance which frontier entries propagate value to
                // other repos. These targets also sort to the top of
                // the per-repo frontier list.
                if ft.cross_enables_count > 0 {
                    out.push_str(&format!(
                        "  ★ 🎯{id} {name}  [enables {n} cross-repo]\n",
                        id = ft.id,
                        name = ft.name,
                        n = ft.cross_enables_count,
                    ));
                } else {
                    out.push_str(&format!("  🎯{id} {name}\n", id = ft.id, name = ft.name));
                }
            }
            out.push('\n');
        }
    }

    // Cross-repo edges section — surfaces every `cross_depends` and
    // `cross_enables` edge across the portfolio so the user sees the
    // coupling that bullseye deliberately does not enforce via the
    // frontier graph. Dangling refs are fine; they just show up as-is.
    let repos_with_cross: Vec<&RepoSummary> = repos
        .iter()
        .filter(|r| !r.cross_depends.is_empty() || !r.cross_enables.is_empty())
        .collect();
    if !repos_with_cross.is_empty() {
        out.push_str("## Cross-repo edges\n\n");
        for repo in &repos_with_cross {
            out.push_str(&format!("**{}**\n", repo.repo));
            for edge_ref in &repo.cross_depends {
                out.push_str(&format!(
                    "  🎯{src} depends on {ref_} @ {repo}",
                    src = edge_ref.source_target,
                    ref_ = edge_ref.edge.reference(),
                    repo = edge_ref.edge.repo,
                ));
                if let Some(ref note) = edge_ref.edge.note {
                    out.push_str(&format!(" — {note}"));
                }
                out.push('\n');
            }
            for edge_ref in &repo.cross_enables {
                out.push_str(&format!(
                    "  🎯{src} enables {ref_} @ {repo}",
                    src = edge_ref.source_target,
                    ref_ = edge_ref.edge.reference(),
                    repo = edge_ref.edge.repo,
                ));
                if let Some(ref note) = edge_ref.edge.note {
                    out.push_str(&format!(" — {note}"));
                }
                out.push('\n');
            }
            out.push('\n');
        }
    }

    // Repos with active but no frontier (all blocked).
    let blocked: Vec<&RepoSummary> = repos
        .iter()
        .filter(|r| r.active > 0 && r.frontier == 0)
        .collect();
    if !blocked.is_empty() {
        out.push_str("## Blocked (active but no frontier)\n\n");
        for repo in &blocked {
            out.push_str(&format!(
                "**{}** — {} active, all blocked\n",
                repo.repo, repo.active,
            ));
        }
        out.push('\n');
    }

    // Repos with only achieved targets.
    let done: Vec<&RepoSummary> = repos
        .iter()
        .filter(|r| r.active == 0 && r.achieved > 0)
        .collect();
    if !done.is_empty() {
        out.push_str(&format!(
            "## Complete ({} repos, all targets achieved)\n\n",
            done.len()
        ));
        for repo in &done {
            out.push_str(&format!("- {} ({} achieved)\n", repo.repo, repo.achieved));
        }
        out.push('\n');
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal [`PortfolioFrontierTarget`] without
    /// cross-repo metadata — used by tests that predate the
    /// cross-enabler priority boost.
    fn pft(id: &str, name: &str, value: f64) -> PortfolioFrontierTarget {
        PortfolioFrontierTarget {
            id: id.to_string(),
            name: name.to_string(),
            value,
            cross_enables_count: 0,
        }
    }

    #[test]
    fn repo_name_github() {
        let p = Path::new("/Users/marcelo/work/github.com/marcelocantos/bullseye");
        assert_eq!(repo_name_from_path(p), "marcelocantos/bullseye");
    }

    #[test]
    fn repo_name_bitbucket() {
        let p = Path::new("/home/user/work/bitbucket.org/team/project");
        assert_eq!(repo_name_from_path(p), "team/project");
    }

    #[test]
    fn repo_name_fallback() {
        let p = Path::new("/some/random/path");
        assert_eq!(repo_name_from_path(p), "random/path");
    }

    #[test]
    fn format_empty_portfolio() {
        let scan = PortfolioScan::default();
        let out = format_portfolio(&scan);
        assert_eq!(out, "No repos with targets found.\n");
    }

    #[test]
    fn format_portfolio_with_repos() {
        let scan = PortfolioScan {
            repos: vec![
                RepoSummary {
                    repo: "org/repo-a".to_string(),
                    path: PathBuf::from("/work/org/repo-a"),
                    active: 3,
                    frontier: 2,
                    achieved: 1,
                    frontier_targets: vec![
                        pft("T1", "First target", 5.0),
                        pft("T2", "Second target", 3.0),
                    ],
                    cross_depends: Vec::new(),
                    cross_enables: Vec::new(),
                },
                RepoSummary {
                    repo: "org/repo-b".to_string(),
                    path: PathBuf::from("/work/org/repo-b"),
                    active: 0,
                    frontier: 0,
                    achieved: 5,
                    frontier_targets: vec![],
                    cross_depends: Vec::new(),
                    cross_enables: Vec::new(),
                },
            ],
            warnings: Vec::new(),
        };

        let out = format_portfolio(&scan);
        assert!(out.contains("Repos scanned: 2"));
        assert!(out.contains("with active targets: 1"));
        assert!(out.contains("3 active, 2 frontier, 6 achieved"));
        assert!(out.contains("**org/repo-a**"));
        assert!(out.contains("🎯T1 First target"));
        assert!(out.contains("## Complete"));
        assert!(out.contains("org/repo-b (5 achieved)"));
        // No warnings → no warnings section.
        assert!(!out.contains("## ⚠ Warnings"));
    }

    #[test]
    fn format_portfolio_surfaces_version_mismatch_warning() {
        // A repo whose bullseye.yaml declares a schema_version this
        // binary doesn't support must appear prominently in the
        // output — otherwise the user would silently lose the
        // upgrade signal.
        let scan = PortfolioScan {
            repos: vec![RepoSummary {
                repo: "org/ok-repo".to_string(),
                path: PathBuf::from("/work/org/ok-repo"),
                active: 1,
                frontier: 1,
                achieved: 0,
                frontier_targets: vec![pft("T1", "A target", 3.0)],
                cross_depends: Vec::new(),
                cross_enables: Vec::new(),
            }],
            warnings: vec![RepoWarning {
                repo: "org/future-repo".to_string(),
                path: PathBuf::from("/work/org/future-repo"),
                kind: RepoWarningKind::VersionMismatch,
                message: "schema_version 2 > supported 1 — upgrade bullseye".to_string(),
            }],
        };

        let out = format_portfolio(&scan);
        assert!(out.contains("## ⚠ Warnings"));
        assert!(out.contains("Schema version mismatch"));
        assert!(out.contains("org/future-repo"));
        assert!(out.contains("schema_version 2 > supported 1"));
        assert!(out.contains("upgrade bullseye"));
        // The good repo is still present.
        assert!(out.contains("org/ok-repo"));
    }

    #[test]
    fn format_portfolio_surfaces_load_error_warning() {
        // A repo whose bullseye.yaml is unparsable should also be
        // surfaced, but under a different heading so the user can
        // distinguish "my bullseye is stale" from "my YAML is broken".
        let scan = PortfolioScan {
            repos: Vec::new(),
            warnings: vec![RepoWarning {
                repo: "org/broken-repo".to_string(),
                path: PathBuf::from("/work/org/broken-repo"),
                kind: RepoWarningKind::LoadError,
                message: "failed to parse /work/.../bullseye.yaml: bad indent at line 7"
                    .to_string(),
            }],
        };

        let out = format_portfolio(&scan);
        assert!(out.contains("## ⚠ Warnings"));
        assert!(out.contains("Targets file could not be loaded"));
        assert!(out.contains("org/broken-repo"));
        assert!(out.contains("bad indent at line 7"));
        // No version-mismatch section when there aren't any.
        assert!(!out.contains("Schema version mismatch"));
    }

    #[test]
    fn format_portfolio_surfaces_cross_repo_edges() {
        // A repo with both cross_depends and cross_enables edges must
        // see them rendered in a dedicated section with source target,
        // direction, referenced repo, and (when present) a note.
        let edge_dep = CrossRepoEdgeRef {
            source_target: "T1".to_string(),
            edge: CrossEdge {
                repo: "marcelocantos/jevon".to_string(),
                target: None,
                capability: Some("Manager API".to_string()),
                note: Some("needed for summarizer lifecycle".to_string()),
            },
        };
        let edge_enable = CrossRepoEdgeRef {
            source_target: "T2".to_string(),
            edge: CrossEdge {
                repo: "marcelocantos/targets".to_string(),
                target: Some("T1.4".to_string()),
                capability: None,
                note: None,
            },
        };

        let scan = PortfolioScan {
            repos: vec![RepoSummary {
                repo: "org/linker".to_string(),
                path: PathBuf::from("/work/org/linker"),
                active: 2,
                frontier: 2,
                achieved: 0,
                // Already in portfolio priority order: cross-enabler
                // first, then by value. `summarize_repo` populates this
                // list in this same order; tests that build summaries
                // by hand must respect the invariant.
                frontier_targets: vec![
                    PortfolioFrontierTarget {
                        id: "T2".to_string(),
                        name: "Cross-repo enabler".to_string(),
                        value: 3.0,
                        cross_enables_count: 1,
                    },
                    PortfolioFrontierTarget {
                        id: "T1".to_string(),
                        name: "Plain work".to_string(),
                        value: 5.0,
                        cross_enables_count: 0,
                    },
                ],
                cross_depends: vec![edge_dep],
                cross_enables: vec![edge_enable],
            }],
            warnings: Vec::new(),
        };

        let out = format_portfolio(&scan);

        // Dedicated cross-repo section appears.
        assert!(out.contains("## Cross-repo edges"));
        // cross_depends line: source → dep (capability form) @ repo — note.
        assert!(
            out.contains("🎯T1 depends on Manager API @ marcelocantos/jevon"),
            "expected cross_depends line; got:\n{out}"
        );
        assert!(out.contains("needed for summarizer lifecycle"));
        // cross_enables line: source → dep (target form) @ repo.
        assert!(
            out.contains("🎯T2 enables 🎯T1.4 @ marcelocantos/targets"),
            "expected cross_enables line; got:\n{out}"
        );

        // Frontier rendering: cross-enabler is marked and boosted above
        // the plain higher-value target.
        let ready = out
            .split("## Ready for work")
            .nth(1)
            .expect("ready section exists");
        let end = ready.find("\n## ").unwrap_or(ready.len());
        let ready_text = &ready[..end];
        let t2_pos = ready_text.find("🎯T2").expect("T2 in ready section");
        let t1_pos = ready_text.find("🎯T1").expect("T1 in ready section");
        assert!(
            t2_pos < t1_pos,
            "T2 (cross-enabler, v=3) should rank above T1 (v=5) in portfolio; got:\n{ready_text}"
        );
        assert!(
            ready_text.contains("★ 🎯T2"),
            "cross-enabler should be marked with ★; got:\n{ready_text}"
        );
        assert!(
            ready_text.contains("[enables 1 cross-repo]"),
            "cross-enabler should be annotated; got:\n{ready_text}"
        );
    }

    #[test]
    fn format_portfolio_no_cross_section_when_empty() {
        // A portfolio with no cross-repo edges must not emit an empty
        // "Cross-repo edges" section.
        let scan = PortfolioScan {
            repos: vec![RepoSummary {
                repo: "org/plain".to_string(),
                path: PathBuf::from("/work/org/plain"),
                active: 1,
                frontier: 1,
                achieved: 0,
                frontier_targets: vec![pft("T1", "Plain", 5.0)],
                cross_depends: Vec::new(),
                cross_enables: Vec::new(),
            }],
            warnings: Vec::new(),
        };

        let out = format_portfolio(&scan);
        assert!(!out.contains("## Cross-repo edges"));
        // And plain frontier targets don't get the ★ marker.
        assert!(!out.contains("★"));
    }
}
