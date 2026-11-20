//! The shapes of trees: precedence, associativity, and the nodes of each form.

mod common;

use avenger_sql_syntax::{
    ast::{self, AstNode},
    SyntaxKind::*,
};
use common::*;

#[test]
fn precedence_follows_googlesql() {
    for (text, expected) in [
        (
            "a OR b AND NOT c = d + e * f",
            "(a OR (b AND (NOT (c = (d + (e * f))))))",
        ),
        ("-a.b.c * 2", "((- a.b.c) * 2)"),
        ("a | b ^ c & d << 1 = x", "((a | (b ^ (c & (d << 1)))) = x)"),
        (
            "x BETWEEN 1 AND 2 AND y IS NOT NULL",
            "((x BETWEEN 1 2) AND (y IS NOT NULL))",
        ),
        ("a || b * c", "(a || (b * c))"),
        ("a >> 2 <> b", "((a >> 2) <> b)"),
        ("f(x).y", "f(..).y"),
        ("x.1e+5 > 0", "((x.1e + 5) > 0)"),
    ] {
        assert_eq!(shape(&condition(text)), expected, "{text}");
    }
}

#[test]
fn commas_and_joins_associate_to_the_left() {
    let root = accepts("FROM a, b JOIN c USING (k)");
    let outer = root.descendants().find_map(ast::Join::cast).unwrap();
    assert!(outer.join_op().and_then(|op| op.join_kw()).is_some());
    assert!(outer.join_condition().is_some());
    let Some(ast::Source::Join(left)) = outer.lhs() else {
        panic!("the left operand is not a join: {outer:?}");
    };
    assert!(left.join_op().and_then(|op| op.comma()).is_some());
    assert_eq!(left.to_string(), "a, b");
}

#[test]
fn forms_hold_their_parts() {
    let expr = condition("ANY_VALUE(x HAVING MAX y) = STRING_AGG(s IGNORE NULLS)");
    assert_eq!(text_of(&expr, HAVING_CLAUSE), "HAVING MAX y");
    assert_eq!(text_of(&expr, NULL_TREATMENT), "IGNORE NULLS");
    let expr = condition("CASE WHEN a THEN 1 ELSE 2 END = 1");
    assert_eq!(text_of(&expr, WHEN_CLAUSE), "WHEN a THEN 1");
    let expr = condition("CURRENT_DATE > d");
    assert_eq!(nodes(&expr, CALL_EXPR).len(), 1);
}

#[test]
fn the_statements_of_a_data_script_hold_their_parts() {
    use ast::CreateStmt;
    let root = accepts_data(
        "LOAD DATA INTO TEMP TABLE s.t (a INT64) OPTIONS (description = 'x')
         FROM FILES (format = 'CSV', uris = ['t.csv']) WITH PARTITION COLUMNS (y INT64)",
    );
    let load = root
        .descendants()
        .find_map(ast::LoadDataStmt::cast)
        .unwrap();
    assert_eq!(load.mode().map(|token| token.kind()), Some(INTO_KW));
    assert_eq!(load.scope().map(|token| token.kind()), Some(TEMP_KW));
    assert_eq!(load.path().unwrap().to_string(), "s.t");
    assert_eq!(load.column_list().unwrap().column_defs().count(), 1);
    assert_eq!(load.options().unwrap().option_entries().count(), 1);
    assert_eq!(load.from_files().unwrap().option_entries().count(), 2);
    let partition = load.partition_columns().and_then(|p| p.column_list());
    assert_eq!(partition.unwrap().column_defs().count(), 1);

    let root = accepts_data("CREATE EXTERNAL TABLE t WITH PARTITION COLUMNS OPTIONS (a = 1)");
    let table = (root.descendants())
        .find_map(ast::CreateExternalTableStmt::cast)
        .unwrap();
    assert_eq!(table.name_path().unwrap().to_string(), "t");
    assert!(table.column_list().is_none());
    assert!(table.partition_columns().unwrap().column_list().is_none());
    assert_eq!(table.options().unwrap().option_entries().count(), 1);

    let root = accepts_data("CREATE TEMP VIEW v OPTIONS (description = 'x') AS FROM t");
    let view = root
        .descendants()
        .find_map(ast::CreateViewStmt::cast)
        .unwrap();
    assert_eq!(view.scope_kind(), Some(ast::Scope::Temp));
    assert_eq!(view.query().unwrap().to_string(), "FROM t");
    assert!(view.options().is_some());

    let root = accepts_data("CREATE OR REPLACE SCHEMA finance");
    let schema = root
        .descendants()
        .find_map(ast::CreateSchemaStmt::cast)
        .unwrap();
    assert!(schema.or_replace());
    assert_eq!(schema.name_path().unwrap().to_string(), "finance");
}
