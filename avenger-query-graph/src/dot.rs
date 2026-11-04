//! Graphviz DOT graphs of a graph's nodes, and of a run's outcomes.

use std::fmt::Write;

use avenger_selection_datafusion::Owners;

use crate::{graph::QueryNode, run::Outcome};

/// Escape `text` for a left-justified line of a DOT label.
fn line(label: &mut String, text: &str) {
    for part in text.lines() {
        let escaped = part.replace('\\', "\\\\").replace('"', "\\\"");
        let _ = write!(label, "{escaped}\\l");
    }
}

fn owners(owners: &Owners) -> String {
    let list = |names: &std::collections::BTreeSet<String>| {
        names.iter().cloned().collect::<Vec<_>>().join(", ")
    };
    match owners {
        Owners::AllBut(skipped) if skipped.is_empty() => "every owner".to_string(),
        Owners::AllBut(skipped) => format!("every owner but {}", list(skipped)),
        Owners::Only(read) => format!("only {}", list(read)),
    }
}

/// A DOT graph of `nodes`, with each node's outcome when `outcomes` is given.
pub(crate) fn to_dot(nodes: &[QueryNode], outcomes: Option<&[Outcome]>) -> String {
    let mut dot = String::from(
        "digraph query_graph {\n  rankdir=BT;\n  node [shape=box, fontname=\"monospace\", fontsize=10];\n",
    );
    for (index, node) in nodes.iter().enumerate() {
        let mut label = String::new();
        let mut title = format!("node {index}");
        if !node.top_of.is_empty() {
            let plans: Vec<String> = node.top_of.iter().map(|plan| plan.to_string()).collect();
            let _ = write!(title, ", top of plan {}", plans.join(", "));
        }
        if !node.kept {
            title.push_str(", not kept");
        }
        if !node.replaces.is_empty() {
            let _ = write!(title, ", {} rollup(s)", node.replaces.len());
        }
        if node.cells {
            title.push_str(", cells");
        }
        line(&mut label, &title);
        if !node.params.is_empty() {
            line(&mut label, &format!("params: {}", node.params.join(", ")));
        }
        for read in &node.selections {
            line(
                &mut label,
                &format!("{}: {}", read.parameter, owners(&read.owners)),
            );
        }
        if !node.tables.is_empty() {
            let tables: Vec<String> = node.tables.iter().map(|table| table.to_string()).collect();
            line(&mut label, &format!("tables: {}", tables.join(", ")));
        }
        let mut fill = "#ffffff";
        if let Some(outcome) = outcomes.and_then(|outcomes| outcomes.get(index)) {
            let (name, output, elapsed, widened, color) = match outcome {
                Outcome::Hit(output) => ("hit", Some(output), None, None, "#cfe8cf"),
                Outcome::Computed {
                    output,
                    elapsed,
                    widened,
                } => (
                    "computed",
                    Some(output),
                    Some(elapsed),
                    Some(widened),
                    "#f6d7a7",
                ),
                Outcome::Uncached { output, elapsed } => {
                    ("uncached", Some(output), Some(elapsed), None, "#f4b6b6")
                }
                Outcome::NotRead => ("not read", None, None, None, "#e6e6e6"),
            };
            fill = color;
            line(&mut label, &format!("outcome: {name}"));
            if let Some(output) = output {
                line(
                    &mut label,
                    &format!("rows: {}, bytes: {}", output.rows, output.bytes),
                );
                line(
                    &mut label,
                    &format!("columns: {}", output.columns.join(", ")),
                );
            }
            if let Some(elapsed) = elapsed {
                line(
                    &mut label,
                    &format!("time: {:.3} ms", elapsed.as_secs_f64() * 1000.0),
                );
            }
            if let Some(widened) = widened.filter(|widened| !widened.is_empty()) {
                line(&mut label, &format!("widened: {}", widened.join(", ")));
            }
        }
        label.push_str("\\l");
        line(&mut label, &node.plan.display_indent().to_string());
        let _ = writeln!(
            dot,
            "  n{index} [label=\"{label}\", style=filled, fillcolor=\"{fill}\"];"
        );
    }
    for (index, node) in nodes.iter().enumerate() {
        for (position, input) in node.inputs.iter().enumerate() {
            // A label names the cut's scan in the reader's plan, which only a reader of several
            // nodes needs.
            match node.inputs.len() {
                1 => {
                    let _ = writeln!(dot, "  n{input} -> n{index};");
                }
                _ => {
                    let _ = writeln!(dot, "  n{input} -> n{index} [label=\"input_{position}\"];");
                }
            }
        }
    }
    dot.push_str("}\n");
    dot
}
