// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Re-insert ledger targets absent from the live file but present at a
//! pinned git revision (🎯T105). Does not overwrite live rows and does
//! not route through apply's explicit-create / `id_reserved` path.

use chrono::Local;

use std::path::Path;

use crate::api::{CodedError, ErrorCode};
use crate::bounded::{GIT_QUERY_TIMEOUT, GitQueryError, git_query_detailed};
use crate::schema::TargetsFile;

/// Caller-supplied restore parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreRequest {
    /// Commit SHA or ref naming the source `bullseye.yaml` snapshot.
    pub source_rev: String,
    /// Target ids to copy from the source when absent live.
    pub ids: Vec<String>,
    /// Human audit note (required, non-empty).
    pub reason: String,
}

/// Outcome of a successful restore mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreReport {
    pub restored: Vec<String>,
}

/// Resolve `rev` to a commit OID and refuse unrelated histories (🎯T105).
///
/// Uses `--end-of-options` so untrusted revision strings cannot inject
/// extra git arguments. Requires a non-empty `merge-base` between
/// `HEAD` and the source commit so a fetched foreign-repo object cannot
/// be used as a restore source.
pub fn resolve_pinned_source_revision(repo: &Path, rev: &str) -> Result<String, CodedError> {
    let trimmed = rev.trim();
    let spec = format!("{trimmed}^{{commit}}");
    let source_sha = match git_query_detailed(
        repo,
        &["rev-parse", "--verify", "--end-of-options", &spec],
        GIT_QUERY_TIMEOUT,
    ) {
        Ok(sha) => sha.trim().to_string(),
        Err(GitQueryError::NotARepo) => {
            return Err(CodedError::new(
                ErrorCode::UnsafeRepo,
                "ledger is not inside a git repository",
            ));
        }
        Err(GitQueryError::Failed(reason)) => {
            return Err(CodedError::new(
                ErrorCode::Validation,
                format!("source revision `{trimmed}` does not resolve: {reason}"),
            ));
        }
    };

    let head = match git_query_detailed(
        repo,
        &["rev-parse", "--verify", "--end-of-options", "HEAD^{commit}"],
        GIT_QUERY_TIMEOUT,
    ) {
        Ok(h) => h.trim().to_string(),
        Err(GitQueryError::NotARepo) => {
            return Err(CodedError::new(
                ErrorCode::UnsafeRepo,
                "ledger is not inside a git repository",
            ));
        }
        Err(GitQueryError::Failed(reason)) => {
            return Err(CodedError::new(
                ErrorCode::Validation,
                format!("cannot resolve HEAD for history check: {reason}"),
            ));
        }
    };

    match git_query_detailed(
        repo,
        &["merge-base", "--end-of-options", &head, &source_sha],
        GIT_QUERY_TIMEOUT,
    ) {
        Ok(base) if !base.trim().is_empty() => Ok(source_sha),
        Ok(_) => Err(CodedError::new(
            ErrorCode::Validation,
            format!(
                "source revision `{trimmed}` shares no git history with the current ledger                  branch (HEAD) — refusing restore from an unrelated object"
            ),
        )),
        Err(GitQueryError::NotARepo) => Err(CodedError::new(
            ErrorCode::UnsafeRepo,
            "ledger is not inside a git repository",
        )),
        Err(GitQueryError::Failed(_)) => Err(CodedError::new(
            ErrorCode::Validation,
            format!(
                "source revision `{trimmed}` shares no git history with the current ledger                  branch (HEAD) — refusing restore from an unrelated object"
            ),
        )),
    }
}

/// Validate request shape before any git I/O.
pub fn validate_request(req: &RestoreRequest) -> Result<(), CodedError> {
    let reason = req.reason.trim();
    if reason.is_empty() {
        return Err(CodedError::new(
            ErrorCode::InvalidArgs,
            "restore requires non-empty `reason`",
        ));
    }
    let rev = req.source_rev.trim();
    if rev.is_empty() {
        return Err(CodedError::new(
            ErrorCode::InvalidArgs,
            "restore requires non-empty `source_rev` (commit SHA or ref)",
        ));
    }
    if req.ids.is_empty() {
        return Err(CodedError::new(
            ErrorCode::InvalidArgs,
            "restore requires at least one target id (`--id`)",
        ));
    }
    Ok(())
}

/// Insert absent targets from `source` into `live`, refusing overwrites.
pub fn restore_absent(
    live: &mut TargetsFile,
    source: &TargetsFile,
    source_rev: &str,
    ids: &[String],
    reason: &str,
) -> Result<RestoreReport, CodedError> {
    let reason = reason.trim();
    let source_rev = source_rev.trim();
    let today = Local::now().format("%Y-%m-%d").to_string();
    let mut restored = Vec::new();

    for id in ids {
        if live.targets.contains_key(id) {
            return Err(CodedError::new(
                ErrorCode::Conflict,
                format!(
                    "🎯{id} already exists in the live ledger — restore refuses to overwrite. \
                     Patch or remove the live row first."
                ),
            ));
        }
        let Some(src) = source.targets.get(id) else {
            return Err(CodedError::new(
                ErrorCode::NotFound,
                format!("🎯{id} is not present in the ledger at source revision `{source_rev}`"),
            ));
        };
        let mut target = src.clone();
        let audit = format!("Restored from git revision {source_rev} on {today}: {reason}");
        if target.context.is_empty() {
            target.context = audit;
        } else {
            target.context.push_str("\n\n");
            target.context.push_str(&audit);
        }
        live.targets.insert(id.clone(), target);
        restored.push(id.clone());
    }

    Ok(RestoreReport { restored })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Status, Target};

    fn sample_target(name: &str) -> Target {
        Target {
            name: name.to_string(),
            status: Status::Identified,
            value: 0.0,
            cost: 0.0,
            acceptance: vec!["a".into()],
            context: "ctx".into(),
            origin: "manual".into(),
            discovered: "2026-01-01".parse().unwrap(),
            checks: vec![],
            gates: vec![],
            depends_on: vec![],
            cross_depends: vec![],
            cross_enables: vec![],
            tags: vec![],
            strategy: None,
            actual_cost: None,
            set_aside_reason: None,
            attestation: None,
            owned_by: None,
            postponed_until: None,
            postpone_predicate: None,
            achieved: None,
        }
    }

    #[test]
    fn restores_absent_row_with_audit() {
        let mut live = TargetsFile::default();
        live.targets.insert("T1".into(), sample_target("live"));
        let mut source = TargetsFile::default();
        source
            .targets
            .insert("T1216".into(), sample_target("specimen"));
        let report = restore_absent(
            &mut live,
            &source,
            "a7b84f49",
            &["T1216".into()],
            "integrator rollback dropped row",
        )
        .expect("restore");
        assert_eq!(report.restored, vec!["T1216".to_string()]);
        let t = live.targets.get("T1216").expect("inserted");
        assert_eq!(t.name, "specimen");
        assert!(t.context.contains("a7b84f49"));
        assert!(t.context.contains("integrator rollback"));
        assert!(live.targets.contains_key("T1"));
    }

    #[test]
    fn refuses_live_collision() {
        let mut live = TargetsFile::default();
        live.targets.insert("T9".into(), sample_target("live"));
        let mut source = TargetsFile::default();
        source.targets.insert("T9".into(), sample_target("old"));
        let err = restore_absent(&mut live, &source, "deadbeef", &["T9".into()], "reason")
            .expect_err("must refuse");
        assert_eq!(err.code, ErrorCode::Conflict);
    }

    #[test]
    fn refuses_missing_in_source() {
        let mut live = TargetsFile::default();
        let source = TargetsFile::default();
        let err = restore_absent(&mut live, &source, "deadbeef", &["T99".into()], "reason")
            .expect_err("must refuse");
        assert_eq!(err.code, ErrorCode::NotFound);
    }
}
