//! `avenger complete` and `avenger signature`: the completions and the signature help at a
//! position of a file, as an editor shows them, from the same analysis as `avenger lsp`.

use avenger_sql_language::CompletionKind;
use avenger_sql_syntax::FunctionRole;
use serde_json::json;

use crate::{at::At, report::Output, Status};

/// Prints the completions at `at`, `PATH:LINE:COLUMN`, at most `limit` of them.
pub async fn complete(at: &str, limit: usize, output: &Output) -> Result<Status, String> {
    let at = At::resolve(at).await?;
    let document = at.document();
    let completions = at.completions();
    // An editor keeps the items that start with what the user typed before the cursor, and the
    // command keeps them at once: the text from the start of the range to the cursor, and the
    // labels, without the quotes of a string or a quoted name.
    let quotes: &[char] = &['\'', '"', '`'];
    let start = usize::from(completions.range.start());
    let typed = document.text()[start..usize::from(at.offset).max(start)]
        .trim_start_matches(quotes)
        .to_ascii_lowercase();
    let matching: Vec<_> = (completions.items.iter())
        .filter(|item| {
            let label = item.label.trim_start_matches(quotes).to_ascii_lowercase();
            label.starts_with(&typed)
        })
        .collect();
    let total = matching.len();
    let items = &matching[..total.min(limit)];
    if output.json() {
        let position = |offset| {
            let (line, column) = at.line_column(offset);
            json!({"line": line, "column": column})
        };
        let range = json!({
            "start": position(completions.range.start()),
            "end": position(completions.range.end()),
        });
        let items: Vec<serde_json::Value> = (items.iter())
            .map(|item| {
                json!({
                    "label": item.label,
                    "kind": kind(item.kind),
                    "detail": item.detail,
                    "insert": item.insert,
                })
            })
            .collect();
        let mut fields = serde_json::Map::new();
        fields.insert("range".into(), range);
        fields.insert("items".into(), json!(items));
        fields.insert("more".into(), json!(total > items.len()));
        output.object(fields);
    } else {
        let width = (items.iter())
            .map(|item| item.label.chars().count())
            .max()
            .unwrap_or(0);
        for item in items.iter() {
            let pad = " ".repeat(width - item.label.chars().count());
            let detail = item
                .detail
                .as_deref()
                .map(|d| format!("  {d}"))
                .unwrap_or_default();
            println!("{}{pad}  {}{detail}", item.label, kind(item.kind));
        }
        if total > items.len() {
            eprintln!(
                "the first {} of {total} completions: `--limit N` for more",
                items.len()
            );
        }
    }
    Ok(match items.is_empty() {
        true => Status::Failed,
        false => Status::Ok,
    })
}

/// Prints the signature help at `at`, `PATH:LINE:COLUMN`.
pub async fn signature(at: &str, output: &Output) -> Result<Status, String> {
    let at = At::resolve(at).await?;
    let Some(help) = at.signature_help() else {
        if output.json() {
            let mut fields = serde_json::Map::new();
            fields.insert("signatures".into(), json!([]));
            output.object(fields);
        } else {
            eprintln!("no call holds the position");
        }
        return Ok(Status::Failed);
    };
    if output.json() {
        let signatures: Vec<serde_json::Value> = (help.signatures.iter().enumerate())
            .map(|(index, signature)| {
                let parameter = (signature.active_parameter)
                    .and_then(|active| signature.parameters.get(active));
                json!({
                    "label": signature.label,
                    "active": index == help.active_signature,
                    "documentation": signature.documentation,
                    "parameter": parameter.map(|p| &signature.label[p.range.clone()]),
                    "parameter_documentation": parameter.and_then(|p| p.documentation.clone()),
                })
            })
            .collect();
        let mut fields = serde_json::Map::new();
        fields.insert("signatures".into(), json!(signatures));
        output.object(fields);
        return Ok(Status::Ok);
    }
    for (index, signature) in help.signatures.iter().enumerate() {
        let marker = match index == help.active_signature {
            true => "> ",
            false => "  ",
        };
        println!("{marker}{}", signature.label);
    }
    let active = &help.signatures[help.active_signature];
    if let Some(documentation) = &active.documentation {
        println!("\n{documentation}");
    }
    let parameter = (active.active_parameter).and_then(|index| active.parameters.get(index));
    if let Some(parameter) = parameter {
        println!("\n`{}`", &active.label[parameter.range.clone()]);
        if let Some(documentation) = &parameter.documentation {
            println!("{documentation}");
        }
    }
    Ok(Status::Ok)
}

/// The name of a completion's kind.
fn kind(kind: CompletionKind) -> &'static str {
    match kind {
        CompletionKind::Keyword => "keyword",
        CompletionKind::Column => "column",
        CompletionKind::RangeVariable => "range variable",
        CompletionKind::Variable => "variable",
        CompletionKind::Field => "field",
        CompletionKind::Table => "table",
        CompletionKind::Query => "query",
        CompletionKind::Function(FunctionRole::Scalar) => "function",
        CompletionKind::Function(FunctionRole::Aggregate) => "aggregate",
        CompletionKind::Function(FunctionRole::Window) => "window function",
        CompletionKind::Function(FunctionRole::Table) => "table function",
        CompletionKind::Function(_) => "function",
        CompletionKind::Constant => "constant",
        CompletionKind::Parameter => "parameter",
        CompletionKind::EnumValue => "value",
        CompletionKind::Type => "type",
        CompletionKind::Window => "window",
        CompletionKind::Option => "option",
        CompletionKind::File => "file",
        CompletionKind::Folder => "folder",
    }
}
