// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Shared wire-contract helpers for the core API (🎯T45).
//!
//! Mutation results and errors use a stable text envelope so agents can
//! branch on codes and IDs without scraping free-form prose alone.

use std::path::Path;

use crate::graph;
use crate::schema::TargetsFile;

/// Stable error codes for agent branching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    NotInitialized,
    Conflict,
    ImmutableAchieved,
    IdReserved,
    Validation,
    UnsafeRepo,
    NotFound,
    InvalidArgs,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotInitialized => "not_initialized",
            Self::Conflict => "conflict",
            Self::ImmutableAchieved => "immutable_achieved",
            Self::IdReserved => "id_reserved",
            Self::Validation => "validation",
            Self::UnsafeRepo => "unsafe_repo",
            Self::NotFound => "not_found",
            Self::InvalidArgs => "invalid_args",
        }
    }
}

/// Format a coded error payload.
pub fn format_error(code: ErrorCode, message: impl AsRef<str>) -> String {
    format!("code={}\nmessage: {}", code.as_str(), message.as_ref())
}

/// Best-effort classification of an existing error string into a code.
pub fn classify_message(msg: &str) -> ErrorCode {
    let lower = msg.to_ascii_lowercase();
    if lower.contains("no bullseye.yaml") || lower.contains("create bullseye.yaml") {
        return ErrorCode::NotInitialized;
    }
    if lower.contains("conflict:") || lower.contains("modified externally") {
        return ErrorCode::Conflict;
    }
    if lower.contains("lock timeout") || lower.contains("lockfile error") {
        return ErrorCode::Conflict;
    }
    if lower.contains("is achieved")
        && (lower.contains("immutable") || lower.contains("content is immutable"))
    {
        return ErrorCode::ImmutableAchieved;
    }
    if lower.contains("collides with a target recorded in git history") {
        return ErrorCode::IdReserved;
    }
    if lower.contains("detached head")
        || lower.contains("submodule")
        || lower.contains("unsafe repo")
        || lower.contains("refusing to mutate")
    {
        return ErrorCode::UnsafeRepo;
    }
    if lower.contains("not found") {
        return ErrorCode::NotFound;
    }
    if lower.contains("validation")
        || lower.contains("unknown status")
        || lower.contains("unknown filter")
        || lower.contains("unknown view")
        || lower.contains("unknown op")
        || lower.contains("required")
        || lower.contains("mutually exclusive")
        || lower.contains("does not exist")
    {
        // Prefer more specific codes already matched above.
        if lower.contains("target") && lower.contains("not found") {
            return ErrorCode::NotFound;
        }
        return ErrorCode::InvalidArgs;
    }
    ErrorCode::InvalidArgs
}

/// Format a mutation success envelope plus human body.
pub fn format_mutation_result(
    op: &str,
    ids: &[String],
    changed: &[String],
    frontier: &[String],
    file: &Path,
    body: &str,
) -> String {
    let mut out = String::from("# result\n");
    out.push_str("ok: true\n");
    out.push_str(&format!("op: {op}\n"));
    out.push_str(&format!("ids: {}\n", join_ids(ids)));
    out.push_str(&format!("changed: {}\n", join_ids(changed)));
    out.push_str(&format!("frontier: {}\n", join_ids(frontier)));
    out.push_str(&format!("file: {}\n", file.display()));
    out.push('\n');
    out.push_str(body);
    if !body.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn join_ids(ids: &[String]) -> String {
    if ids.is_empty() {
        return "(none)".to_string();
    }
    ids.join(", ")
}

/// Frontier target IDs in repo-scope order (fanout, then id).
pub fn frontier_ids(file: &TargetsFile) -> Vec<String> {
    let targets = graph::frontier(file);
    let ranked = graph::rank_frontier(file, &targets);
    ranked.into_iter().map(|r| r.target.id.clone()).collect()
}

/// Load frontier IDs from a yaml path; empty on load failure.
pub fn frontier_ids_from_path(path: &Path) -> Vec<String> {
    match crate::store::load(path) {
        Ok(file) => frontier_ids(&file),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_not_initialized() {
        assert_eq!(
            classify_message("no bullseye.yaml found for /tmp/x"),
            ErrorCode::NotInitialized
        );
    }

    #[test]
    fn classify_immutable() {
        assert_eq!(
            classify_message("🎯T1 is achieved — its content is immutable. Re-open"),
            ErrorCode::ImmutableAchieved
        );
    }

    #[test]
    fn classify_id_reserved() {
        assert_eq!(
            classify_message("🎯T9 collides with a target recorded in git history"),
            ErrorCode::IdReserved
        );
    }

    #[test]
    fn format_mutation_lists_ids() {
        let text = format_mutation_result(
            "track",
            &["T2".into()],
            &["T2".into(), "T3".into()],
            &["T2".into()],
            Path::new("/tmp/bullseye.yaml"),
            "Created 🎯T2",
        );
        assert!(text.contains("ok: true"));
        assert!(text.contains("op: track"));
        assert!(text.contains("ids: T2"));
        assert!(text.contains("changed: T2, T3"));
        assert!(text.contains("frontier: T2"));
        assert!(text.contains("Created 🎯T2"));
    }
}
