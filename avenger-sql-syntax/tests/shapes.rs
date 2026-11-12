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
    let refs = nodes(&root, NAME_REF);
    let names: Vec<_> = refs.iter().map(|node| node.text().to_string()).collect();
    assert_eq!(names, ["my-project-2.", "dataset", "table"]);
}

#[test]
fn every_operator_has_its_node() {
    let root = accepts(OPERATORS);
    let kinds: Vec<_> = nodes(&root, QUERY)[0]
        .children()
        .skip(1)
        .map(|node| node.kind())
        .collect();
    assert_eq!(
        kinds,
        [
            PIPE_WHERE,
            PIPE_EXTEND,
            PIPE_SET,
            PIPE_DROP,
            PIPE_RENAME,
            PIPE_AS,
            PIPE_SELECT,
            PIPE_AGGREGATE,
            PIPE_DISTINCT,
            PIPE_ORDER_BY,
            PIPE_LIMIT,
            PIPE_SET_OPERATION,
            PIPE_JOIN,
            PIPE_JOIN,
            PIPE_CALL,
            PIPE_SAMPLE,
            PIPE_WITH,
            PIPE_PIVOT,
            PIPE_UNPIVOT,
            PIPE_SET_OPERATION,
            PIPE_SET_OPERATION,
        ]
    );
    assert_eq!(
        text_of(&root, GROUP_BY),
        "GROUP AND ORDER BY day DESC NULLS LAST"
    );
    assert_eq!(text_of(&root, SET_OP_MODE), "FULL OUTER");
    assert_eq!(text_of(&root, WINDOW_CLAUSE), "WINDOW w AS (ORDER BY day)");
}

#[test]
fn forms_hold_their_parts() {
    let root = accepts(SOURCES);
    assert_eq!(text_of(&root, ON_CONDITION), "ON e.k = a.k");
    assert_eq!(text_of(&root, USING_CONDITION), "USING (k)");
    assert_eq!(text_of(&root, WEIGHT), "WITH WEIGHT AS w");
    assert_eq!(nodes(&root, GROUPING_EMPTY).len(), 2);
    assert_eq!(
        text_of(&root, GROUPING_SETS),
        "GROUPING SETS((a, b), (), ROLLUP(d))"
    );
    let condition = condition("ANY_VALUE(x HAVING MAX y) = STRING_AGG(s IGNORE NULLS)");
    assert_eq!(text_of(&condition, HAVING_CLAUSE), "HAVING MAX y");
    assert_eq!(text_of(&condition, NULL_TREATMENT), "IGNORE NULLS");
    let condition = condition_text("CASE WHEN a THEN 1 ELSE 2 END = 1");
    assert_eq!(text_of(&condition, WHEN_CLAUSE), "WHEN a THEN 1");
    let condition = condition_text("CURRENT_DATE > d");
    assert_eq!(nodes(&condition, CALL_EXPR).len(), 1);
}

fn condition_text(text: &str) -> avenger_sql_syntax::SyntaxNode {
    condition(text)
}
