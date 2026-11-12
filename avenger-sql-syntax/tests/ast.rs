//! The typed nodes: their accessors, and the values that they decode.

mod common;

use avenger_sql_syntax::ast::{self, AstNode, BinaryOp, LiteralKind};
use common::*;

/// The condition of `FROM t |> WHERE <text>`, as an expression.
fn expr(text: &str) -> ast::Expr {
    ast::Expr::cast(condition(text)).expect("an expression")
}

fn literal(text: &str) -> ast::Literal {
    match expr(text) {
        ast::Expr::Literal(literal) => literal,
        other => panic!("{text} is not a literal: {other:?}"),
    }
}

#[test]
fn literals_hold_their_values() {
    assert_eq!(literal("42").int_value(), Some(42));
    assert_eq!(literal("0x1F").int_value(), Some(31));
    assert_eq!(literal("18446744073709551616").int_value(), None);
    assert_eq!(literal("1.5e1").float_value(), Some(15.0));
    assert_eq!(literal("1.").float_value(), Some(1.0));
    assert_eq!(literal("NULL").kind(), Some(LiteralKind::Null));
    assert_eq!(literal("true").kind(), Some(LiteralKind::True));
    assert_eq!(
        literal(r"'a\n\x41é\101'").string_value().as_deref(),
        Some("a\nAéA")
    );
    assert_eq!(literal(r"r'\n'").string_value().as_deref(), Some(r"\n"));
    assert_eq!(
        literal("'''x\ny''' \"z\"").string_value().as_deref(),
        Some("x\nyz")
    );
    assert_eq!(literal(r"b'\x00\xff'").bytes_value(), Some(vec![0, 255]));
    assert_eq!(literal(r"rb'\x'").bytes_value(), Some(br"\x".to_vec()));
}

#[test]
fn names_drop_their_quotes() {
    let root = accepts("FROM `my table`.`a\\x62` AS `select` |> SELECT my_col");
    let source = root
        .descendants()
        .find_map(ast::TableSource::cast)
        .expect("a table");
    let path = source.path().expect("a path");
    assert_eq!(
        path.names(),
        Some(vec!["my table".to_string(), "ab".to_string()])
    );
    let alias = source
        .alias()
        .and_then(|alias| alias.name())
        .expect("an alias");
    assert_eq!(alias.text().as_deref(), Some("select"));
    let dashed = accepts("FROM my-project-2.dataset.t");
    let path = dashed
        .descendants()
        .find_map(ast::Path::cast)
        .expect("a path");
    assert_eq!(
        path.names(),
        Some(vec![
            "my-project-2".to_string(),
            "dataset".to_string(),
            "t".to_string()
        ])
    );
}

#[test]
fn operators_read_their_tokens() {
    let ops = [
        ("a <> b", BinaryOp::NotEq),
        ("a != b", BinaryOp::NotEq),
        ("a >> b", BinaryOp::ShiftRight),
        ("a > b", BinaryOp::Gt),
        ("a || b", BinaryOp::Concat),
        ("a AND b", BinaryOp::And),
    ];
    for (text, op) in ops {
        let ast::Expr::BinExpr(bin) = expr(text) else {
            panic!("{text} is not a binary expression");
        };
        assert_eq!(bin.operator(), Some(op), "{text}");
        assert!(bin.lhs().is_some() && bin.rhs().is_some(), "{text}");
    }
    let ast::Expr::BetweenExpr(between) = expr("x NOT BETWEEN 1 AND 2") else {
        panic!("not a BETWEEN");
    };
    assert!(between.not_kw().is_some());
    assert_eq!(
        between.low().map(|low| low.to_string()).as_deref(),
        Some("1")
    );
    assert_eq!(
        between.high().map(|high| high.to_string()).as_deref(),
        Some("2")
    );
}

#[test]
fn a_query_reads_as_typed_nodes() {
    let parse = avenger_sql_syntax::parse(
        "FROM orders AS o JOIN customers c USING (id) |> WHERE o.total > 10 |> AGGREGATE SUM(o.total) AS total GROUP BY c.region",
    );
    assert!(parse.ok());
    let script = parse.script().expect("a script");
    let Some(ast::Stmt::Query(query)) = script.stmts().next() else {
        panic!("not a query");
    };
    let source = query.from_clause().and_then(|from| from.source());
    let Some(ast::Source::Join(join)) = source else {
        panic!("not a join");
    };
    assert!(matches!(join.lhs(), Some(ast::Source::TableSource(_))));
    assert!(matches!(
        join.join_condition(),
        Some(ast::JoinCondition::UsingCondition(_))
    ));
    let operators: Vec<ast::PipeOperator> = query.pipe_operators().collect();
    assert_eq!(operators.len(), 2);
    let ast::PipeOperator::PipeAggregate(aggregate) = &operators[1] else {
        panic!("not an AGGREGATE");
    };
    let item = aggregate.aggregate_items().next().expect("an item");
    assert_eq!(
        item.alias()
            .and_then(|alias| alias.name())
            .and_then(|name| name.text())
            .as_deref(),
        Some("total")
    );
    let groups: Vec<_> = aggregate
        .group_by()
        .expect("a GROUP BY")
        .grouping_items()
        .collect();
    assert_eq!(groups.len(), 1);
}

#[test]
fn statements_read_their_modifiers() {
    use ast::{CreateStmt, FunctionKind, Scope};
    let parse = avenger_sql_syntax::parse(
        "CREATE TEMP AGGREGATE FUNCTION f(x INT64 NOT AGGREGATE, y FLOAT64) RETURNS FLOAT64 AS (SUM(y) * x)",
    );
    assert!(parse.ok());
    let Some(ast::Stmt::CreateFunctionStmt(function)) =
        parse.script().and_then(|script| script.stmts().next())
    else {
        panic!("not a CREATE FUNCTION");
    };
    assert_eq!(function.function_kind(), FunctionKind::Aggregate);
    assert_eq!(function.scope_kind(), Some(Scope::Temp));
    assert!(!function.or_replace());
    let parameters: Vec<_> = function
        .param_list()
        .expect("parameters")
        .parameters()
        .collect();
    assert_eq!(parameters.len(), 2);
    assert!(parameters[0].not_kw().is_some());
    assert!(function
        .function_body()
        .and_then(|body| body.expr())
        .is_some());
}
