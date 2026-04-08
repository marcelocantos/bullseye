// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use std::collections::BTreeMap;

use chrono::NaiveDate;
use regex::Regex;

use crate::schema::{GateEdge, Kind, Status, Target, TargetsFile};

/// Parse a targets.md markdown file into a TargetsFile.
///
/// Tolerant of formatting variations across repos:
/// - Free-text descriptions between the header and bullet list
/// - Case-insensitive status values
/// - Missing optional fields
pub fn parse_markdown(input: &str) -> Result<TargetsFile, String> {
    let last_evaluated = parse_last_evaluated(input);
    let mut targets = BTreeMap::new();

    let header_re = Regex::new(r"^###\s+🎯(T[\d.]+)\s*(✓)?\s+(.+)$").expect("invalid header regex");

    let lines: Vec<&str> = input.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        if let Some(caps) = header_re.captures(line) {
            let id = caps[1].to_string();
            let is_verify = caps.get(2).is_some();
            let name = caps[3].trim().to_string();

            i += 1;

            // Skip blank lines after header.
            while i < lines.len() && lines[i].trim().is_empty() {
                i += 1;
            }

            // Collect free-text description lines (between header and first bullet).
            let mut description_lines = Vec::new();
            while i < lines.len() {
                let l = lines[i].trim();
                if l.starts_with("- **") {
                    break;
                }
                // Skip the mermaid graph and section headers.
                if l.starts_with("##") || l.starts_with("```") {
                    break;
                }
                if l.is_empty() && description_lines.is_empty() {
                    // Leading blank — skip.
                    i += 1;
                    continue;
                }
                if l.is_empty() {
                    // Could be end of description or paragraph break — peek ahead.
                    let mut j = i + 1;
                    while j < lines.len() && lines[j].trim().is_empty() {
                        j += 1;
                    }
                    if j < lines.len() && lines[j].trim().starts_with("- **") {
                        break;
                    }
                    // Not followed by bullets — could be paragraph break in description.
                    // But if followed by another header, stop.
                    if j < lines.len() && lines[j].trim().starts_with("##") {
                        break;
                    }
                }
                description_lines.push(lines[i]);
                i += 1;
            }

            // Skip blank lines between description and bullets.
            while i < lines.len() && lines[i].trim().is_empty() {
                i += 1;
            }

            // Parse bullet fields.
            let mut fields = ParsedFields::default();
            while i < lines.len() {
                let l = lines[i];
                let trimmed = l.trim();

                // Stop at next header or section.
                if trimmed.starts_with("##") || trimmed.starts_with("```") {
                    break;
                }

                if trimmed.starts_with("- **") {
                    parse_field(trimmed, &lines, &mut i, &mut fields);
                } else if trimmed.is_empty() {
                    // Blank line between bullets — skip but continue.
                    i += 1;
                } else {
                    // Unknown line (maybe continuation of free text) — skip.
                    i += 1;
                }
            }

            // Build context: prefer the Context field, fall back to free-text
            // description, combine if both present.
            let context = match (!description_lines.is_empty(), !fields.context.is_empty()) {
                (true, true) => {
                    let desc = description_lines
                        .iter()
                        .map(|l| l.trim())
                        .collect::<Vec<_>>()
                        .join(" ");
                    format!("{}\n{}", desc, fields.context)
                }
                (true, false) => description_lines
                    .iter()
                    .map(|l| l.trim())
                    .collect::<Vec<_>>()
                    .join(" "),
                (false, true) => fields.context.clone(),
                (false, false) => String::new(),
            };

            let target = Target {
                name,
                kind: if is_verify { Kind::Verify } else { Kind::Work },
                status: fields.status.unwrap_or(Status::Identified),
                value: fields.value.unwrap_or(1.0),
                cost: fields.cost.unwrap_or(1.0),
                actual_cost: fields.actual_cost,
                acceptance: fields.acceptance,
                context,
                gates: fields.gates,
                depends_on: fields.depends_on,
                verifies: fields.verifies,
                rework: fields.rework,
                retry_budget: fields.retry_budget,
                retries: fields.retries,
                tags: fields.tags,
                origin: fields.origin.unwrap_or_else(|| "manual".to_string()),
                discovered: fields
                    .discovered
                    .unwrap_or_else(|| chrono::Local::now().date_naive()),
                achieved: fields.achieved,
            };

            targets.insert(id, target);
        } else {
            i += 1;
        }
    }

    if targets.is_empty() {
        return Err("no targets found in markdown".to_string());
    }

    Ok(TargetsFile {
        last_evaluated,
        targets,
    })
}

fn parse_last_evaluated(input: &str) -> Option<String> {
    let re = Regex::new(r"<!--\s*last-evaluated:\s*(\S+)\s*-->").expect("invalid regex");
    re.captures(input).map(|c| c[1].to_string())
}

#[derive(Default)]
struct ParsedFields {
    status: Option<Status>,
    value: Option<f64>,
    cost: Option<f64>,
    actual_cost: Option<f64>,
    acceptance: Vec<String>,
    context: String,
    gates: Vec<GateEdge>,
    depends_on: Vec<String>,
    verifies: Vec<String>,
    rework: Option<String>,
    retry_budget: Option<u32>,
    retries: u32,
    tags: Vec<String>,
    origin: Option<String>,
    discovered: Option<NaiveDate>,
    achieved: Option<NaiveDate>,
}

fn parse_field(line: &str, lines: &[&str], i: &mut usize, fields: &mut ParsedFields) {
    // Extract field name and value from `- **Name**: value` or `- **Name**:`.
    let field_re = Regex::new(r"^-\s+\*\*([^*]+)\*\*:\s*(.*)$").expect("invalid field regex");

    let Some(caps) = field_re.captures(line) else {
        *i += 1;
        return;
    };

    let field_name = caps[1].trim();
    let field_value = caps[2].trim();

    match field_name {
        "Weight" => {
            // Backward compat: parse old "N (value V / cost C)" format.
            let weight_re = Regex::new(r"value\s+(\d+\.?\d*)\s*/\s*cost\s+(\d+\.?\d*)")
                .expect("invalid weight regex");
            if let Some(wc) = weight_re.captures(field_value) {
                fields.value = wc[1].parse().ok();
                fields.cost = wc[2].parse().ok();
            }
        }
        "Value" => {
            fields.value = field_value.parse().ok();
        }
        "Cost" | "Estimated-cost" => {
            fields.cost = field_value.parse().ok();
        }
        "Acceptance" => {
            if !field_value.is_empty() {
                // Single-line acceptance.
                fields.acceptance.push(field_value.to_string());
            }
            // Check for multi-line acceptance (sub-bullets).
            *i += 1;
            while *i < lines.len() {
                let next = lines[*i].trim();
                if next.starts_with("- ") && !next.starts_with("- **") {
                    fields
                        .acceptance
                        .push(next.trim_start_matches("- ").to_string());
                    *i += 1;
                } else {
                    break;
                }
            }
            return; // Already advanced i.
        }
        "Context" => {
            fields.context = field_value.to_string();
        }
        "Parent" => {
            // Backward compat: convert parent to depends_on.
            if let Some(parent_id) = parse_target_ref(field_value)
                && !fields.depends_on.contains(&parent_id)
            {
                fields.depends_on.push(parent_id);
            }
        }
        "Gates" => {
            fields.gates = parse_gates(field_value);
        }
        "Depends on" => {
            fields.depends_on = parse_target_refs(field_value);
        }
        "Verifies" => {
            fields.verifies = parse_target_refs(field_value);
        }
        "Rework" => {
            fields.rework = parse_target_ref(field_value);
        }
        "Retry budget" => {
            fields.retry_budget = field_value.parse().ok();
        }
        "Retries" => {
            fields.retries = field_value.parse().unwrap_or(0);
        }
        "Tags" => {
            fields.tags = field_value
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();
        }
        "Origin" => {
            fields.origin = Some(field_value.to_string());
        }
        "Status" => {
            fields.status = parse_status(field_value);
        }
        "Discovered" => {
            fields.discovered = NaiveDate::parse_from_str(field_value, "%Y-%m-%d").ok();
        }
        "Achieved" => {
            fields.achieved = NaiveDate::parse_from_str(field_value, "%Y-%m-%d").ok();
        }
        "Actual-cost" => {
            fields.actual_cost = field_value.parse().ok();
        }
        _ => {
            // Unknown field — ignore.
        }
    }

    *i += 1;
}

fn parse_status(s: &str) -> Option<Status> {
    match s.to_lowercase().as_str() {
        "identified" => Some(Status::Identified),
        "converging" => Some(Status::Converging),
        "achieved" => Some(Status::Achieved),
        _ => None,
    }
}

/// Extract a single target ref like "🎯T1" or "🎯T1.2" → "T1" / "T1.2".
fn parse_target_ref(s: &str) -> Option<String> {
    let re = Regex::new(r"🎯(T[\d.]+)").expect("invalid target ref regex");
    re.captures(s).map(|c| c[1].to_string())
}

/// Extract multiple target refs from a comma-separated list.
fn parse_target_refs(s: &str) -> Vec<String> {
    let re = Regex::new(r"🎯(T[\d.]+)").expect("invalid target ref regex");
    re.captures_iter(s).map(|c| c[1].to_string()).collect()
}

/// Parse gates from "🎯T1 (80%), 🎯T2" format.
fn parse_gates(s: &str) -> Vec<GateEdge> {
    let re = Regex::new(r"🎯(T[\d.]+)(?:\s*\((\d+)%\))?").expect("invalid gates regex");
    re.captures_iter(s)
        .map(|c| {
            let target = c[1].to_string();
            let criticality = c
                .get(2)
                .and_then(|m| m.as_str().parse::<f64>().ok())
                .map(|p| p / 100.0)
                .unwrap_or(1.0);
            GateEdge {
                target,
                criticality,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_roundtrip() {
        let yaml_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/docs/targets.yaml");
        let file = crate::store::load(&yaml_path).unwrap();
        let markdown = crate::render::render_markdown(&file);
        let parsed = parse_markdown(&markdown).unwrap();

        assert_eq!(parsed.targets.len(), file.targets.len());
        assert_eq!(parsed.last_evaluated, file.last_evaluated);

        for (id, original) in &file.targets {
            let imported = parsed
                .targets
                .get(id)
                .unwrap_or_else(|| panic!("missing target {id} in parsed output"));
            assert_eq!(imported.name, original.name, "name mismatch for {id}");
            assert_eq!(imported.kind, original.kind, "kind mismatch for {id}");
            assert_eq!(imported.status, original.status, "status mismatch for {id}");
            assert!(
                (imported.value - original.value).abs() < 0.01,
                "value mismatch for {id}: {} vs {}",
                imported.value,
                original.value
            );
            assert!(
                (imported.cost - original.cost).abs() < 0.01,
                "cost mismatch for {id}: {} vs {}",
                imported.cost,
                original.cost
            );
            assert_eq!(
                imported.acceptance, original.acceptance,
                "acceptance mismatch for {id}"
            );
            // depends_on may include converted parent refs; check subset.

            assert_eq!(
                imported.depends_on, original.depends_on,
                "depends_on mismatch for {id}"
            );
            assert_eq!(
                imported.verifies, original.verifies,
                "verifies mismatch for {id}"
            );
            assert_eq!(imported.rework, original.rework, "rework mismatch for {id}");
            assert_eq!(
                imported.retry_budget, original.retry_budget,
                "retry_budget mismatch for {id}"
            );
            assert_eq!(imported.tags, original.tags, "tags mismatch for {id}");
            assert_eq!(
                imported.discovered, original.discovered,
                "discovered mismatch for {id}"
            );
            assert_eq!(
                imported.achieved, original.achieved,
                "achieved mismatch for {id}"
            );
        }
    }

    #[test]
    fn parse_self_targets() {
        // Parse bullseye's own docs/targets.md — a real-world file.
        let md_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/targets.md");
        if !md_path.exists() {
            return; // Skip if not present.
        }
        let content = std::fs::read_to_string(&md_path).unwrap();
        let parsed = parse_markdown(&content).unwrap();
        assert!(parsed.targets.len() >= 20, "expected at least 20 targets");

        // Spot-check a known target.
        let t5_1 = parsed.targets.get("T5.1").expect("T5.1 not found");
        assert_eq!(t5_1.name, "Markdown-to-YAML target converter");
        assert!(
            t5_1.depends_on.contains(&"T5".to_string()),
            "T5.1 should depend on T5"
        );
        assert!(
            t5_1.status == Status::Identified
                || t5_1.status == Status::Converging
                || t5_1.status == Status::Achieved,
            "T5.1 should have a valid status"
        );
    }

    #[test]
    fn parse_gates() {
        let md = r#"# Targets

## Active

### 🎯T1 Test target
- **Weight**: 2 (value 5 / cost 3)
- **Acceptance**: it works
- **Gates**: 🎯T2 (80%), 🎯T3
- **Status**: Identified
- **Discovered**: 2026-04-07
"#;
        let file = parse_markdown(md).unwrap();
        let t1 = &file.targets["T1"];
        assert_eq!(t1.gates.len(), 2);
        assert_eq!(t1.gates[0].target, "T2");
        assert!((t1.gates[0].criticality - 0.8).abs() < 0.01);
        assert_eq!(t1.gates[1].target, "T3");
        assert!((t1.gates[1].criticality - 1.0).abs() < 0.01);
    }

    #[test]
    fn parse_verify_kind() {
        let md = r#"# Targets

## Active

### 🎯T5 ✓ Verify things work
- **Weight**: 3 (value 3 / cost 1)
- **Acceptance**: tests pass
- **Verifies**: 🎯T1, 🎯T3
- **Rework**: 🎯T1
- **Status**: Identified
- **Discovered**: 2026-03-10
"#;
        let file = parse_markdown(md).unwrap();
        let t5 = &file.targets["T5"];
        assert_eq!(t5.kind, Kind::Verify);
        assert_eq!(t5.verifies, vec!["T1", "T3"]);
        assert_eq!(t5.rework.as_deref(), Some("T1"));
    }

    #[test]
    fn parse_free_text_description() {
        let md = r#"# Targets

## Active

### 🎯T1 Some target with description

This is a free-text description that spans
multiple lines before the bullets.

- **Weight**: 2 (value 5 / cost 3)
- **Acceptance**: it works
- **Status**: Identified
- **Discovered**: 2026-04-07
"#;
        let file = parse_markdown(md).unwrap();
        let t1 = &file.targets["T1"];
        assert!(t1.context.contains("free-text description"));
    }

    #[test]
    fn parse_empty_returns_error() {
        let result = parse_markdown("# Targets\n\n## Active\n\n(none)\n");
        assert!(result.is_err());
    }
}
