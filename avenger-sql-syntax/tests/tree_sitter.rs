//! The agreement of the Tree-sitter grammar of avenger-sql, the crate `tree-sitter-avenger-sql`,
//! with this crate's parser.
//!
//! For each input that this crate's parser accepts, in the first role that accepts it, the test
//! parses the text with the Tree-sitter grammar and compares the trees:
//!
//! - **Errors.** The Tree-sitter tree has no `ERROR` or `MISSING` node.
//! - **Shape.** Each node of this crate's tree has a Tree-sitter node of the same name and range,
//!   and each named Tree-sitter node has a node or a token of this crate's tree of the same name
//!   and range. A node's name is its kind in snake case, as `pipe_select` for `PIPE_SELECT`, and
//!   a token's name is in [`token_name`]. A range runs from the first token that is not trivia to
//!   the last, because the two trees attach trivia at a node's edges by their own rules.
//!
//! The report gives each input's result, and the test compares it with the baseline
//! `tests/baselines/tree-sitter.txt`. While the grammar is incomplete, the baseline records the
//! inputs that do not agree yet, so each change of the grammar shows the inputs that it brings
//! into agreement. A changed report goes to `tests/output/tree-sitter.txt`, and a copy over the
//! baseline accepts it.
//!
//! The inputs are the texts of this crate's tests, the `.sql` and `.sqlm` files of the repository,
//! the `sql` code blocks of `avenger-sql/SPEC.md` and `avenger-sql/STYLE.md`, and every keyword in
//! each kind of name position, from [`keyword_inputs`]. The report lists a keyword input only when
//! the trees disagree, and counts the others, since most keywords are names in some positions and
//! not in others.
//!
//! During work on the grammar, the test runs against a local checkout of it:
//!
//! ```bash
//! cargo test --release -p avenger-sql-syntax --test tree_sitter --config 'patch."https://github.com/avenger-vis/tree-sitter-avenger-sql".tree-sitter-avenger-sql.path="../tree-sitter-avenger-sql"'
//! ```

mod common;

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    path::{Path, PathBuf},
};

use avenger_sql_syntax::{Role, SyntaxKind, SyntaxNode, SyntaxToken};
use common::*;

/// An input: its label, its text, and the role that its place gives it, which the test tries
/// before the others.
struct Input {
    label: String,
    text: String,
    role: Role,
}

/// A node or a named token: its name and its range in bytes.
type Item = (String, usize, usize);

/// The root of the repository.
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn input(label: impl Into<String>, text: impl Into<String>, role: Role) -> Input {
    Input {
        label: label.into(),
        text: text.into(),
        role,
    }
}

fn inputs() -> Vec<Input> {
    let mut inputs = vec![
        input("crate:OPERATORS", OPERATORS, Role::Script),
        input("crate:STATEMENTS", STATEMENTS, Role::Script),
        input("crate:MODULE", MODULE, Role::Module),
        input("crate:DATA", DATA, Role::Data),
        input("crate:SOURCES", SOURCES, Role::Script),
    ];
    for (i, text) in QUERIES.iter().enumerate() {
        inputs.push(input(format!("crate:QUERIES:{i}"), *text, Role::Script));
    }
    for (i, text) in EXPRESSIONS.iter().enumerate() {
        let text = format!("FROM t |> WHERE {text}");
        inputs.push(input(format!("crate:EXPRESSIONS:{i}"), text, Role::Script));
    }
    for (i, text) in TRAILING_COMMAS.iter().enumerate() {
        inputs.push(input(
            format!("crate:TRAILING_COMMAS:{i}"),
            *text,
            Role::Script,
        ));
    }

    let root = root();
    let mut files = Vec::new();
    collect_files(&root, &mut files);
    files.sort();
    for path in files {
        let label = path.strip_prefix(&root).unwrap().display().to_string();
        let text = std::fs::read_to_string(&path).unwrap();
        let role = if label.ends_with(".sqlm") {
            Role::Module
        } else if label.split('/').any(|name| name == "data") {
            Role::Data
        } else {
            Role::Script
        };
        inputs.push(input(label, text, role));
    }

    for document in ["avenger-sql/SPEC.md", "avenger-sql/STYLE.md"] {
        let text = std::fs::read_to_string(root.join(document)).unwrap();
        for (line, block) in sql_blocks(&text) {
            inputs.push(input(format!("{document}:{line}"), block, Role::Script));
        }
    }
    inputs
}

/// The `.sql` and `.sqlm` files under `directory`, outside build output, scratch files, and
/// hidden directories.
fn collect_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if !name.starts_with('.')
                && !["target", "scratch", "node_modules"].contains(&name.as_str())
            {
                collect_files(&path, files);
            }
        } else if name.ends_with(".sql") || name.ends_with(".sqlm") {
            files.push(path);
        }
    }
}

/// The `sql` code blocks of a Markdown text, each with the line of its fence, from 1.
fn sql_blocks(text: &str) -> Vec<(usize, String)> {
    let mut blocks = Vec::new();
    let mut open: Option<(usize, String)> = None;
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        match &mut open {
            None if trimmed.starts_with("```sql") => open = Some((i + 1, String::new())),
            Some(_) if trimmed.starts_with("```") => blocks.push(open.take().unwrap()),
            Some((_, block)) => {
                block.push_str(line);
                block.push('\n');
            }
            None => {}
        }
    }
    blocks
}

/// Every keyword of `src/syntax_kind.rs`, reserved and not, in lower case.
fn keywords() -> Vec<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/syntax_kind.rs");
    let source = std::fs::read_to_string(path).unwrap();
    let mut words: Vec<String> = source
        .lines()
        .filter_map(|line| {
            let (_, rest) = line.trim().split_once("_KW = \"")?;
            Some(rest.split('"').next()?.to_ascii_lowercase())
        })
        .collect();
    words.sort();
    words.dedup();
    words
}

/// Each keyword in each kind of position where a name can be: a column, an alias with and
/// without `AS`, a table and its alias, a function, a field, an argument's name, an order key,
/// and a variable.
fn keyword_inputs() -> Vec<Input> {
    const POSITIONS: &[(&str, &str)] = &[
        ("column", "FROM t |> SELECT {}"),
        ("alias", "FROM t |> SELECT a AS {}"),
        ("bare alias", "FROM t |> SELECT a {}"),
        ("table", "FROM {}"),
        ("table alias", "FROM t {}"),
        ("function", "FROM t |> SELECT {}(1)"),
        ("field", "FROM t |> SELECT t.{}"),
        ("argument", "FROM t |> SELECT f({} => 1)"),
        ("order key", "FROM t |> ORDER BY {}"),
        ("variable", "DECLARE {} INT64"),
    ];
    let mut inputs = Vec::new();
    for word in keywords() {
        for (position, template) in POSITIONS {
            let label = format!("keyword:{word}:{position}");
            inputs.push(input(label, template.replace("{}", &word), Role::Script));
        }
    }
    inputs
}

/// The tree of `text` in the first role that accepts it, `preferred` first, or `None` when every
/// role rejects it.
fn accepted(text: &str, preferred: Role) -> Option<SyntaxNode> {
    let mut roles = vec![preferred];
    roles.extend(
        [Role::Script, Role::Data, Role::Module]
            .into_iter()
            .filter(|role| *role != preferred),
    );
    roles.into_iter().find_map(|role| {
        let parse = parse_as(text, role);
        parse.ok().then(|| parse.syntax())
    })
}

/// The name of a token of this crate's tree in the Tree-sitter tree, or `None` for a token that
/// the Tree-sitter tree holds as an anonymous node, such as a keyword or punctuation.
fn token_name(token: &SyntaxToken) -> Option<&'static str> {
    let in_name = || {
        token
            .parent()
            .is_some_and(|node| matches!(node.kind(), SyntaxKind::NAME | SyntaxKind::NAME_REF))
    };
    Some(match token.kind() {
        SyntaxKind::WORD if in_name() => "identifier",
        SyntaxKind::QUOTED_IDENT => "quoted_identifier",
        SyntaxKind::STRING => "string",
        SyntaxKind::BYTES => "bytes",
        SyntaxKind::INTEGER => "integer",
        SyntaxKind::FLOAT => "float",
        SyntaxKind::PARAM => "param",
        SyntaxKind::HINT => "hint",
        SyntaxKind::COMMENT if token.text().starts_with("--|") => "doc_comment",
        SyntaxKind::COMMENT => "comment",
        _ => return None,
    })
}

/// The nodes and named tokens of this crate's tree.
fn rust_items(root: &SyntaxNode) -> Vec<Item> {
    let mut items = Vec::new();
    for node in root.descendants() {
        let mut tokens = node
            .descendants_with_tokens()
            .filter_map(|element| element.into_token())
            .filter(|token| !token.kind().is_trivia());
        let Some(first) = tokens.next() else {
            continue;
        };
        let last = tokens.last().unwrap_or_else(|| first.clone());
        let name = format!("{:?}", node.kind()).to_lowercase();
        items.push((
            name,
            first.text_range().start().into(),
            last.text_range().end().into(),
        ));
    }
    for token in root
        .descendants_with_tokens()
        .filter_map(|element| element.into_token())
    {
        if let Some(name) = token_name(&token) {
            let range = token.text_range();
            items.push((name.to_string(), range.start().into(), range.end().into()));
        }
    }
    items.sort();
    items
}

/// The named nodes of the Tree-sitter tree, without its root and the parts of strings, which
/// this crate's tree holds as one token.
fn tree_sitter_items(root: tree_sitter::Node) -> Vec<Item> {
    // A string or bytes literal is one token here, and its quotes are hidden tokens of the
    // Tree-sitter tree, so its range is the whole node's, not its visible children's.
    fn trimmed(node: tree_sitter::Node) -> Option<(usize, usize)> {
        if node.child_count() == 0 || matches!(node.kind(), "string" | "bytes") {
            return (!node.is_extra()).then(|| (node.start_byte(), node.end_byte()));
        }
        let mut cursor = node.walk();
        let ranges: Vec<_> = node
            .children(&mut cursor)
            .filter(|child| !child.is_extra())
            .filter_map(trimmed)
            .collect();
        Some((ranges.first()?.0, ranges.last()?.1))
    }
    fn visit(node: tree_sitter::Node, items: &mut Vec<Item>) {
        let skipped = matches!(
            node.kind(),
            "source_file" | "string_content" | "escape_sequence"
        );
        if node.is_named() && !skipped {
            let range = match node.is_extra() || node.child_count() == 0 {
                true => Some((node.start_byte(), node.end_byte())),
                false => trimmed(node),
            };
            if let Some((start, end)) = range {
                items.push((node.kind().to_string(), start, end));
            }
        }
        if !matches!(node.kind(), "string" | "bytes") {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                visit(child, items);
            }
        }
    }
    let mut items = Vec::new();
    visit(root, &mut items);
    items.sort();
    items
}

/// The first `ERROR` or `MISSING` node of the Tree-sitter tree.
fn first_error(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    if node.is_error() || node.is_missing() {
        return Some(node);
    }
    if !node.has_error() {
        return None;
    }
    let mut cursor = node.walk();
    let children: Vec<_> = node.children(&mut cursor).collect();
    children.into_iter().find_map(first_error)
}

/// The text of `start..end`, on one line and at most 40 characters.
fn snippet(text: &str, start: usize, end: usize) -> String {
    let piece: String = text[start..end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    match piece.chars().count() > 40 {
        true => format!("`{}…`", piece.chars().take(40).collect::<String>()),
        false => format!("`{piece}`"),
    }
}

/// The items of `of` that `other` lacks, counting repeats.
fn difference(of: &[Item], other: &[Item]) -> Vec<Item> {
    let mut counts: BTreeMap<&Item, i64> = BTreeMap::new();
    for item in other {
        *counts.entry(item).or_default() += 1;
    }
    let mut missing = Vec::new();
    for item in of {
        let count = counts.entry(item).or_default();
        if *count > 0 {
            *count -= 1;
        } else {
            missing.push(item.clone());
        }
    }
    missing.sort_by_key(|(name, start, end)| (*start, std::cmp::Reverse(*end), name.clone()));
    missing
}

#[test]
fn the_tree_sitter_grammar_agrees_with_the_parser() {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_avenger_sql::LANGUAGE.into())
        .unwrap();

    let mut report = String::new();
    let mut counts = BTreeMap::<&str, usize>::new();
    let mut keyword_counts = BTreeMap::<&str, usize>::new();
    let ordinary = inputs();
    let quiet = ordinary.len();
    for (index, input) in ordinary.into_iter().chain(keyword_inputs()).enumerate() {
        // A keyword input counts apart, and its line goes in the report only when the trees
        // disagree.
        let keyword = index >= quiet;
        let counts = match keyword {
            true => &mut keyword_counts,
            false => &mut counts,
        };
        let Some(root) = accepted(&input.text, input.role) else {
            *counts.entry("rejected").or_default() += 1;
            if !keyword {
                writeln!(report, "rejected  {}", input.label).unwrap();
            }
            continue;
        };
        let tree = parser.parse(&input.text, None).unwrap();
        if let Some(error) = first_error(tree.root_node()) {
            let point = error.start_position();
            let what = match error.is_missing() {
                true => format!("missing `{}`", error.kind()),
                false => snippet(&input.text, error.start_byte(), error.end_byte()),
            };
            *counts.entry("error").or_default() += 1;
            writeln!(
                report,
                "error     {} at {}:{}: {what}",
                input.label,
                point.row + 1,
                point.column + 1
            )
            .unwrap();
            continue;
        }
        let expected = rust_items(&root);
        let actual = tree_sitter_items(tree.root_node());
        let missing = difference(&expected, &actual);
        let extra = difference(&actual, &expected);
        if missing.is_empty() && extra.is_empty() {
            *counts.entry("agree").or_default() += 1;
            if !keyword {
                writeln!(report, "agree     {}", input.label).unwrap();
            }
            continue;
        }
        *counts.entry("shape").or_default() += 1;
        let describe = |items: &[Item]| {
            items
                .first()
                .map_or_else(String::new, |(name, start, end)| {
                    format!("{name} {}", snippet(&input.text, *start, *end))
                })
        };
        writeln!(
            report,
            "shape     {}: {} missing, first {}; {} extra, first {}",
            input.label,
            missing.len(),
            describe(&missing),
            extra.len(),
            describe(&extra),
        )
        .unwrap();
    }
    let summary = |counts: &BTreeMap<&str, usize>| {
        let parts: Vec<_> = counts
            .iter()
            .map(|(kind, count)| format!("{count} {kind}"))
            .collect();
        parts.join(", ")
    };
    writeln!(report, "\n{}", summary(&counts)).unwrap();
    writeln!(report, "keywords: {}", summary(&keyword_counts)).unwrap();

    assert_text_baseline("tree-sitter", &report);
}
