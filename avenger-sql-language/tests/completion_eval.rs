//! An evaluation of completion over 100 example queries: the chart, mark, transform, constructor,
//! and script queries of avenger-sql's tests, and the queries of GoogleSQL's recorded pipe corpus
//! that avenger-sql accepts, in `tests/data/completion_queries.txt`.
//!
//! At up to 10 locations of each query, over the kinds of its tokens, it completes twice: in the
//! whole query, and in the text cut at the cursor. The cursor is at the start of the token or two
//! characters into it. Each probe records whether completion offers the token that the query
//! has, which items cannot follow the text before the cursor, and, in the cut text, which valid
//! items completion leaves out.
//!
//! An oracle judges an item: it inserts the item after the text before the cursor, closes the
//! brackets that the text leaves open, and parses and analyzes the result. The item is invalid
//! when a syntax error before its end, or an error of the analysis at the item, or at the path
//! that a `.` makes of it, says so. An error that the text after the item can fix is not
//! counted: the missing arguments of a call, the type of an operand, or the value of a literal.
//! A function is valid where some call of it is: with an argument of its type for each that it
//! needs, with and without `OVER`, and over an aggregate. A keyword is tried alone, before a `(`,
//! and before a string, and a keyword that starts an expression with expressions that it starts.
//! A keyword or a name that the parser reads as a name that the text defines, such as an alias,
//! is not counted as missed.
//!
//! The oracle cannot judge some places, which the probes count apart: the arguments of `lump`,
//! `impute`, and the marks, which the pass binds itself, the body of a function that the pass
//! plans only at a call, a call of a window that the text does not define or of a window
//! function without its `OVER`, and a `PIVOT` without its `FOR`.
//!
//! The test writes its probes and a summary to `tests/output/completion_eval/`, and fails when a
//! probe does not offer its token, or offers an invalid item, or misses a valid one.

mod common;

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    sync::{Arc, Mutex},
};

use avenger_sql::{
    analysis::Options,
    catalog::Catalog,
    library::{Accept, Arity, Family, Library, Signature, Ty},
};
use avenger_sql_language::{CompletionKind, Document, Highlight, Offset, Source};
use avenger_sql_syntax::ast::{self, AstNode};
use avenger_sql_syntax::NameRole;

struct Env {
    catalog: Arc<dyn Catalog + Send + Sync>,
    library: Arc<Library>,
    keywords: Vec<String>,
    functions: Vec<String>,
    tables: Vec<String>,
    columns: Vec<String>,
}

impl Env {
    fn document(&self, text: &str) -> Document {
        Document::new(
            text,
            self.catalog.clone(),
            self.library.clone(),
            Options::default(),
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Item {
    Name,
    Function,
    Keyword,
}

/// The reason that `insert` cannot follow `prefix`, or `None` when it can.
fn oracle(env: &Env, prefix: &str, insert: &str, item: Item) -> Option<String> {
    match item {
        // A keyword alone, before a `(`, as `CAST (`, or before a string, as a typed literal.
        // Or over an aggregate, as `STRUCT(ANY_VALUE(1))` or `ARRAY[ANY_VALUE(1)]`.
        Item::Keyword => {
            // `OVER` completes the call before it, whose place can forbid a window.
            if insert == "OVER" {
                return over(env, prefix);
            }
            // A keyword that starts an expression is valid where some expression that it starts
            // is, as `NOT TRUE`, and not merely where the parser takes the keyword.
            if starts_expression(env, prefix, insert) {
                let mut first = None;
                for after in EXPRESSIONS {
                    match judge(env, prefix, insert, after, true) {
                        None => return None,
                        Some(reason) => {
                            first.get_or_insert(reason);
                        }
                    }
                }
                return first;
            }
            let bare = judge(env, prefix, insert, "", false);
            bare.as_ref()?;
            judge(env, prefix, insert, " (", false)?;
            judge(env, prefix, insert, " ''", false)?;
            judge(env, prefix, insert, "(ANY_VALUE(1))", true)?;
            judge(env, prefix, insert, "[ANY_VALUE(1)]", true).and(bare)
        }
        // A function is valid where some call of it is: without arguments when a form takes
        // none, and with one otherwise, with and without `OVER`, and over an aggregate, as
        // `ABS(SUM(x))` in `|> AGGREGATE`. A call without arguments of a function that takes some
        // fails on its arguments before its place, so the oracle doesn't try one.
        Item::Function => {
            // A call of each form, with an argument of its type for each that the form needs,
            // so that an error of a type doesn't hide the error of the place, with and without
            // `OVER`, and over an aggregate, as `ABS(ANY_VALUE(1))` in `|> AGGREGATE`.
            let forms = env.library.get(insert).map(|function| &function.forms);
            let mut calls: Vec<String> = Vec::new();
            for form in forms.into_iter().flatten() {
                let signature = &form.signature;
                for base in [call_of(signature, false), call_of(signature, true)] {
                    for over in ["", " OVER ()", " OVER (ORDER BY 1)"] {
                        let call = format!("{base}{over}");
                        if !calls.contains(&call) {
                            calls.push(call);
                        }
                    }
                }
            }
            if calls.is_empty() {
                calls.push("()".to_string());
            }
            let mut first = None;
            for call in calls {
                match judge(env, prefix, insert, &call, true) {
                    None => return None,
                    Some(reason) => {
                        first.get_or_insert(reason);
                    }
                }
            }
            first
        }
        Item::Name => judge(env, prefix, insert, "", false),
    }
}

/// A literal of the type `ty`, whose type variables take the first type of their sets in
/// `vars`.
fn literal(ty: &Ty, vars: &[(String, Vec<Ty>)]) -> String {
    match ty {
        Ty::Float64 => "1.5".to_string(),
        Ty::Numeric => "NUMERIC '1'".to_string(),
        Ty::BigNumeric => "BIGNUMERIC '1'".to_string(),
        Ty::Bool => "TRUE".to_string(),
        Ty::String => "'a'".to_string(),
        Ty::Bytes => "b'a'".to_string(),
        Ty::Date => "DATE '2024-01-01'".to_string(),
        Ty::Time => "TIME '01:02:03'".to_string(),
        Ty::Datetime => "DATETIME '2024-01-01 00:00:00'".to_string(),
        Ty::Timestamp => "TIMESTAMP '2024-01-01 00:00:00+00'".to_string(),
        Ty::Interval => "INTERVAL 1 DAY".to_string(),
        Ty::Array(element) => format!("[{}]", literal(element, vars)),
        Ty::Struct(fields) => {
            let fields: Vec<String> = (fields.iter())
                .map(|(name, ty)| format!("{} AS {name}", literal(ty, vars)))
                .collect();
            format!("STRUCT({})", fields.join(", "))
        }
        Ty::Var(name) => match (vars.iter()).find(|(var, _)| var == name) {
            Some((_, set)) if !set.is_empty() => literal(&set[0], vars),
            _ => "1".to_string(),
        },
        Ty::Int64 | Ty::Any | Ty::Null | Ty::Table => "1".to_string(),
    }
}

/// The arguments of a call of a form of `signature`: one of its type for each argument that it
/// needs. When `wrapped` holds, its first value is an aggregate's, as `ANY_VALUE(1)`, of an
/// optional argument when it needs none, as `axis(title => ANY_VALUE('a'))`.
fn call_of(signature: &Signature, wrapped: bool) -> String {
    let needs_value = (signature.args.iter())
        .any(|arg| arg.arity != Arity::Optional && matches!(arg.accept, Accept::Value(_)));
    let mut first = true;
    let mut values: Vec<String> = Vec::new();
    for arg in &signature.args {
        let value_arg = matches!(arg.accept, Accept::Value(_));
        let wrap = wrapped && first && value_arg;
        let needed = arg.arity != Arity::Optional;
        // An optional positional argument after a missing one goes by its name.
        let optional = wrap && !needs_value;
        if !needed && !optional {
            continue;
        }
        let named = arg.named_only || values.len() != positional(signature, arg);
        let mut value = match &arg.accept {
            Accept::Value(ty) | Accept::Const(ty) => literal(ty, &signature.vars),
            Accept::OneOf(values) => format!("'{}'", values[0]),
            Accept::Part(_) => "DAY".to_string(),
        };
        if wrap {
            value = format!("ANY_VALUE({value})");
            first = false;
        }
        values.push(match named {
            true => format!("{} => {value}", arg.param),
            false => value,
        });
    }
    format!("({})", values.join(", "))
}

/// The number of positional arguments of `signature` before `arg`.
fn positional(signature: &Signature, arg: &avenger_sql::library::FormArg) -> usize {
    (signature.args.iter())
        .take_while(|other| !std::ptr::eq(*other, arg))
        .filter(|other| !other.named_only)
        .count()
}

/// The reason that `insert` and `after` cannot follow `prefix`, or `None`. An error inside
/// `after` counts when `whole` holds, as of a call that the oracle writes whole, and not
/// otherwise, as of a `(` that only tries what a keyword can start.
fn judge(env: &Env, prefix: &str, insert: &str, after: &str, whole: bool) -> Option<String> {
    let mut text = format!("{prefix}{insert}");
    let end = prefix.len() + insert.trim_end().len() + if whole { after.len() } else { 0 };
    text.push_str(after);
    // Close the open brackets, outside strings and quoted names.
    let mut stack = Vec::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in text.chars() {
        if let Some(q) = quote {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, c) if c == q => quote = None,
                _ => {}
            }
            continue;
        }
        match c {
            '\'' | '"' | '`' => quote = Some(c),
            '(' => stack.push(')'),
            '[' => stack.push(']'),
            ')' | ']' => {
                stack.pop();
            }
            _ => {}
        }
    }
    if quote.is_none() {
        text.push(' ');
        text.extend(stack.iter().rev());
    }
    let document = env.document(&text);
    if let Some(error) = (document.diagnostics().into_iter())
        .find(|d| d.source == Source::Syntax && usize::from(d.range.start()) < end)
    {
        return Some(format!("syntax: {}", error.message));
    }
    // An error at the insert, or at the path that a `.` makes of it, as `p.COUNT`.
    let start = prefix
        .trim_end_matches(|c: char| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        .len();
    (document.analysis().diagnostics.iter())
        .find(|d| {
            let s = usize::from(d.range.start());
            s >= start && s < end.max(prefix.len() + 1) && !soft(&d.message)
        })
        .map(|d| format!("semantic: {}", d.message))
}

/// The texts after a keyword that complete an expression that it starts, of each keyword, and
/// over an aggregate for `|> AGGREGATE`.
const EXPRESSIONS: &[&str] = &[
    " TRUE",
    " ANY_VALUE(TRUE)",
    " 1 DAY",
    " ANY_VALUE(1) DAY",
    " (FROM one)",
    " ''",
    "(1 AS INT64)",
    "(ANY_VALUE(1) AS INT64)",
    "(DAY FROM CURRENT_DATE())",
    "(DAY FROM ANY_VALUE(CURRENT_DATE()))",
    " WHEN TRUE THEN 1 END",
    " WHEN ANY_VALUE(TRUE) THEN 1 END",
    "(ANY_VALUE(1))",
    "[ANY_VALUE(1)]",
];

/// Whether `keyword` after `prefix` starts an expression: the parser expects one after it.
fn starts_expression(env: &Env, prefix: &str, keyword: &str) -> bool {
    let text = format!("{prefix}{keyword}");
    (env.document(&text).diagnostics().into_iter())
        .find(|d| d.source == Source::Syntax && usize::from(d.range.start()) >= text.len())
        .is_some_and(|d| d.message.starts_with("expected an expression"))
}

/// The reason that `OVER` cannot follow `prefix`, or `None`: an error of `OVER ()` or of
/// `OVER (ORDER BY 1)` at the call that `prefix` ends with.
fn over(env: &Env, prefix: &str) -> Option<String> {
    let trimmed = prefix.trim_end();
    if !trimmed.ends_with(')') {
        return Some("no call before OVER".to_string());
    }
    // The start of the call: its `(` and the name before it.
    let mut depth = 0;
    let mut open = 0;
    for (index, c) in trimmed.char_indices().rev() {
        match c {
            ')' => depth += 1,
            '(' => {
                depth -= 1;
                if depth == 0 {
                    open = index;
                    break;
                }
            }
            _ => {}
        }
    }
    let start = trimmed[..open]
        .trim_end_matches(|c: char| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        .len();
    let mut first = None;
    for after in ["OVER ()", "OVER (ORDER BY 1)"] {
        let text = format!("{prefix}{after}");
        let end = text.len();
        let document = env.document(&text);
        let error = (document.diagnostics().into_iter())
            .find(|d| usize::from(d.range.start()) >= start && usize::from(d.range.start()) <= end)
            .filter(|d| d.source == Source::Syntax || !soft(&d.message));
        match error {
            None => return None,
            Some(error) => {
                first.get_or_insert(error.message);
            }
        }
    }
    first
}

/// Whether the parser reads `keyword` after `prefix` as a keyword, and not as a name, such as
/// an alias or a window, which completion offers as the names that the text defines.
fn read_as_keyword(env: &Env, prefix: &str, keyword: &str) -> bool {
    let document = env.document(&format!("{prefix}{keyword}"));
    let start = prefix.len();
    (document.highlights().into_iter())
        .find(|h| usize::from(h.range.start()) == start)
        .is_none_or(|h| h.highlight == Highlight::Keyword)
}

/// Whether an error of the analysis is one that the rest of the text can fix: the arguments of a
/// call, the type of an operand, or a window function's `OVER`.
fn soft(message: &str) -> bool {
    // A call that its place forbids, as an analytic function in an aggregate's argument, or
    // with more positional arguments than its function takes.
    if message.contains("not allowed") || message.contains("positional argument") {
        return false;
    }
    [
        "signature",
        "argument",
        " takes ",
        "should return type",
        "requires at least",
        "must be type",
        "incompatible",
        "must be arrays",
        "unnest plan",
        "expects",
        "mismatched",
        "needs a value",
        "Expected type",
        "literal:",
        // What the text after the cursor gives: the query after `CORRESPONDING BY (...)`, a
        // window that a `WINDOW` defines, or an alias of a table.
        "is not a column of its queries",
        "Unrecognized window alias",
        "Duplicate table alias",
    ]
    .iter()
    .any(|pattern| message.contains(pattern))
}

/// The first word of a label, in upper case and without backticks.
fn norm(label: &str) -> String {
    let word = label
        .trim_matches('`')
        .split_whitespace()
        .next()
        .unwrap_or("");
    word.trim_matches('`').to_ascii_uppercase()
}

struct Location {
    start: usize,
    end: usize,
    category: String,
}

fn category(highlight: Highlight, definition: bool, after: &str) -> Option<String> {
    let base = match highlight {
        Highlight::Keyword if after == "|>" => "pipe-operator".to_string(),
        Highlight::Keyword => "keyword".to_string(),
        Highlight::EnumValue => "enum-value".to_string(),
        Highlight::Name(_) if definition => "definition".to_string(),
        Highlight::Name(NameRole::Function(role)) => format!("function-{role:?}"),
        Highlight::Name(role) => format!("{role:?}"),
        _ => return None,
    };
    Some(match after {
        "." => format!("after-dot-{base}"),
        _ => base,
    })
}

/// Up to 10 locations of the document, round-robin over the categories of its tokens.
fn locations(document: &Document) -> Vec<Location> {
    let text = document.text();
    let mut by_category: BTreeMap<String, Vec<Location>> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    for range in document.highlights() {
        let start = usize::from(range.range.start());
        let before = text[..start].trim_end();
        let after = match () {
            _ if before.ends_with("|>") => "|>",
            _ if before.ends_with('.') => ".",
            _ => "",
        };
        let Some(category) = category(range.highlight, range.definition, after) else {
            continue;
        };
        if !order.contains(&category) {
            order.push(category.clone());
        }
        by_category
            .entry(category.clone())
            .or_default()
            .push(Location {
                start,
                end: usize::from(range.range.end()),
                category,
            });
    }
    let mut chosen = Vec::new();
    let mut round = 0;
    while chosen.len() < 10 {
        let mut added = false;
        for category in &order {
            let list = &by_category[category];
            let picks = list.len().min(1 + list.len() / 3);
            if round < picks && chosen.len() < 10 {
                let l = &list[round * list.len() / picks];
                chosen.push(Location {
                    start: l.start,
                    end: l.end,
                    category: l.category.clone(),
                });
                added = true;
            }
        }
        if !added {
            break;
        }
        round += 1;
    }
    chosen.sort_by_key(|l| l.start);
    chosen
}

struct Probe {
    query: usize,
    mode: &'static str,
    /// Whether the text before the cursor can tell the token: not a name that the text defines
    /// after the cursor, which the cut text lacks, nor a column of a table parameter.
    knowable: bool,
    category: String,
    context: String,
    expected: String,
    offered: bool,
    judged: bool,
    items: usize,
    invalid: Vec<(String, String)>,
    missed: Vec<String>,
}

fn context(text: &str, cursor: usize) -> String {
    let mut a = cursor.saturating_sub(40);
    while !text.is_char_boundary(a) {
        a -= 1;
    }
    let mut b = (cursor + 20).min(text.len());
    while !text.is_char_boundary(b) {
        b += 1;
    }
    format!("{}‸{}", &text[a..cursor], &text[cursor..b]).replace('\n', "⏎")
}

/// Whether the oracle can judge the items at `offset` of `document`: not in the arguments of a
/// call that the pass binds itself, or of a mark, whose errors of its arguments hide the others,
/// not in the body of a function of a parameter of any type or a table, which the pass plans
/// only at a call, not after a window that the text does not define or in a window function
/// without its `OVER`, and not in a `PIVOT` without its `FOR`.
fn judged(env: &Env, document: &Document, offset: usize) -> bool {
    let parse = avenger_sql_syntax::parse(document.text());
    // Whether `node` holds `offset`, or ends before it, open, with only spaces between.
    let holds = |node: &avenger_sql_syntax::SyntaxNode| {
        let range = node.text_range();
        let (start, end) = (usize::from(range.start()), usize::from(range.end()));
        let open = !node.text().to_string().ends_with(')');
        start < offset
            && (offset <= end || (open && document.text()[end..offset].trim().is_empty()))
    };
    let at_call = (parse.syntax().descendants())
        .filter_map(ast::CreateFunctionStmt::cast)
        .filter(|create| (create.function_body()).is_some_and(|body| holds(body.syntax())))
        .any(|create| {
            (create.param_list().iter())
                .flat_map(|list| list.parameters())
                .any(|parameter| !matches!(parameter.param_type(), Some(ast::ParamType::Type(_))))
        });
    // A window that the text does not define stops the plan of the call that names it, and a
    // window function without its `OVER`, which the text after the cursor gives, has no plan of
    // its arguments.
    let undefined_window = (document.analysis().diagnostics.iter()).any(|d| {
        let (start, end) = (usize::from(d.range.start()), usize::from(d.range.end()));
        (d.message.starts_with("Unrecognized window alias") && start < offset)
            || (d
                .message
                .contains("cannot be called without an OVER clause")
                && start < offset
                && offset <= end)
    });
    // A `PIVOT` without its `FOR` has no plan, and so no errors of its expressions.
    let open_pivot = (parse.syntax().descendants())
        .filter_map(ast::PivotClause::cast)
        .any(|clause| clause.column().is_none() && holds(clause.syntax()));
    if at_call || undefined_window || open_pivot {
        return false;
    }
    let text = &document.text()[..offset];
    let mut depth = 0i32;
    for (index, c) in text.char_indices().rev() {
        match c {
            ')' => depth += 1,
            '(' if depth > 0 => depth -= 1,
            '(' => {
                let name = text[..index].trim_end();
                let word: String = (name.chars().rev())
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                let mark = (env.library.get(&word)).is_some_and(|f| f.family == Family::Mark);
                if mark || matches!(word.to_ascii_lowercase().as_str(), "lump" | "impute") {
                    return false;
                }
            }
            _ => {}
        }
    }
    true
}

/// Whether completion can know the token of `location` at `cursor` of `text` in `mode`: not a name
/// that a definition after the cursor gives, in the cut text, which lacks it, and not a column of
/// a table function's `ANY TABLE` parameter, whose columns only a call knows.
fn knowable(text: &str, location: &Location, cursor: usize, mode: &str) -> bool {
    let token = text[location.start..location.end].to_ascii_lowercase();
    let parse = avenger_sql_syntax::parse(text);
    let defined_after = (avenger_sql_syntax::highlight(&parse.syntax()).iter())
        .filter(|h| h.definition && usize::from(h.range.start()) >= cursor)
        .any(|h| text[std::ops::Range::<usize>::from(h.range)].eq_ignore_ascii_case(&token));
    if mode == "cut" && defined_after {
        return false;
    }
    let root = parse.syntax();
    let token_at = root
        .token_at_offset((location.start as u32).into())
        .right_biased();
    let table_function = (token_at.iter())
        .flat_map(|token| token.parent_ancestors())
        .find_map(ast::CreateFunctionStmt::cast)
        .filter(|create| create.function_kind() == ast::FunctionKind::Table);
    let reads_a_table_parameter = table_function.is_some_and(|create| {
        (create.param_list().iter())
            .flat_map(|list| list.parameters())
            .any(|p| matches!(p.param_type(), Some(ast::ParamType::AnyTable(_))))
    });
    !(reads_a_table_parameter && location.category == "Column")
}

fn probe(env: &Env, query: usize, text: &str, location: &Location, index: usize) -> Vec<Probe> {
    let token = &text[location.start..location.end];
    let enum_value = location.category.ends_with("enum-value");
    let skip = usize::from(enum_value);
    let typed = match index % 2 {
        0 => 0,
        _ => 2.min(token.len().saturating_sub(1 + skip)),
    };
    let mut cursor = location.start + skip + typed;
    while !text.is_char_boundary(cursor) {
        cursor += 1;
    }
    let mut probes = Vec::new();
    for (mode, mode_text) in [("valid", text), ("cut", &text[..cursor])] {
        let document = env.document(mode_text);
        let completions = document.completions(Offset::from(cursor as u32));
        let prefix = &mode_text[..usize::from(completions.range.start())];
        let offered: BTreeSet<String> = completions.items.iter().map(|i| norm(&i.label)).collect();
        let expected = match enum_value {
            true => token.to_string(),
            false => norm(token),
        };
        let found = match enum_value {
            true => completions.items.iter().any(|i| i.label == token),
            false => offered.contains(&expected),
        };
        let knowable = knowable(text, location, cursor, mode);
        let judged = location.category != "definition" && judged(env, &document, cursor);
        let mut invalid = Vec::new();
        let mut missed = Vec::new();
        if judged {
            for item in &completions.items {
                let kind = match item.kind {
                    CompletionKind::Function(_) => Item::Function,
                    CompletionKind::Keyword => Item::Keyword,
                    _ => Item::Name,
                };
                if let Some(reason) = oracle(env, prefix, &item.insert, kind) {
                    invalid.push((item.label.clone(), reason));
                }
            }
            if mode == "cut" && !enum_value {
                let names = defined_names(&document);
                // A keyword is no field after a `.`, and a word that names a function is judged
                // as the function.
                let after_dot = prefix.trim_end().ends_with('.');
                let keywords = (env.keywords.iter()).filter(|k| {
                    !after_dot && env.library.get(k).is_none() && read_as_keyword(env, prefix, k)
                });
                let functions = env.functions.iter();
                // Where the analysis takes any name, as an alias or a window that a later
                // `WINDOW` can define, no name is one that completion can know.
                let any_name = oracle(env, prefix, "zz_unknown_name", Item::Name).is_none();
                let names =
                    (env.tables.iter().chain(&env.columns).chain(&names)).filter(|_| !any_name);
                let candidates = (keywords.map(|k| (k, Item::Keyword)))
                    .chain(functions.map(|f| (f, Item::Function)))
                    .chain(names.map(|n| (n, Item::Name)));
                for (candidate, kind) in candidates {
                    if !offered.contains(&norm(candidate))
                        && oracle(env, prefix, candidate, kind).is_none()
                    {
                        missed.push(candidate.clone());
                    }
                }
            }
        }
        probes.push(Probe {
            query,
            mode,
            category: location.category.clone(),
            context: context(mode_text, cursor),
            expected,
            offered: found,
            knowable,
            judged,
            items: completions.items.len(),
            invalid,
            missed,
        });
    }
    probes
}

/// The names that a script defines: aliases, `WITH` names, windows, and variables.
fn defined_names(document: &Document) -> Vec<String> {
    let text = document.text();
    let mut names: Vec<String> = (document.highlights().into_iter())
        .filter(|h| h.definition)
        .map(|h| text[std::ops::Range::<usize>::from(h.range)].to_string())
        .collect();
    names.sort();
    names.dedup();
    names
}

#[test]
fn completion_offers_only_and_all_valid_items_over_the_example_queries() {
    let library = Library::new();
    let grammar = include_str!("../../avenger-sql-syntax/sql.ungram");
    let keywords: BTreeSet<String> = (grammar.split('\'').skip(1).step_by(2))
        .filter(|part| {
            !part.is_empty() && part.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
        })
        .map(str::to_ascii_uppercase)
        .collect();
    let catalog = common::examples();
    let mut columns: BTreeSet<String> = BTreeSet::new();
    let mut tables = Vec::new();
    for table in catalog.tables() {
        tables.push(table.names.join("."));
        columns.extend(
            table
                .schema()
                .fields()
                .iter()
                .map(|field| field.name().clone()),
        );
    }
    let env = Env {
        functions: library.functions().map(|f| f.name.clone()).collect(),
        catalog: Arc::new(catalog),
        library: Arc::new(library),
        keywords: keywords.into_iter().collect(),
        tables,
        columns: columns.into_iter().collect(),
    };
    let data = include_str!("data/completion_queries.txt");
    let queries: Vec<&str> = (data.split('\u{1e}'))
        .map(|record| record.trim_start_matches('\n'))
        .filter(|record| !record.trim().is_empty())
        .map(|record| {
            let record = (record.lines())
                .skip_while(|line| line.starts_with('#'))
                .collect::<Vec<_>>()
                .join("\n");
            let (_, text) = record.split_once('\n').unwrap();
            Box::leak(text.to_string().into_boxed_str()) as &str
        })
        .collect();
    assert_eq!(queries.len(), 100);
    for (index, text) in queries.iter().enumerate() {
        let diagnostics = env.document(text).diagnostics();
        assert!(diagnostics.is_empty(), "query {index}: {diagnostics:?}");
    }

    let probes = Mutex::new(Vec::new());
    let next = Mutex::new(0usize);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| loop {
                let index = {
                    let mut next = next.lock().unwrap();
                    *next += 1;
                    *next - 1
                };
                let Some(text) = queries.get(index) else {
                    break;
                };
                let document = env.document(text);
                let found: Vec<Probe> = (locations(&document).iter().enumerate())
                    .flat_map(|(i, location)| probe(&env, index, text, location, i))
                    .collect();
                probes.lock().unwrap().extend(found);
            });
        }
    });
    let mut probes = probes.into_inner().unwrap();
    probes.sort_by(|a, b| (a.query, &a.context, a.mode).cmp(&(b.query, &b.context, b.mode)));

    let mut lines = String::new();
    for p in &probes {
        let invalid: Vec<String> = p
            .invalid
            .iter()
            .map(|(l, r)| format!("{l} [{r}]"))
            .collect();
        writeln!(
            lines,
            "q{} {} {} | {} | expected {} offered={} judged={} items={} | invalid {:?} | missed {:?}",
            p.query, p.mode, p.category, p.context, p.expected, p.offered, p.judged, p.items,
            invalid, p.missed
        )
        .unwrap();
    }
    let counted: Vec<&Probe> = (probes.iter())
        .filter(|p| p.category != "definition" && p.knowable)
        .collect();
    let unknowable = probes.iter().filter(|p| !p.knowable).count();
    let offered = |mode: &str| {
        counted
            .iter()
            .filter(|p| p.mode == mode && p.offered)
            .count()
    };
    let total = |mode: &str| counted.iter().filter(|p| p.mode == mode).count();
    let invalid: usize = probes.iter().map(|p| p.invalid.len()).sum();
    let missed: usize = probes.iter().map(|p| p.missed.len()).sum();
    let unjudged = probes
        .iter()
        .filter(|p| !p.judged && p.category != "definition")
        .count();
    let summary = format!(
        "{} probes over {} queries, {unjudged} that the oracle cannot judge, {unknowable} whose \
         token the text cannot tell\n\
         token offered: {} of {} in the whole query, {} of {} in the cut text\n\
         invalid items: {invalid}\nmissed items: {missed}\n",
        probes.len(),
        queries.len(),
        offered("valid"),
        total("valid"),
        offered("cut"),
        total("cut"),
    );
    let out = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/output/completion_eval");
    std::fs::create_dir_all(out).unwrap();
    std::fs::write(format!("{out}/probes.txt"), &lines).unwrap();
    std::fs::write(format!("{out}/summary.txt"), &summary).unwrap();
    println!("{summary}");
    assert!(
        offered("valid") == total("valid") && offered("cut") == total("cut"),
        "a probe does not offer its token: {summary}see {out}/probes.txt"
    );
    assert_eq!(invalid, 0, "invalid items: {summary}see {out}/probes.txt");
    assert_eq!(missed, 0, "missed items: {summary}see {out}/probes.txt");
}
