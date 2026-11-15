//! Recovery: after an error, the parser reports it, skips what it cannot read, and goes on.

mod common;

use avenger_sql_syntax::{parse, DiagnosticCode, SyntaxKind::*};
use common::*;

fn recovered(text: &str, errors: usize) -> avenger_sql_syntax::SyntaxNode {
    let parse = parse(text);
    assert_eq!(parse.syntax().text().to_string(), text);
    let messages: Vec<_> = parse
        .diagnostics()
        .iter()
        .map(|d| describe(text, d))
        .collect();
    assert_eq!(messages.len(), errors, "{text}: {messages:#?}");
    parse.syntax()
}

#[test]
fn a_missing_expression_leaves_the_next_operator() {
    let root = recovered("FROM t |> WHERE |> SELECT a", 1);
    assert_eq!(text_of(&root, PIPE_SELECT), "|> SELECT a");
}

#[test]
fn a_bad_item_leaves_the_other_items() {
    let root = recovered("FROM t |> SELECT a, , b", 1);
    let items: Vec<_> = nodes(&root, SELECT_ITEM)
        .iter()
        .map(|item| item.text().to_string().trim().to_string())
        .collect();
    assert_eq!(items.last().map(String::as_str), Some("b"));
}

#[test]
fn an_unknown_operator_leaves_the_next_operator() {
    let root = recovered("FROM t |> FOO x y |> SELECT a", 1);
    assert_eq!(text_of(&root, PIPE_SELECT), "|> SELECT a");
}

#[test]
fn an_unterminated_string_ends_at_its_line() {
    let root = recovered("FROM t |> WHERE x = 'abc\n|> SELECT b", 1);
    assert_eq!(text_of(&root, PIPE_SELECT), "|> SELECT b");
}

#[test]
fn a_missing_parenthesis_leaves_the_next_operator() {
    let root = recovered("FROM t |> WHERE (a + b |> SELECT c", 1);
    assert_eq!(text_of(&root, PIPE_SELECT), "|> SELECT c");
}

#[test]
fn a_bad_statement_leaves_the_next_statement() {
    let root = recovered("SELECT a FROM t; FROM u |> SELECT b", 1);
    assert_eq!(text_of(&root, PIPE_SELECT), "|> SELECT b");
}

#[test]
fn deep_nesting_is_one_diagnostic() {
    let text = format!("FROM t |> WHERE {}1{}", "(".repeat(2000), ")".repeat(2000));
    let parse = parse(&text);
    assert_eq!(parse.syntax().text().to_string(), text);
    let deep: Vec<_> = parse
        .diagnostics()
        .iter()
        .filter(|d| d.code == DiagnosticCode::TooDeep)
        .collect();
    assert_eq!(deep.len(), 1);
}

/// A standard query of another shape keeps its diagnostic, without a rewrite.
#[test]
fn standard_queries_with_aggregates_or_grouping_have_no_rewrite() {
    for text in [
        "SELECT COUNT(*) FROM t",
        "SELECT a FROM t GROUP BY a",
        "SELECT a FROM t UNION ALL SELECT b FROM u",
        "SELECT 1",
    ] {
        let parse = parse(text);
        let diagnostic = &parse.diagnostics()[0];
        assert_eq!(diagnostic.code, DiagnosticCode::NotPipeSyntax, "{text}");
        assert!(
            diagnostic.fixes.is_empty(),
            "{text}: {:?}",
            diagnostic.fixes
        );
    }
}

#[test]
fn fixes_repair_the_text() {
    for (text, fixed) in [
        ("FROM t |> DROP order", "FROM t |> DROP `order`"),
        ("FROM t |> WHERE a = NOT b", "FROM t |> WHERE a = (NOT b)"),
        ("FROM t |> SELECT 5abc", "FROM t |> SELECT `5abc`"),
        ("FROM t |> WHERE x = 'a''b'", "FROM t |> WHERE x = 'a' 'b'"),
        // A standard query of a simple shape, rewritten as a pipe query.
        (
            "SELECT a, b + 1 AS c FROM t JOIN u USING (k) WHERE a > 1 ORDER BY c DESC LIMIT 5",
            "FROM t JOIN u USING (k) |> WHERE a > 1 |> SELECT a, b + 1 AS c |> ORDER BY c DESC |> LIMIT 5",
        ),
        (
            "FROM t |> WHERE x IN (SELECT DISTINCT y FROM u)",
            "FROM t |> WHERE x IN (FROM u |> SELECT DISTINCT y)",
        ),
        ("SELECT * FROM t;", "FROM t |> SELECT *;"),
    ] {
        let parse = parse(text);
        let fix = &parse.diagnostics()[0].fixes[0];
        let mut repaired = text.to_string();
        for edit in fix.edits.iter().rev() {
            repaired.replace_range(
                std::ops::Range::<usize>::from(edit.range),
                &edit.replacement,
            );
        }
        assert_eq!(repaired, fixed);
        accepts(&repaired);
    }
}
