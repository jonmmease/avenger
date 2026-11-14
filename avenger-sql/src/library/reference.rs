//! The reference of the library's functions, as Markdown: for each function, its signatures,
//! its summary and description, its notes, its parameters, and its examples, in the layout of
//! GoogleSQL's function reference.

use super::{Function, FunctionKind, Library, Need, Over};

/// The reference of every function of `library`, as Markdown.
pub fn reference(library: &Library) -> String {
    let mut out = String::from("# Functions\n");
    for function in library.functions() {
        out.push('\n');
        out.push_str(&entry(function));
    }
    out
}

/// The reference of `function`, as Markdown.
pub fn entry(function: &Function) -> String {
    let mut out = format!("## `{}`\n\n", function.name);
    out.push_str("```googlesql\n");
    for signature in function.signatures() {
        out.push_str(&signature.text);
        out.push('\n');
    }
    if let Some(over) = over(function) {
        out.push_str(&over);
        out.push('\n');
    }
    out.push_str("```\n\n");
    out.push_str(&function.summary);
    out.push('\n');
    if !function.description.is_empty() && function.description != function.summary {
        out.push('\n');
        out.push_str(&function.description);
        out.push('\n');
    }
    for note in &function.notes {
        out.push_str(&format!("\n**Note:** {note}\n"));
    }
    if !function.params.is_empty() {
        out.push_str("\n**Parameters**\n\n");
        for param in &function.params {
            // The paragraphs after the first indent, so they stay in the item of the list.
            let doc = param.doc.replace('\n', "\n  ").replace("\n  \n", "\n\n");
            out.push_str(&format!("+ `{}`: {doc}\n", param.name));
        }
    }
    let clauses = clauses(function);
    if !clauses.is_empty() {
        out.push_str(&format!("\n**Clauses**: {}\n", clauses.join(", ")));
    }
    if function.safe {
        out.push_str("\nA call can write `SAFE.` before the name.\n");
    }
    if !function.examples.is_empty() {
        out.push_str("\n**Examples**\n\n");
        for example in &function.examples {
            out.push_str(&format!("```googlesql\n{}\n```\n\n", example.query));
            match function.kind {
                FunctionKind::Scalar if !example.result.contains('\n') => {
                    out.push_str(&format!("Result: `{}`\n\n", example.result));
                }
                _ => out.push_str(&table(&example.result)),
            }
        }
    }
    out
}

/// The `OVER` clause of a window function, as its signature shows it.
fn over(function: &Function) -> Option<String> {
    let Over::Required { order_by, frame } = function.clauses.over else {
        return None;
    };
    let order = match order_by {
        Need::Required => " ORDER BY expression[, ...]",
        Need::Allowed => " [ORDER BY expression[, ...]]",
        Need::Forbidden => "",
    };
    let frame = if frame { " [frame]" } else { "" };
    Some(format!(
        "OVER ([PARTITION BY expression[, ...]]{order}{frame})"
    ))
}

/// The clauses that a call takes in its parentheses, and its `OVER`, for an aggregate.
fn clauses(function: &Function) -> Vec<&'static str> {
    let clauses = function.clauses;
    let mut out = Vec::new();
    if clauses.distinct {
        out.push("`DISTINCT`");
    }
    if clauses.null_treatment {
        out.push("`IGNORE NULLS` and `RESPECT NULLS`");
    }
    if clauses.order_by {
        out.push("`ORDER BY`");
    }
    if clauses.having {
        out.push("`HAVING MAX` and `HAVING MIN`");
    }
    if clauses.limit {
        out.push("`LIMIT`");
    }
    if clauses.over == Over::Optional {
        out.push("`OVER`");
    }
    out
}

/// The rows of a result, whose lines hold its columns separated by ` | `, the first line their
/// names, as a Markdown table, or a sentence as it is.
fn table(result: &str) -> String {
    let mut lines = result.lines();
    let Some(header) = lines.next() else {
        return String::new();
    };
    if !result.contains(" | ") && !result.contains('\n') {
        return format!("{result}\n\n");
    }
    let columns = header.split(" | ").count();
    let mut out = format!("| {header} |\n|{}\n", " --- |".repeat(columns));
    for line in lines {
        out.push_str(&format!("| {line} |\n"));
    }
    out.push('\n');
    out
}
