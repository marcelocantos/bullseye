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

/// Fraction of downstream target value that propagates back through
/// a `cross_enables` edge. Chosen so a single enabler edge to a
/// high-value (v=13) downstream target roughly doubles the WSJF
/// contribution: `1.0 + 0.25 * 13 = 4.25`. Low enough to avoid
/// overwhelming local value; high enough to noticeably promote
/// cross-repo enablers above equally-valued local-only targets.
const ENABLER_FRACTION: f64 = 0.25;

/// A frontier target within a portfolio summary. Carries the bits
/// `format_portfolio` needs to render with priority boosts and WSJF
/// reasoning.
#[derive(Debug, Clone)]
pub struct PortfolioFrontierTarget {
    /// Target ID (e.g. `"T1"`, `"T1.2"`).
    pub id: String,
    /// Target name.
    pub name: String,
    /// Target value (for ordering).
    pub value: f64,
    /// Target cost (for WSJF computation).
    pub cost: f64,
    /// Number of cross-repo enablers attached to the target. A
    /// non-zero count boosts the target's rank in the portfolio view:
    /// finishing it unblocks work in other repos, so it deserves
    /// visibility ahead of targets that only pay out locally.
    pub cross_enables_count: usize,
    /// Momentum multiplier applied to this target's WSJF contribution.
    /// Defaults to 1.0 when no momentum is supplied.
    pub momentum: f64,
    /// Computed enabler boost: `1.0 + ENABLER_FRACTION * sum(downstream_values)`.
    /// For targets without `cross_enables`, this is 1.0.
    pub enabler_boost: f64,
    /// Per-target WSJF contribution: `(value / cost) * momentum * enabler_boost`.
    pub wsjf: f64,
}

/// A discovered repo with its targets summary and aggregate WSJF score.
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
    /// Aggregate WSJF score: `sum(wsjf_i) / frontier_size`.
    /// The divisor encodes the bias that repos with many parallelisable
    /// frontier targets need less per-unit human attention. Zero when
    /// the frontier is empty.
    pub wsjf_score: f64,
    /// Frontier targets, ordered by per-target WSJF desc, then by ID
    /// for stable tiebreaks.
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

/// Per-target momentum multiplier entry. Mirrors the shape from
/// `tools::MomentumEntry` but decoupled so portfolio doesn't depend
/// on the MCP tool layer.
#[derive(Debug, Clone)]
pub struct Momentum {
    pub id: String,
    pub multiplier: f64,
}

/// Scan a workspace root for repos containing `bullseye.yaml` and
/// compute WSJF-ranked portfolio output.
///
/// Two-pass architecture:
/// 1. **Discover** — walk the tree, load each repo's targets, collect
///    raw frontier data and cross-repo edges.
/// 2. **Score** — resolve `cross_enables` edges against the scanned
///    portfolio to compute weighted enabler boosts, apply momentum,
///    compute per-target and per-repo WSJF.
///
/// The two passes are necessary because enabler boost computation
/// requires the *downstream* target's value, which lives in a
/// different repo that may not have been scanned yet during pass 1.
pub fn discover_repos(root: &Path, max_depth: usize, momentum: &[Momentum]) -> PortfolioScan {
    let mut scan = PortfolioScan::default();
    let mut dirs = vec![(root.to_path_buf(), 0usize)];

    // Build momentum lookup: target_id → multiplier. Last entry wins
    // for duplicates, matching bullseye_summary behaviour.
    let momentum_map: std::collections::HashMap<&str, f64> = momentum
        .iter()
        .map(|m| (m.id.as_str(), m.multiplier))
        .collect();

    // Pass 1: discover and load repos.
    // We collect raw repo data alongside the loaded TargetsFile so
    // pass 2 can resolve cross-repo enabler values.
    struct RawRepo {
        summary: RepoSummary,
    }
    let mut raw_repos: Vec<RawRepo> = Vec::new();

    while let Some((dir, depth)) = dirs.pop() {
        let targets_path = dir.join("bullseye.yaml");
        if targets_path.is_file() {
            match summarize_repo_raw(&dir, &targets_path, &momentum_map) {
                Ok(summary) => raw_repos.push(RawRepo { summary }),
                Err(warning) => scan.warnings.push(warning),
            }
            continue;
        }

        if depth < max_depth
            && let Ok(entries) = std::fs::read_dir(&dir)
        {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
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

    // Pass 2: resolve enabler boosts and compute final WSJF scores.
    //
    // Build an owned lookup: (repo_name, target_id) → value, so we
    // can resolve cross_enables edges to downstream target values.
    // Owned keys avoid borrow-checker conflicts with the mutable
    // pass below.
    let target_value_lookup: std::collections::HashMap<(String, String), f64> = raw_repos
        .iter()
        .flat_map(|r| {
            r.summary
                .frontier_targets
                .iter()
                .map(move |ft| ((r.summary.repo.clone(), ft.id.clone()), ft.value))
        })
        .collect();

    for raw in &mut raw_repos {
        // Build a map from source_target → list of cross_enables edges
        // so we can efficiently look up edges per frontier target.
        let mut enables_map: std::collections::HashMap<&str, Vec<&CrossRepoEdgeRef>> =
            std::collections::HashMap::new();
        for edge_ref in &raw.summary.cross_enables {
            enables_map
                .entry(edge_ref.source_target.as_str())
                .or_default()
                .push(edge_ref);
        }

        for ft in &mut raw.summary.frontier_targets {
            let mut downstream_value_sum = 0.0f64;
            if let Some(edges) = enables_map.get(ft.id.as_str()) {
                for edge_ref in edges {
                    // Try to resolve the downstream target's value from
                    // the portfolio scan. If the edge references a
                    // specific target, look it up. Otherwise (capability
                    // reference or unresolvable), use a default boost
                    // equivalent to enabling a value-5 target.
                    let resolved_value = edge_ref
                        .edge
                        .target
                        .as_ref()
                        .and_then(|tid| {
                            let key = (edge_ref.edge.repo.clone(), tid.clone());
                            target_value_lookup.get(&key).copied()
                        })
                        .unwrap_or(5.0);
                    downstream_value_sum += resolved_value;
                }
            }

            ft.enabler_boost = 1.0 + ENABLER_FRACTION * downstream_value_sum;
            ft.wsjf = if ft.cost > 0.0 {
                (ft.value / ft.cost) * ft.momentum * ft.enabler_boost
            } else {
                // cost=0 means "not set at repo scope" (portfolio metadata
                // is optional since 🎯T11). These targets score 0 in WSJF
                // and won't rank in cross-repo prioritisation.
                0.0
            };
        }

        // Re-sort frontier targets by WSJF desc, then ID for stability.
        raw.summary.frontier_targets.sort_by(|a, b| {
            b.wsjf
                .partial_cmp(&a.wsjf)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.id.cmp(&b.id))
        });

        // Compute aggregate repo WSJF score.
        let frontier_size = raw.summary.frontier_targets.len();
        if frontier_size > 0 {
            let wsjf_sum: f64 = raw.summary.frontier_targets.iter().map(|ft| ft.wsjf).sum();
            raw.summary.wsjf_score = wsjf_sum / frontier_size as f64;
        }
    }

    scan.repos = raw_repos.into_iter().map(|r| r.summary).collect();

    // Sort repos by WSJF score desc (primary), then by name for stability.
    scan.repos.sort_by(|a, b| {
        b.wsjf_score
            .partial_cmp(&a.wsjf_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.repo.cmp(&b.repo))
    });
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

/// First-pass repo summarization: loads targets, collects frontier and
/// cross-repo edges, applies momentum, but leaves `enabler_boost` and
/// `wsjf` at defaults (1.0 and 0.0). Pass 2 in `discover_repos`
/// resolves these once the full portfolio is available.
fn summarize_repo_raw(
    repo_root: &Path,
    targets_path: &Path,
    momentum_map: &std::collections::HashMap<&str, f64>,
) -> Result<RepoSummary, RepoWarning> {
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

    let errors = graph::validate(&file);
    let frontier_targets = if errors.is_empty() {
        graph::frontier(&file)
    } else {
        Vec::new()
    };

    // Build frontier entries with momentum applied but enabler_boost
    // and wsjf deferred to pass 2.
    let frontier_named: Vec<PortfolioFrontierTarget> = frontier_targets
        .iter()
        .map(|ft| {
            let (value, cost, cross_enables_count) = file
                .targets
                .get(&ft.id)
                .map(|t| (t.value, t.cost, t.cross_enables.len()))
                .unwrap_or((0.0, 1.0, 0));
            let mom = momentum_map.get(ft.id.as_str()).copied().unwrap_or(1.0);
            PortfolioFrontierTarget {
                id: ft.id.clone(),
                name: ft.name.clone(),
                value,
                cost,
                cross_enables_count,
                momentum: mom,
                enabler_boost: 1.0, // pass 2
                wsjf: 0.0,          // pass 2
            }
        })
        .collect();

    Ok(RepoSummary {
        repo: repo_name_from_path(repo_root),
        path: repo_root.to_path_buf(),
        active: active_count,
        frontier: frontier_named.len(),
        achieved: achieved_count,
        wsjf_score: 0.0, // pass 2
        frontier_targets: frontier_named,
        cross_depends,
        cross_enables,
    })
}

/// Format a portfolio summary for agent consumption.
///
/// Repos are ranked by aggregate WSJF score (already sorted by
/// `discover_repos`). Each frontier target includes a WSJF breakdown
/// showing the contributing factors.
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
         Total: {} active, {} frontier, {} achieved\n\
         Ranking: WSJF = sum(value/cost × momentum × enabler_boost) / frontier_size\n\n",
        repos.len(),
        repos_with_active,
        total_active,
        total_frontier,
        total_achieved,
    );

    // Warnings first.
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

    // Repos with frontier targets, ranked by WSJF score desc.
    let with_frontier: Vec<&RepoSummary> = repos.iter().filter(|r| r.frontier > 0).collect();

    if !with_frontier.is_empty() {
        out.push_str("## Ready for work (ranked by WSJF)\n\n");
        for (rank, repo) in with_frontier.iter().enumerate() {
            out.push_str(&format!(
                "**#{rank} {repo_name}** — WSJF {score:.2}, {active} active, {frontier} frontier\n",
                rank = rank + 1,
                repo_name = repo.repo,
                score = repo.wsjf_score,
                active = repo.active,
                frontier = repo.frontier,
            ));

            // Per-repo reasoning: top contributing targets.
            for ft in &repo.frontier_targets {
                let mut annotations = Vec::new();
                if ft.cross_enables_count > 0 {
                    annotations.push(format!("enables {} cross-repo", ft.cross_enables_count));
                }
                if (ft.enabler_boost - 1.0).abs() > f64::EPSILON {
                    annotations.push(format!("boost {:.2}×", ft.enabler_boost));
                }
                if (ft.momentum - 1.0).abs() > f64::EPSILON {
                    annotations.push(format!("momentum {:.2}×", ft.momentum));
                }

                let prefix = if ft.cross_enables_count > 0 {
                    "★ "
                } else {
                    ""
                };
                let annotation_str = if annotations.is_empty() {
                    String::new()
                } else {
                    format!("  [{}]", annotations.join(", "))
                };
                out.push_str(&format!(
                    "  {prefix}🎯{id} {name}  (wsjf {wsjf:.2}: v={value}/c={cost}{annotation})\n",
                    id = ft.id,
                    name = ft.name,
                    wsjf = ft.wsjf,
                    value = ft.value,
                    cost = ft.cost,
                    annotation = annotation_str,
                ));
            }
            out.push('\n');
        }
    }

    // Cross-repo edges section.
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

    /// Build a minimal [`PortfolioFrontierTarget`] with default
    /// momentum (1.0), enabler_boost (1.0), and cost (1.0).
    /// WSJF is computed as value/cost since momentum and boost are 1.0.
    fn pft(id: &str, name: &str, value: f64) -> PortfolioFrontierTarget {
        PortfolioFrontierTarget {
            id: id.to_string(),
            name: name.to_string(),
            value,
            cost: 1.0,
            cross_enables_count: 0,
            momentum: 1.0,
            enabler_boost: 1.0,
            wsjf: value, // value / cost(1.0) * momentum(1.0) * boost(1.0)
        }
    }

    /// Build a [`PortfolioFrontierTarget`] with explicit cost and WSJF.
    fn pft_full(
        id: &str,
        name: &str,
        value: f64,
        cost: f64,
        momentum: f64,
        enabler_boost: f64,
        cross_enables_count: usize,
    ) -> PortfolioFrontierTarget {
        let wsjf = if cost > 0.0 {
            (value / cost) * momentum * enabler_boost
        } else {
            0.0
        };
        PortfolioFrontierTarget {
            id: id.to_string(),
            name: name.to_string(),
            value,
            cost,
            cross_enables_count,
            momentum,
            enabler_boost,
            wsjf,
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
                    wsjf_score: 4.0, // (5+3)/2
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
                    wsjf_score: 0.0,
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
        assert!(out.contains("org/repo-a"));
        assert!(out.contains("🎯T1 First target"));
        assert!(out.contains("WSJF 4.00"));
        assert!(out.contains("## Complete"));
        assert!(out.contains("org/repo-b (5 achieved)"));
        assert!(!out.contains("## ⚠ Warnings"));
    }

    #[test]
    fn format_portfolio_surfaces_version_mismatch_warning() {
        let scan = PortfolioScan {
            repos: vec![RepoSummary {
                repo: "org/ok-repo".to_string(),
                path: PathBuf::from("/work/org/ok-repo"),
                active: 1,
                frontier: 1,
                achieved: 0,
                wsjf_score: 3.0,
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

        // T2 has enabler_boost from cross_enables (1 + 0.25*5 = 2.25),
        // giving it wsjf = 3/1 * 1.0 * 2.25 = 6.75, which beats
        // T1's wsjf = 5/1 * 1.0 * 1.0 = 5.0.
        let scan = PortfolioScan {
            repos: vec![RepoSummary {
                repo: "org/linker".to_string(),
                path: PathBuf::from("/work/org/linker"),
                active: 2,
                frontier: 2,
                achieved: 0,
                wsjf_score: 5.875, // (6.75 + 5.0) / 2
                frontier_targets: vec![
                    pft_full("T2", "Cross-repo enabler", 3.0, 1.0, 1.0, 2.25, 1),
                    pft_full("T1", "Plain work", 5.0, 1.0, 1.0, 1.0, 0),
                ],
                cross_depends: vec![edge_dep],
                cross_enables: vec![edge_enable],
            }],
            warnings: Vec::new(),
        };

        let out = format_portfolio(&scan);

        // Dedicated cross-repo section appears.
        assert!(out.contains("## Cross-repo edges"));
        assert!(
            out.contains("🎯T1 depends on Manager API @ marcelocantos/jevon"),
            "expected cross_depends line; got:\n{out}"
        );
        assert!(out.contains("needed for summarizer lifecycle"));
        assert!(
            out.contains("🎯T2 enables 🎯T1.4 @ marcelocantos/targets"),
            "expected cross_enables line; got:\n{out}"
        );

        // Frontier rendering: cross-enabler ranks above plain target
        // due to higher WSJF from enabler_boost.
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
            "T2 (cross-enabler, wsjf=6.75) should rank above T1 (wsjf=5.0); got:\n{ready_text}"
        );
        assert!(
            ready_text.contains("★ 🎯T2"),
            "cross-enabler should be marked with ★; got:\n{ready_text}"
        );
        assert!(
            ready_text.contains("enables 1 cross-repo"),
            "cross-enabler should be annotated; got:\n{ready_text}"
        );
        // WSJF breakdown is shown.
        assert!(
            ready_text.contains("wsjf 6.75"),
            "per-target WSJF should appear; got:\n{ready_text}"
        );
    }

    #[test]
    fn format_portfolio_no_cross_section_when_empty() {
        let scan = PortfolioScan {
            repos: vec![RepoSummary {
                repo: "org/plain".to_string(),
                path: PathBuf::from("/work/org/plain"),
                active: 1,
                frontier: 1,
                achieved: 0,
                wsjf_score: 5.0,
                frontier_targets: vec![pft("T1", "Plain", 5.0)],
                cross_depends: Vec::new(),
                cross_enables: Vec::new(),
            }],
            warnings: Vec::new(),
        };

        let out = format_portfolio(&scan);
        assert!(!out.contains("## Cross-repo edges"));
        assert!(!out.contains("★"));
    }

    #[test]
    fn wsjf_ranking_with_momentum() {
        // Repo A has a lower-value target but with 2× momentum,
        // giving it a higher WSJF than repo B's higher-value target.
        let scan = PortfolioScan {
            repos: vec![
                RepoSummary {
                    repo: "org/repo-a".to_string(),
                    path: PathBuf::from("/work/org/repo-a"),
                    active: 1,
                    frontier: 1,
                    achieved: 0,
                    wsjf_score: 6.0, // 3/1 * 2.0 * 1.0 = 6.0
                    frontier_targets: vec![pft_full("T1", "Boosted", 3.0, 1.0, 2.0, 1.0, 0)],
                    cross_depends: Vec::new(),
                    cross_enables: Vec::new(),
                },
                RepoSummary {
                    repo: "org/repo-b".to_string(),
                    path: PathBuf::from("/work/org/repo-b"),
                    active: 1,
                    frontier: 1,
                    achieved: 0,
                    wsjf_score: 5.0, // 5/1 * 1.0 * 1.0 = 5.0
                    frontier_targets: vec![pft("T1", "Plain high-value", 5.0)],
                    cross_depends: Vec::new(),
                    cross_enables: Vec::new(),
                },
            ],
            warnings: Vec::new(),
        };

        let out = format_portfolio(&scan);
        let a_pos = out.find("org/repo-a").expect("repo-a in output");
        let b_pos = out.find("org/repo-b").expect("repo-b in output");
        assert!(
            a_pos < b_pos,
            "repo-a (WSJF 6.0) should rank above repo-b (WSJF 5.0); got:\n{out}"
        );
        assert!(
            out.contains("momentum 2.00×"),
            "momentum annotation should appear; got:\n{out}"
        );
    }

    #[test]
    fn wsjf_formula_components() {
        // Verify the per-target WSJF formula:
        // wsjf = (value / cost) * momentum * enabler_boost
        let ft = pft_full("T1", "Test", 8.0, 2.0, 1.5, 2.0, 1);
        // Expected: (8/2) * 1.5 * 2.0 = 12.0
        assert!(
            (ft.wsjf - 12.0).abs() < f64::EPSILON,
            "expected wsjf=12.0, got {}",
            ft.wsjf
        );
    }

    #[test]
    fn wsjf_aggregate_divides_by_frontier_size() {
        // Per-repo WSJF is sum(wsjf_i) / frontier_size, encoding
        // that parallelisable repos need less per-unit attention.
        let scan = PortfolioScan {
            repos: vec![
                RepoSummary {
                    repo: "org/focused".to_string(),
                    path: PathBuf::from("/work/org/focused"),
                    active: 1,
                    frontier: 1,
                    achieved: 0,
                    wsjf_score: 8.0, // single target: 8/1 = 8.0
                    frontier_targets: vec![pft("T1", "Solo", 8.0)],
                    cross_depends: Vec::new(),
                    cross_enables: Vec::new(),
                },
                RepoSummary {
                    repo: "org/spread".to_string(),
                    path: PathBuf::from("/work/org/spread"),
                    active: 4,
                    frontier: 4,
                    achieved: 0,
                    wsjf_score: 5.0, // 4 targets × 5.0 / 4 = 5.0
                    frontier_targets: vec![
                        pft("T1", "A", 5.0),
                        pft("T2", "B", 5.0),
                        pft("T3", "C", 5.0),
                        pft("T4", "D", 5.0),
                    ],
                    cross_depends: Vec::new(),
                    cross_enables: Vec::new(),
                },
            ],
            warnings: Vec::new(),
        };

        let out = format_portfolio(&scan);
        let focused_pos = out.find("org/focused").expect("focused in output");
        let spread_pos = out.find("org/spread").expect("spread in output");
        assert!(
            focused_pos < spread_pos,
            "focused repo (WSJF 8.0) should rank above spread repo (WSJF 5.0); got:\n{out}"
        );
    }
}
