//! What the grammar accepts at a cursor, for completion.

use avenger_sql_syntax::{expected_at, Expected, NameRole, SyntaxKind::*};

fn at_end(text: &str) -> Vec<Expected> {
    expected_at(text, (text.len() as u32).into())
}

#[test]
fn completion_offers_what_the_grammar_accepts() {
    assert!(at_end("").contains(&Expected::Statement));
    assert!(at_end("FROM ").contains(&Expected::Name(NameRole::Table)));
    assert!(at_end("FROM t |> ").contains(&Expected::PipeOperator));
    assert!(at_end("FROM t |> SEL").contains(&Expected::PipeOperator));
    assert!(at_end("FROM t |> WHERE ").contains(&Expected::Expression));
    let group = at_end("FROM t |> AGGREGATE COUNT(*) GROUP ");
    assert!(group.contains(&Expected::Keyword(BY_KW)), "{group:?}");
    assert!(group.contains(&Expected::Keyword(AND_KW)), "{group:?}");
    assert!(at_end("FROM t |> SELECT t.").contains(&Expected::Name(NameRole::Field)));
    let call = at_end("FROM t |> WHERE f(");
    assert!(call.contains(&Expected::Expression), "{call:?}");
    assert!(
        call.contains(&Expected::Name(NameRole::Parameter)),
        "{call:?}"
    );
    assert!(at_end("FROM t |> WHERE CAST(x AS ").contains(&Expected::Type));
}
