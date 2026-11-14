//! The calls of `impute` in the text of the tests' queries, `FROM impute(input, key, groups,
//! value, name => value, ...)`, planned through [`impute::plan`]: the adapter splits the call's
//! arguments, plans its input and its constants with DataFusion's SQL, registers the plan of the
//! call as a view, and runs the query with the view in the call's place. An [`ArgError`] becomes
//! a diagnostic at the place of its argument in the query.

use avenger_datafusion_transforms::impute::{self, ImputeCall, Named};
use avenger_datafusion_transforms::table_function::{ArgAt, ArgError};
use datafusion::common::{Column, DFSchema, DataFusionError, Diagnostic, Location, Result, Span};
use datafusion::dataframe::DataFrame;
use datafusion::logical_expr::LogicalPlan;
use datafusion::prelude::SessionContext;

/// A part of the text of a call, and its place, a byte offset in the query.
#[derive(Clone, Debug)]
struct Part {
    text: String,
    at: usize,
}

/// The parts of `text`, at the byte offset `at`, between the commas outside brackets, quotes and
/// parentheses.
fn split(text: &str, at: usize) -> Vec<Part> {
    let mut parts = Vec::new();
    let (mut depth, mut quote, mut start) = (0i32, false, 0);
    for (i, c) in text.char_indices() {
        match c {
            '\'' => quote = !quote,
            '(' | '[' if !quote => depth += 1,
            ')' | ']' if !quote => depth -= 1,
            ',' if !quote && depth == 0 => {
                parts.push(part(&text[start..i], at + start));
                start = i + 1;
            }
            _ => {}
        }
    }
    if !text[start..].trim().is_empty() {
        parts.push(part(&text[start..], at + start));
    }
    parts
}

fn part(text: &str, at: usize) -> Part {
    let lead = text.len() - text.trim_start().len();
    Part {
        text: text.trim().to_string(),
        at: at + lead,
    }
}

/// The elements of a list `[a, b]`, or the part itself without brackets.
fn elements(part: &Part) -> Vec<Part> {
    match part
        .text
        .strip_prefix('[')
        .and_then(|t| t.strip_suffix(']'))
    {
        Some(inner) => split(inner, part.at + 1),
        None => vec![part.clone()],
    }
}

/// An error at the byte offset `at` of `query`, as a diagnostic of its place.
fn error_at(query: &str, at: usize, message: impl Into<String>) -> DataFusionError {
    let message = message.into();
    let before = &query[..at];
    let line = before.matches('\n').count() as u64 + 1;
    let column = (at - before.rfind('\n').map_or(0, |i| i + 1)) as u64 + 1;
    let location = Location { line, column };
    DataFusionError::Diagnostic(
        Box::new(Diagnostic::new_error(
            message.clone(),
            Some(Span::new(location, location)),
        )),
        Box::new(DataFusionError::Plan(message)),
    )
}

/// The column of `schema` that `name` names, without regard to its qualifier.
fn column(schema: &DFSchema, name: &str) -> Option<Column> {
    let (qualifier, field) = schema.qualified_field_with_unqualified_name(name).ok()?;
    Some(Column::new(qualifier.cloned(), field.name()))
}

/// The query `query`, with a call of `impute` in its `FROM` planned through the transforms'
/// planning API.
pub async fn sql(ctx: &SessionContext, query: &str) -> Result<DataFrame> {
    let Some(start) = query.find("impute(") else {
        return ctx.sql(query).await;
    };
    let open = start + "impute(".len();
    let mut depth = 1;
    let mut close = open;
    for (i, c) in query[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            close = open + i;
            break;
        }
    }
    let parts = split(&query[open..close], open);
    let positional: Vec<&Part> = parts.iter().filter(|p| !p.text.contains("=>")).collect();
    let named: Vec<(&str, Part)> = parts
        .iter()
        .filter_map(|p| {
            let (name, value) = p.text.split_once("=>")?;
            let offset = p.text.find("=>").expect("a named argument") + 2;
            let value_at = p.at + offset + (value.len() - value.trim_start().len());
            Some((
                name.trim(),
                Part {
                    text: value.trim().to_string(),
                    at: value_at,
                },
            ))
        })
        .collect();
    if positional.len() != 4 {
        return Err(error_at(
            query,
            start,
            format!(
                "impute takes 4 positional arguments; found {}",
                positional.len()
            ),
        ));
    }
    let input_text = match positional[0].text.strip_prefix('(') {
        Some(_) => positional[0].text.clone(),
        None => format!("(SELECT * FROM {})", positional[0].text),
    };
    let input: LogicalPlan = ctx
        .sql(&format!("SELECT * FROM {input_text}"))
        .await
        .map_err(|e| error_at(query, positional[0].at, e.strip_backtrace()))?
        .logical_plan()
        .clone();
    let schema = input.schema().clone();
    let resolve = |part: &Part, what: &str| -> Result<Column> {
        column(&schema, &part.text).ok_or_else(|| {
            error_at(
                query,
                part.at,
                format!("the {what} of impute must be a column of its input"),
            )
        })
    };
    let key = resolve(positional[1], "key")?;
    let groups = (elements(positional[2]).iter())
        .map(|p| resolve(p, "group"))
        .collect::<Result<Vec<_>>>()?;
    let value = resolve(positional[3], "value column")?;
    let empty = DFSchema::empty();
    let expr = |part: &Part| ctx.parse_sql_expr(&part.text, &empty);
    let mut call_named = Vec::new();
    for (name, part) in &named {
        let arg = match name.to_ascii_lowercase().as_str() {
            "flag" => Named::Name(part.text.clone()),
            "partition_by" => Named::Columns(
                (elements(part).iter())
                    .map(|p| resolve(p, "partition column"))
                    .collect::<Result<Vec<_>>>()?,
            ),
            "frame" | "keys" | "extra_keys" if part.text.starts_with('[') => Named::List(
                (elements(part).iter())
                    .map(expr)
                    .collect::<Result<Vec<_>>>()?,
            ),
            _ => Named::Value(expr(part)?),
        };
        call_named.push((name.to_string(), arg));
    }
    let call = ImputeCall {
        key,
        groups,
        value,
        named: call_named,
    };
    let plan = impute::plan(input, call).map_err(|ArgError { at, message }| {
        let offset = match at {
            ArgAt::Call => start,
            ArgAt::Positional(i) => positional[i + 1].at,
            ArgAt::Element(i, j) => elements(positional[i + 1])[j].at,
            ArgAt::Named(i) => named[i].1.at,
            ArgAt::NamedElement(i, j) => elements(&named[i].1)[j].at,
        };
        error_at(query, offset, message)
    })?;
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let view = format!(
        "__impute_{}",
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    ctx.register_table(&view, DataFrame::new(ctx.state(), plan).into_view())?;
    let rewritten = format!("{}{view}{}", &query[..start], &query[close + 1..]);
    ctx.sql(&rewritten).await
}

/// The index of the first `word` of `text` outside quotes and parentheses.
fn top_level(text: &str, word: &str) -> Option<usize> {
    let (mut depth, mut quote) = (0i32, false);
    for (i, c) in text.char_indices() {
        match c {
            '\'' => quote = !quote,
            '(' | '[' if !quote => depth += 1,
            ')' | ']' if !quote => depth -= 1,
            _ if !quote && depth == 0 && text[i..].starts_with(word) => return Some(i),
            _ => {}
        }
    }
    None
}

/// The text between the parenthesis at `open` and its match, and the index after the match.
fn parenthesized(text: &str, open: usize) -> (&str, usize) {
    let mut depth = 0;
    for (i, c) in text[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            return (&text[open + 1..open + i], open + i + 1);
        }
    }
    panic!("an unclosed parenthesis in {text}")
}

/// The text of a string literal, `'a''b'` as `a'b`.
fn unquote(text: &str) -> String {
    (text.strip_prefix('\'').and_then(|t| t.strip_suffix('\'')))
        .unwrap_or_else(|| panic!("a string literal, not {text}"))
        .replace("''", "'")
}

/// The query `SELECT *, <items> FROM <input>` registered as a view, whose name this returns,
/// where each item `lump(category, name => value, ...) OVER (PARTITION BY ...) AS name` is
/// planned through [`lump::plan`] and the other items through DataFusion's SQL. In the `keep`
/// of a lump, the window `ranked` is written out with the lump's weight and category.
pub async fn lump_view(ctx: &SessionContext, query: &str) -> Result<String> {
    use avenger_datafusion_transforms::lump::{self, Keep, LumpCall, Nulls};
    let rest = query
        .strip_prefix("SELECT *, ")
        .expect("SELECT *, <items> FROM <input>");
    let from = top_level(rest, " FROM ").expect("a FROM");
    let items = split(&rest[..from], 0);
    let mut plan: LogicalPlan = ctx
        .sql(&format!("SELECT *{}", &rest[from..]))
        .await?
        .logical_plan()
        .clone();
    let mut others = Vec::new();
    for item in items {
        if !item.text.starts_with("lump(") {
            others.push(item.text);
            continue;
        }
        let (args, after) = parenthesized(&item.text, "lump".len());
        let tail = item.text[after..].trim();
        let tail = tail.strip_prefix("OVER").expect("an OVER").trim_start();
        let (over, after) = parenthesized(tail, 0);
        let name = tail[after..]
            .trim()
            .strip_prefix("AS ")
            .expect("an AS")
            .trim();
        let parts = split(args, 0);
        let schema = plan.schema().clone();
        let expr = |text: &str| ctx.parse_sql_expr(text, &schema);
        let category_text = parts[0].text.clone();
        let mut weight_text = None;
        let (mut n, mut keep_text, mut other, mut nulls) =
            (None, None, "Other".to_string(), Nulls::Keep);
        for part in &parts[1..] {
            let (key, value) = part.text.split_once("=>").expect("a named argument");
            let value = value.trim();
            match key.trim() {
                "weight" => weight_text = Some(value.to_string()),
                "n" => n = Some(value.parse::<i64>().expect("an integer")),
                "keep" => keep_text = Some(value.to_string()),
                "other" => other = unquote(value),
                "null_category" => nulls = Nulls::parse(&unquote(value)).expect("a mode"),
                key => panic!("an argument {key} of lump"),
            }
        }
        let weight = weight_text.as_deref().map(expr).transpose()?;
        let keep = match (n, keep_text) {
            (Some(n), None) => Keep::First(n),
            (None, Some(text)) => {
                let order = format!(
                    "ORDER BY {} DESC NULLS LAST, {category_text} ASC NULLS LAST",
                    weight_text.as_deref().unwrap_or("count(*)")
                );
                let text = text
                    .replace("OVER ranked", &format!("OVER ({order})"))
                    .replace("OVER (ranked ", &format!("OVER ({order} "));
                Keep::Condition(expr(&text)?)
            }
            _ => panic!("one of n and keep"),
        };
        let partition = match over.trim().strip_prefix("PARTITION BY ") {
            Some(keys) => (split(keys, 0).iter())
                .map(|key| expr(&key.text))
                .collect::<Result<Vec<_>>>()?,
            None if over.trim().is_empty() => Vec::new(),
            None => panic!("a window of PARTITION BY, not {over}"),
        };
        let call = LumpCall {
            category: expr(&category_text)?,
            partition,
            weight,
            keep,
            other,
            null_category: nulls,
        };
        plan = lump::plan(plan, call, Column::from_name(name))?;
    }
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let view = format!(
        "__lump_{}",
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    ctx.register_table(&view, DataFrame::new(ctx.state(), plan).into_view())?;
    if others.is_empty() {
        return Ok(view);
    }
    let outer = format!("{view}_items");
    let df = ctx
        .sql(&format!("SELECT *, {} FROM {view}", others.join(", ")))
        .await?;
    ctx.register_table(&outer, df.into_view())?;
    Ok(outer)
}
