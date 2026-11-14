//! The shapes of trees: precedence, associativity, and the nodes of each form.

mod common;

use avenger_sql_syntax::SyntaxKind::*;
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
    let joins = nodes(&root, JOIN);
    assert_eq!(joins.len(), 2);
    let outer = &joins[0];
    let left = outer.children().next().unwrap();
    assert_eq!(left.kind(), JOIN);
    assert_eq!(text_of(&left, JOIN_OP), ",");
}

#[test]
fn a_dashed_table_name_holds_its_dashes() {
    let root = accepts("FROM my-project-2.dataset.table");
    assert_eq!(text_of(&root, PATH), "my-project-2.dataset.table");
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
