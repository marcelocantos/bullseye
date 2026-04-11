// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

use std::fmt::Write;
use std::path::Path;

use crate::graph;
use crate::schema::{CrossEdge, Kind, Target, TargetsFile};

/// Render a TargetsFile to the standard targets.md markdown format.
pub fn render_markdown(file: &TargetsFile) -> String {
    let mut out = String::new();
    out.push_str("# Targets\n\n");

    if let Some(ref sha) = file.last_evaluated {
        writeln!(out, "<!-- last-evaluated: {sha} -->\n").unwrap();
    }

    // Active section.
    let mut active: Vec<(&str, &Target)> = file.active().into_iter().collect();
    active.sort_by(|a, b| a.0.cmp(b.0));

    out.push_str("## Active\n");
    for (id, target) in &active {
        render_target(&mut out, id, target);
    }
    if active.is_empty() {
        out.push_str("\n(none)\n");
    }

    // Achieved section.
    let mut achieved: Vec<(&str, &Target)> = file.achieved().into_iter().collect();
    achieved.sort_by(|a, b| {
        // Most recently achieved first.
        b.1.achieved.cmp(&a.1.achieved)
    });

    out.push_str("\n## Achieved\n");
    for (id, target) in &achieved {
        render_target(&mut out, id, target);
    }
    if achieved.is_empty() {
        out.push_str("\n(none)\n");
    }

    // Mermaid graph if there are active targets.
    if !active.is_empty() {
        let mermaid = graph::mermaid(file);
        writeln!(out, "\n## Graph\n\n```mermaid\n{mermaid}\n```").unwrap();
    }

    out
}

fn render_target(out: &mut String, id: &str, t: &Target) {
    // Verify targets are observable by definition; only mark
    // observable work targets explicitly. See 🎯T7.
    let kind_suffix = match t.kind {
        Kind::Verify => " ✓",
        Kind::Work if t.observable => " ⦿",
        Kind::Work => "",
    };
    writeln!(out, "\n### 🎯{id}{kind_suffix} {}", t.name).unwrap();
    writeln!(out, "- **Value**: {}", t.value).unwrap();
    writeln!(out, "- **Cost**: {}", t.cost).unwrap();
    if t.observable {
        writeln!(out, "- **Observable**: true").unwrap();
    }

    if t.acceptance.len() == 1 {
        writeln!(out, "- **Acceptance**: {}", t.acceptance[0]).unwrap();
    } else {
        out.push_str("- **Acceptance**:\n");
        for criterion in &t.acceptance {
            writeln!(out, "  - {criterion}").unwrap();
        }
    }

    if !t.context.is_empty() {
        writeln!(out, "- **Context**: {}", t.context).unwrap();
    }

    if !t.depends_on.is_empty() {
        let deps: Vec<String> = t.depends_on.iter().map(|d| format!("🎯{d}")).collect();
        writeln!(out, "- **Depends on**: {}", deps.join(", ")).unwrap();
    }

    if !t.cross_depends.is_empty() {
        out.push_str("- **Cross-depends**:\n");
        for edge in &t.cross_depends {
            render_cross_edge(out, edge);
        }
    }

    if !t.cross_enables.is_empty() {
        out.push_str("- **Cross-enables**:\n");
        for edge in &t.cross_enables {
            render_cross_edge(out, edge);
        }
    }

    if !t.verifies.is_empty() {
        let vs: Vec<String> = t.verifies.iter().map(|v| format!("🎯{v}")).collect();
        writeln!(out, "- **Verifies**: {}", vs.join(", ")).unwrap();
    }

    if let Some(ref rework) = t.rework {
        writeln!(out, "- **Rework**: 🎯{rework}").unwrap();
    }

    if let Some(budget) = t.retry_budget {
        writeln!(out, "- **Retry budget**: {budget}").unwrap();
    }

    if t.retries > 0 {
        writeln!(out, "- **Retries**: {}", t.retries).unwrap();
    }

    if !t.tags.is_empty() {
        writeln!(out, "- **Tags**: {}", t.tags.join(", ")).unwrap();
    }

    if t.origin != "manual" {
        writeln!(out, "- **Origin**: {}", t.origin).unwrap();
    }

    writeln!(out, "- **Status**: {:?}", t.status).unwrap();
    writeln!(out, "- **Discovered**: {}", t.discovered).unwrap();

    if let Some(achieved) = t.achieved {
        writeln!(out, "- **Achieved**: {achieved}").unwrap();
    }

    if let Some(actual) = t.actual_cost {
        writeln!(out, "- **Actual-cost**: {actual}").unwrap();
    }
}

/// Render a single cross-repo edge as a nested bullet line.
fn render_cross_edge(out: &mut String, edge: &CrossEdge) {
    let ref_ = edge.reference();
    let mut line = format!("  - {ref_} @ `{}`", edge.repo);
    if let Some(ref note) = edge.note {
        line.push_str(" — ");
        line.push_str(note);
    }
    line.push('\n');
    out.push_str(&line);
}

/// Derive the markdown path from the YAML path.
/// `docs/targets.yaml` → `docs/targets.md`
pub fn markdown_path(yaml_path: &Path) -> std::path::PathBuf {
    yaml_path.with_extension("md")
}

/// Render and write the markdown file alongside the YAML source.
pub fn render_to_file(yaml_path: &Path, file: &TargetsFile) -> Result<(), String> {
    let md_path = markdown_path(yaml_path);
    let content = render_markdown(file);
    std::fs::write(&md_path, content)
        .map_err(|e| format!("failed to write {}: {e}", md_path.display()))
}
