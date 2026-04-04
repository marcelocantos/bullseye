// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use crate::schema::{Kind, Status, TargetsFile};

/// Result of a rework operation.
#[derive(Debug)]
pub struct ReworkResult {
    /// The rework destination target ID.
    pub rework_id: String,
    /// Name of the rework destination.
    pub rework_name: String,
    /// New retry count after this rework.
    pub retries: u32,
    /// Retry budget (if set).
    pub budget: Option<u32>,
    /// Whether the retry budget is exhausted.
    pub budget_exhausted: bool,
}

/// Error from a rework operation.
#[derive(Debug, PartialEq)]
pub enum ReworkError {
    TargetNotFound(String),
    NotVerifyTarget(String),
    NoReworkTarget(String),
    ReworkDestNotFound(String),
}

impl std::fmt::Display for ReworkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReworkError::TargetNotFound(id) => write!(f, "target {id} not found"),
            ReworkError::NotVerifyTarget(id) => write!(f, "🎯{id} is not a verify target"),
            ReworkError::NoReworkTarget(id) => write!(f, "🎯{id} has no rework target"),
            ReworkError::ReworkDestNotFound(id) => write!(f, "rework target {id} does not exist"),
        }
    }
}

/// Execute a rework cycle: reset verify target to identified, reset
/// rework destination to converging, increment retries, append diagnosis.
///
/// Returns information about the rework for display. Does NOT save to disk.
pub fn rework(
    file: &mut TargetsFile,
    verify_id: &str,
    diagnosis: &str,
) -> Result<ReworkResult, ReworkError> {
    // Validate the verify target.
    let verify = file
        .targets
        .get(verify_id)
        .ok_or_else(|| ReworkError::TargetNotFound(verify_id.to_string()))?;

    if verify.kind != Kind::Verify {
        return Err(ReworkError::NotVerifyTarget(verify_id.to_string()));
    }

    let rework_id = verify
        .rework
        .clone()
        .ok_or_else(|| ReworkError::NoReworkTarget(verify_id.to_string()))?;

    if !file.targets.contains_key(&rework_id) {
        return Err(ReworkError::ReworkDestNotFound(rework_id));
    }

    // Reset the verify target to identified.
    file.targets.get_mut(verify_id).unwrap().status = Status::Identified;

    // Reset the rework target to converging and increment retries.
    let rework_target = file.targets.get_mut(&rework_id).unwrap();
    rework_target.status = Status::Converging;
    rework_target.retries += 1;
    let retries = rework_target.retries;
    let budget = rework_target.retry_budget;
    let rework_name = rework_target.name.clone();

    // Append diagnosis to rework target's context if provided.
    if !diagnosis.is_empty() {
        let ctx = &mut file.targets.get_mut(&rework_id).unwrap().context;
        if !ctx.is_empty() {
            ctx.push_str("\n\n");
        }
        ctx.push_str(&format!("Rework #{retries}: {diagnosis}"));
    }

    Ok(ReworkResult {
        rework_id,
        rework_name,
        retries,
        budget,
        budget_exhausted: budget.is_some_and(|b| retries >= b),
    })
}
