//! The forms and the docs of the library's functions: every function is documented, its examples
//! return their results and GoogleSQL's, its forms are forms of GoogleSQL, resolution picks
//! GoogleSQL's form, and the reference of every function matches its baseline.

mod eval;

use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use avenger_sql::library::{reference, Accept, Arity, Form, FunctionKind, Library, Ty};
use eval::*;

/// The functions of avenger, whose forms GoogleSQL doesn't have.
fn is_avenger(name: &str) -> bool {
    name.chars().any(|c| c.is_ascii_lowercase())
}

#[test]
fn every_function_is_documented() {
    let library = Library::new();
    let mut missing = Vec::new();
    for function in library.functions() {
        let name = &function.name;
        if function.summary.trim().is_empty() {
            missing.push(format!("{name}: no summary"));
        }
        if function.forms.is_empty() {
            missing.push(format!("{name}: no form"));
        }
        let used: BTreeSet<String> = function
            .forms
            .iter()
            .flat_map(|form| {
                form.signature
                    .args
                    .iter()
                    .map(|arg| arg.param.to_ascii_lowercase())
            })
            .collect();
        for param in &function.params {
            if param.doc.trim().is_empty() {
                missing.push(format!("{name}({}): no doc", param.name));
            }
            if !used.contains(&param.name.to_ascii_lowercase()) {
                missing.push(format!("{name}({}): in no form", param.name));
            }
        }
        for form in &function.forms {
            for arg in &form.signature.args {
                if function.param_named(&arg.param).is_none() {
                    missing.push(format!(
                        "{name}({}): a form's argument without a parameter",
                        arg.param
                    ));
                }
            }
        }
    }
    assert!(missing.is_empty(), "\n{}", missing.join("\n"));
}

#[tokio::test]
async fn examples_return_their_results_and_googlesqls() {
    let library = Library::new();
    let googlesql: BTreeMap<String, String> = include_str!("data/googlesql_examples.tsv")
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(expr, value)| (expr.to_string(), value.to_string()))
        .collect();
    let mut failures = Vec::new();
    let mut checked = 0;
    for function in library.functions() {
        if function.kind != FunctionKind::Scalar || is_avenger(&function.name) {
            continue;
        }
        for example in &function.examples {
            let planned = match plan(&library, &parse(&example.query)) {
                Ok(planned) => planned,
                Err(error) => {
                    failures.push(format!("{}: {error}", example.query));
                    continue;
                }
            };
            let array = match tokio::spawn(run(planned)).await {
                Ok(array) => array,
                Err(error) => {
                    failures.push(format!("{}: a failed run: {error}", example.query));
                    continue;
                }
            };
            let found = literal(&array, 0);
            if found != example.result {
                failures.push(format!(
                    "{}: the docs say {}, avenger-sql {found}",
                    example.query, example.result
                ));
            }
            match googlesql.get(&example.query) {
                Some(value) if *value != json(&array, 0) => failures.push(format!(
                    "{}: GoogleSQL {value}, avenger-sql {}",
                    example.query,
                    json(&array, 0)
                )),
                Some(_) => {}
                None => failures.push(format!("{}: no recorded value of GoogleSQL", example.query)),
            }
            checked += 1;
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    assert!(checked > 100, "{checked} examples");
}

/// The concrete argument lists of a form of GoogleSQL's listing, as `[STRING, INT64]`, with an
/// optional argument present and absent, a repeated argument as `X...`, and `DOUBLE` as
/// `FLOAT64`, or none for a form of a type or an argument that avenger-sql doesn't have.
fn listed(form: &str) -> Option<Vec<Vec<String>>> {
    let open = form.find('(')?;
    let inner = &form[open + 1..form.rfind(')')?];
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0, 0);
    for (i, c) in inner.char_indices() {
        match c {
            '[' | '<' => depth += 1,
            ']' | '>' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(inner[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    if !inner.trim().is_empty() {
        parts.push(inner[start..].trim().to_string());
    }
    let unsupported = ["INT32", "UINT32", "UINT64", "FLOAT", "ROUNDING_MODE", "=>"];
    let mut required = Vec::new();
    let mut optional = Vec::new();
    for part in parts {
        let tokens: Vec<&str> = part
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '=' && c != '>')
            .collect();
        if tokens.iter().any(|token| unsupported.contains(token)) || part.contains("=>") {
            return None;
        }
        let part = part.replace("DOUBLE", "FLOAT64");
        if let Some(repeated) = part
            .strip_prefix('[')
            .and_then(|p| p.strip_suffix(", ...]"))
        {
            // `X, [X, ...]` and `[X, ...]` are one repeated argument.
            if required.last() == Some(&repeated.to_string()) {
                required.pop();
            }
            required.push(format!("{repeated}..."));
        } else if let Some(inner) = part.strip_prefix('[').and_then(|p| p.strip_suffix(']')) {
            optional.push(inner.to_string());
        } else {
            required.push(part);
        }
    }
    Some(
        (0..=optional.len())
            .map(|count| {
                required
                    .iter()
                    .cloned()
                    .chain(optional[..count].iter().cloned())
                    .collect()
            })
            .collect(),
    )
}

/// The concrete argument lists of a form of the library, in the terms of GoogleSQL's listing.
fn ours(name: &str, form: &Form) -> Vec<Vec<String>> {
    let signature = &form.signature;
    let mut assignments: Vec<BTreeMap<String, String>> = vec![BTreeMap::new()];
    for (var, set) in &signature.vars {
        let types: Vec<String> = match set.is_empty() {
            true => vec!["T1".to_string()],
            false => set.iter().map(Ty::to_string).collect(),
        };
        assignments = assignments
            .into_iter()
            .flat_map(|assignment| {
                types.iter().map(move |ty| {
                    let mut next = assignment.clone();
                    next.insert(var.clone(), ty.clone());
                    next
                })
            })
            .collect();
    }
    let shift = name.ends_with("_ADD") || name.ends_with("_SUB");
    let mut lists = Vec::new();
    for vars in assignments {
        let show = |ty: &Ty| {
            let mut text = ty.to_string();
            for (var, value) in &vars {
                text = text.replace(&format!("<{var}>"), &format!("<{value}>"));
                if text == *var {
                    text = value.clone();
                }
            }
            match (shift, text.as_str()) {
                (true, "INTERVAL") => "INTERVAL INT64 DATE_TIME_PART".to_string(),
                _ => text,
            }
        };
        let mut required = Vec::new();
        let mut optional = Vec::new();
        for arg in &signature.args {
            let text = match &arg.accept {
                Accept::Value(ty) | Accept::Const(ty) => show(ty),
                Accept::OneOf(_) => "STRING".to_string(),
                Accept::Part(_) => "DATE_TIME_PART".to_string(),
            };
            match arg.arity {
                Arity::Required => required.push(text),
                Arity::Optional => optional.push(text),
                Arity::Repeated => required.push(format!("{text}...")),
            }
        }
        for count in 0..=optional.len() {
            lists.push(
                required
                    .iter()
                    .cloned()
                    .chain(optional[..count].iter().cloned())
                    .collect(),
            );
        }
    }
    lists
}

/// The forms of GoogleSQL that the library doesn't map, by function, and why.
const OMITTED: &[(&str, &str, &str)] = &[
    ("SUM", "INTERVAL", "DataFusion's sum doesn't add intervals"),
    (
        "AVG",
        "INTERVAL",
        "DataFusion's avg doesn't average intervals",
    ),
    (
        "STRING_AGG",
        "BYTES",
        "DataFusion's string_agg joins strings",
    ),
    (
        "GENERATE_ARRAY",
        "NUMERIC",
        "DataFusion's range generates integers",
    ),
    (
        "GENERATE_ARRAY",
        "BIGNUMERIC",
        "DataFusion's range generates integers",
    ),
    (
        "GENERATE_ARRAY",
        "FLOAT64",
        "DataFusion's range generates integers",
    ),
    ("LENGTH", "BYTES", "DataFusion's octet_length takes strings"),
    (
        "LOWER",
        "BYTES",
        "the string functions of DataFusion take strings",
    ),
    (
        "UPPER",
        "BYTES",
        "the string functions of DataFusion take strings",
    ),
    (
        "SUBSTR",
        "BYTES",
        "the string functions of DataFusion take strings",
    ),
    (
        "SUBSTRING",
        "BYTES",
        "the string functions of DataFusion take strings",
    ),
    (
        "TRIM",
        "BYTES",
        "the string functions of DataFusion take strings",
    ),
    (
        "LTRIM",
        "BYTES",
        "the string functions of DataFusion take strings",
    ),
    (
        "RTRIM",
        "BYTES",
        "the string functions of DataFusion take strings",
    ),
    (
        "REPLACE",
        "BYTES",
        "the string functions of DataFusion take strings",
    ),
    (
        "STARTS_WITH",
        "BYTES",
        "the string functions of DataFusion take strings",
    ),
    (
        "ENDS_WITH",
        "BYTES",
        "the string functions of DataFusion take strings",
    ),
    (
        "STRPOS",
        "BYTES",
        "the string functions of DataFusion take strings",
    ),
    (
        "SPLIT",
        "BYTES",
        "the string functions of DataFusion take strings",
    ),
    (
        "REGEXP_CONTAINS",
        "BYTES",
        "the regular expressions of DataFusion take strings",
    ),
    (
        "REGEXP_EXTRACT",
        "BYTES",
        "the regular expressions of DataFusion take strings",
    ),
    (
        "REGEXP_REPLACE",
        "BYTES",
        "the regular expressions of DataFusion take strings",
    ),
    (
        "REGEXP_EXTRACT",
        "INT64",
        "the position and the occurrence of a match have no mapping",
    ),
];

#[test]
fn the_forms_are_googlesqls() {
    let library = Library::new();
    let mut listing: BTreeMap<String, BTreeSet<Vec<String>>> = BTreeMap::new();
    for line in include_str!("data/googlesql_forms.txt").lines() {
        let Some((name, form)) = line.split_once('\t') else {
            continue;
        };
        for list in listed(form).into_iter().flatten() {
            listing.entry(name.to_string()).or_default().insert(list);
        }
    }
    let mut failures = Vec::new();
    for function in library.functions() {
        if is_avenger(&function.name) {
            continue;
        }
        let theirs = listing.remove(&function.name).unwrap_or_default();
        let mut mine = BTreeSet::new();
        for form in &function.forms {
            for list in ours(&function.name, form) {
                if !theirs.contains(&list) {
                    failures.push(format!(
                        "{}({}): not a form of GoogleSQL",
                        function.name,
                        list.join(", ")
                    ));
                }
                mine.insert(list);
            }
        }
        for list in theirs.difference(&mine) {
            let omitted = OMITTED.iter().any(|(name, ty, _)| {
                *name == function.name && list.iter().any(|arg| arg.contains(ty))
            });
            if !omitted {
                failures.push(format!(
                    "{}({}): a form of GoogleSQL without a mapping or an omission",
                    function.name,
                    list.join(", ")
                ));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[tokio::test]
async fn resolution_picks_googlesqls_form() {
    // GoogleSQL's types of these calls, from `execute_query --mode=analyze`.
    let cases = [
        ("SQRT(4)", "FLOAT64"),
        ("ROUND(4)", "FLOAT64"),
        ("ROUND(NUMERIC '4.5', 1)", "NUMERIC"),
        ("MOD(NUMERIC '4', 2)", "NUMERIC"),
        ("DATE_TRUNC('2024-05-17', MONTH)", "DATE"),
        ("DATETIME_TRUNC(DATE '2024-01-01', DAY)", "DATETIME"),
        (
            "DATE_DIFF(DATE '2024-01-02', DATETIME '2024-01-01 00:00:00', DAY)",
            "INT64",
        ),
        ("IF(TRUE, 1, 2.5)", "FLOAT64"),
        ("GREATEST(1, NUMERIC '2')", "NUMERIC"),
        ("FORMAT_DATE('%Y', '2024-01-01')", "STRING"),
        ("IF(TRUE, NULL, NULL)", "INT64"),
    ];
    let library = Library::new();
    let mut failures = Vec::new();
    for (text, expected) in cases {
        let array = run(expr(&library, text)).await;
        let found = avenger_sql::types::googlesql_name(array.data_type());
        if found != expected {
            failures.push(format!("{text}: GoogleSQL {expected}, avenger-sql {found}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn the_reference_of_every_function() {
    let text = reference(&Library::new());
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let baseline = manifest.join("tests/baselines/functions.md");
    let expected = std::fs::read_to_string(&baseline).unwrap_or_default();
    if expected != text {
        let output = manifest.join("tests/output");
        std::fs::create_dir_all(&output).unwrap();
        let written = output.join("functions.md");
        std::fs::write(&written, &text).unwrap();
        panic!(
            "the reference differs from {}; the new reference is in {}",
            baseline.display(),
            written.display()
        );
    }
}
