// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Surgical repair for `context: |` literal blocks where a continuation line
//! lost its indentation and breaks YAML parsing (🎯T106 / claudia T226).

use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

use crate::api::ErrorCode;
use crate::schema::TargetsFile;
use crate::store::{compute_content_hash, parse_targets_content_strict};

/// Caller parameters for a context-indent repair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextIndentRepairRequest {
    pub pre_repair_sha256: String,
    pub reason: String,
    pub dry_run: bool,
}

/// Outcome of a successful repair (or dry-run preview).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextIndentRepairReport {
    pub pre_repair_sha256: String,
    pub post_repair_sha256: String,
    pub repaired_lines: Vec<usize>,
    pub reason: String,
    pub dry_run: bool,
    pub target_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairError {
    pub code: ErrorCode,
    pub message: String,
}

pub fn validate_request(req: &ContextIndentRepairRequest) -> Result<(), RepairError> {
    if req.pre_repair_sha256.trim().is_empty() {
        return Err(RepairError {
            code: ErrorCode::InvalidArgs,
            message: "pre-repair-sha256 must be non-empty".into(),
        });
    }
    if req.reason.trim().is_empty() {
        return Err(RepairError {
            code: ErrorCode::InvalidArgs,
            message: "reason must be non-empty (audit trail)".into(),
        });
    }
    Ok(())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex_encode(&digest)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

/// Collect target ids from raw ledger text (stable across surgical edits).
pub fn target_ids_in_raw(raw: &str) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for line in raw.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with('T') {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix('T') {
            let id: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if !id.is_empty() && trimmed[id.len() + 1..].starts_with(':') {
                ids.insert(format!("T{id}"));
            }
        }
    }
    ids
}

fn leading_spaces(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ').count()
}

fn is_target_key_line(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with('T') && t.contains(':') && leading_spaces(line) <= 2
}

fn is_sibling_field_line(line: &str, context_key_indent: usize) -> bool {
    if line.trim().is_empty() {
        return false;
    }
    let indent = leading_spaces(line);
    indent == context_key_indent && line[indent..].contains(':') && !line[indent..].starts_with('|')
}

/// Re-indent continuation lines that dedented inside an active `context: |` block.
pub fn repair_dedented_context_literal_lines(raw: &str) -> (String, Vec<usize>) {
    let mut out = String::new();
    let mut repaired_lines = Vec::new();

    let mut in_block = false;
    let mut context_key_indent = 0usize;
    let mut content_indent: Option<usize> = None;

    for (idx, line) in raw.split_inclusive('\n').enumerate() {
        let line_no = idx + 1;
        let line_body = line.strip_suffix('\n').unwrap_or(line);

        if let Some(pos) = line_body.find("context:") {
            let before = &line_body[..pos];
            if before.chars().all(|c| c == ' ')
                && line_body[pos..].trim_start().starts_with("context:")
                && line_body.contains('|')
            {
                in_block = true;
                context_key_indent = before.len();
                content_indent = None;
                out.push_str(line);
                continue;
            }
        }

        if in_block {
            if line_body.trim().is_empty() {
                out.push_str(line);
                continue;
            }

            if is_target_key_line(line_body) {
                in_block = false;
                out.push_str(line);
                continue;
            }

            let indent = leading_spaces(line_body);
            if is_sibling_field_line(line_body, context_key_indent) {
                in_block = false;
                out.push_str(line);
                continue;
            }

            if indent > context_key_indent {
                if content_indent.is_none() {
                    content_indent = Some(indent);
                }
                out.push_str(line);
                continue;
            }

            if let Some(expected) = content_indent {
                if indent < expected && !line_body.trim().is_empty() {
                    let pad = " ".repeat(expected - indent);
                    repaired_lines.push(line_no);
                    out.push_str(&pad);
                    out.push_str(line_body);
                    if line.ends_with('\n') {
                        out.push('\n');
                    }
                    continue;
                }
            }
        }

        out.push_str(line);
    }

    (out, repaired_lines)
}

const HASH_LINE_PREFIX: &str = "# content_hash: sha256:";

/// Replace the stamped content-hash line to match a parsed ledger.
pub fn refresh_content_hash_line(raw: &str, file: &TargetsFile) -> String {
    let expected = compute_content_hash(file);
    let mut out = String::new();
    let mut replaced = false;
    for line in raw.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        if body.starts_with(HASH_LINE_PREFIX) {
            out.push_str(HASH_LINE_PREFIX);
            out.push_str(&expected);
            if line.ends_with('\n') {
                out.push('\n');
            }
            replaced = true;
            continue;
        }
        out.push_str(line);
    }
    if !replaced {
        return raw.to_string();
    }
    out
}

pub fn prepare_repaired_ledger(
    raw: &str,
    pre_repair_sha256: &str,
) -> Result<(String, Vec<usize>, TargetsFile), RepairError> {
    let actual = sha256_hex(raw.as_bytes());
    if actual != pre_repair_sha256.trim().to_lowercase() {
        return Err(RepairError {
            code: ErrorCode::Validation,
            message: format!(
                "pre-repair SHA256 mismatch: expected {pre_repair_sha256}, found {actual}"
            ),
        });
    }

    let ids_before = target_ids_in_raw(raw);
    let (repaired, lines) = repair_dedented_context_literal_lines(raw);
    if lines.is_empty() {
        return Err(RepairError {
            code: ErrorCode::Validation,
            message: "no dedented context literal lines found to repair".into(),
        });
    }

    let path_placeholder = std::path::Path::new("bullseye.yaml");
    let file =
        parse_targets_content_strict(&repaired, path_placeholder).map_err(|e| RepairError {
            code: ErrorCode::Validation,
            message: format!("repaired text still does not parse: {e}"),
        })?;

    let ids_after = target_ids_in_raw(&repaired);
    if ids_before != ids_after {
        return Err(RepairError {
            code: ErrorCode::Validation,
            message: format!(
                "target id set changed during repair (before={}, after={})",
                ids_before.len(),
                ids_after.len()
            ),
        });
    }

    let with_hash = refresh_content_hash_line(&repaired, &file);
    Ok((with_hash, lines, file))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = "schema_version: 3
targets:
  T1:
    name: specimen
    status: identified
    value: 0.0
    cost: 0.0
    acceptance:
    - ok
    discovered: 2026-10-25
    context: |
      first line of context
claudia-po / root: **WITHHOLD achieve** @ private tip
      third line after repair
";

    #[test]
    fn repairs_single_dedented_line_in_context_block() {
        let (fixed, lines) = repair_dedented_context_literal_lines(FIXTURE);
        assert_eq!(lines, vec![13]);
        assert!(
            fixed.contains("      claudia-po / root"),
            "dedented line must gain sibling indent: {fixed}"
        );
        let file = parse_targets_content_strict(&fixed, std::path::Path::new("bullseye.yaml"))
            .expect("parse");
        let ctx = file.targets.get("T1").expect("T1").context.as_str();
        assert!(ctx.contains("WITHHOLD achieve"));
        assert!(ctx.contains("first line"));
        assert!(ctx.contains("third line"));
    }

    #[test]
    fn prepare_rejects_wrong_pre_repair_hash() {
        let raw = FIXTURE;
        let err = prepare_repaired_ledger(raw, "00").expect_err("hash");
        assert_eq!(err.code, ErrorCode::Validation);
    }

    #[test]
    fn prepare_round_trip_preserves_target_ids() {
        let raw = FIXTURE;
        let hash = sha256_hex(raw.as_bytes());
        let (fixed, lines, file) = prepare_repaired_ledger(raw, &hash).expect("repair");
        assert_eq!(lines, vec![13]);
        assert_eq!(file.targets.len(), 1);
        assert!(fixed.contains("claudia-po"));
    }
}
