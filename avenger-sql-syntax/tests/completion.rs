//! What the grammar accepts at a cursor, for completion.

use avenger_sql_syntax::{
    expected_at, parse, Expected, NameRole, SyntaxKind::*, DATE_PARTS, INTERVAL_PARTS,
};

fn at_end(text: &str) -> Vec<Expected> {
    expected_at(text, (text.len() as u32).into())
}

/// The words and phrases that the grammar takes at the end of `text`, each as one string.
fn words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    for expected in at_end(text) {
        match expected {
            Expected::Words(set) => words.extend(set.iter().map(|word| word.to_string())),
            Expected::Phrase(phrase) => words.push(phrase.join(" ")),
            Expected::Keyword(kind) => {
                let name = format!("{kind:?}");
                words.push(name.trim_end_matches("_KW").to_string());
            }
            _ => {}
        }
    }
    words
}

#[test]
fn completion_offers_what_the_grammar_accepts() {
    assert!(at_end("").contains(&Expected::Statement));
    assert!(at_end("FROM ").contains(&Expected::Name(NameRole::Table)));
    assert!(at_end("FROM t |> ").contains(&Expected::PipeOperator));
    assert!(at_end("FROM t |> SEL").contains(&Expected::PipeOperator));
    assert!(at_end("FROM t |> WHERE ").contains(&Expected::Expression));
    assert!(at_end("FROM t |> AGGREGATE ").contains(&Expected::Expression));
    // A statement starts after a `;`, not after a statement that the text has not ended.
    assert!(at_end("FROM t; ").contains(&Expected::Statement));
    assert!(!at_end("FROM t |> WHERE x = 1 ").contains(&Expected::Statement));
    // Past a missing `)`, the clauses after it are not what the cursor takes.
    let over = at_end("FROM t |> EXTEND f() OVER (");
    assert!(over.contains(&Expected::Keyword(PARTITION_KW)), "{over:?}");
    assert!(!over.contains(&Expected::Keyword(AS_KW)), "{over:?}");
    assert!(!over.contains(&Expected::Keyword(WINDOW_KW)), "{over:?}");
    let group = at_end("FROM t |> AGGREGATE COUNT(*) GROUP ");
    assert!(group.contains(&Expected::Keyword(BY_KW)), "{group:?}");
    assert!(
        group.contains(&Expected::Phrase(&["AND", "ORDER"])),
        "{group:?}"
    );
    assert!(at_end("FROM t |> SELECT t.").contains(&Expected::Name(NameRole::Field)));
    let call = at_end("FROM t |> WHERE f(");
    assert!(call.contains(&Expected::Expression), "{call:?}");
    assert!(
        call.contains(&Expected::Name(NameRole::Parameter)),
        "{call:?}"
    );
    assert!(at_end("FROM t |> WHERE CAST(x AS ").contains(&Expected::Type));
}

#[test]
fn completion_offers_what_can_follow_an_operand_and_start_an_expression() {
    // An item after a comma of a list that takes a comma after its last item.
    assert!(at_end("FROM t |> SELECT x, ").contains(&Expected::Expression));
    assert!(at_end("FROM t |> AGGREGATE COUNT(*), ").contains(&Expected::Expression));
    // The operators after an operand.
    let after = words("FROM t |> WHERE x ");
    for word in ["IN", "AND", "IS", "LIKE", "BETWEEN", "NOT IN"] {
        assert!(after.iter().any(|w| w == word), "{word}: {after:?}");
    }
    assert!(at_end("FROM t |> WHERE x ").contains(&Expected::Token(EQ)));
    // The words that start an expression, and a subquery after `(`.
    let start = words("FROM t |> WHERE ");
    for word in [
        "NOT", "EXISTS", "CASE", "CAST", "TRUE", "DATE", "INTERVAL", "STRUCT",
    ] {
        assert!(start.iter().any(|w| w == word), "{word}: {start:?}");
    }
    assert!(at_end("FROM t |> WHERE x IN (").contains(&Expected::Keyword(FROM_KW)));
}

#[test]
fn completion_offers_the_words_of_a_set_and_the_rest_of_a_phrase() {
    assert!(at_end("FROM t |> SELECT INTERVAL 1 ").contains(&Expected::Words(INTERVAL_PARTS)));
    assert!(at_end("FROM t |> SELECT EXTRACT(").contains(&Expected::Words(DATE_PARTS)));
    assert!(words("FROM t |> TABLESAMPLE ").contains(&"BERNOULLI".to_string()));
    assert!(words("FROM t |> UNION ALL ").contains(&"BY NAME".to_string()));
    assert!(at_end("FROM t |> UNION ALL BY ").contains(&Expected::Phrase(&["NAME"])));
    assert!(words("CREATE TEMP TABLE ").contains(&"IF NOT EXISTS".to_string()));
    assert!(at_end("DECLARE k ").contains(&Expected::Type));
    assert!(at_end("FROM t |> EXTEND RANK() OVER (").contains(&Expected::Name(NameRole::Window)));
}

#[test]
fn completion_leaves_out_the_forms_that_avenger_sql_rejects() {
    let none = |text: &str, rejected: &[&str]| {
        let found = words(text);
        for word in rejected {
            assert!(
                !found.iter().any(|w| w.split(' ').next() == Some(*word)),
                "{text}: {word} in {found:?}"
            );
        }
    };
    none("", &["SELECT", "WITH"]);
    none("FROM t |> UNION ALL (", &["SELECT", "WITH"]);
    none("CREATE TEMP ", &["CONSTANT"]);
    none("CREATE ", &["PUBLIC", "PRIVATE"]);
    none("FROM t |> AGGREGATE COUNT(x ", &["GROUP", "WHERE"]);
    none("FROM t |> LIMIT ", &["ALL"]);
    none(
        "CREATE TEMP FUNCTION f(x INT64) RETURNS INT64 ",
        &["LANGUAGE"],
    );
    none("FROM t |> SELECT CAST(x AS ", &["RANGE"]);
    none("FROM t ", &["FOR"]);
    none("FROM t |> JOIN ", &["HASH", "LOOKUP"]);
}

#[test]
fn each_pipe_operator_that_completion_offers_parses() {
    let operators = words("FROM t |> ");
    assert!(operators.len() > 20, "{operators:?}");
    for operator in operators {
        let text = format!("FROM t |> {operator}");
        let errors: Vec<_> = (parse(&text).diagnostics().iter())
            .filter(|diagnostic| usize::from(diagnostic.range.start()) < text.trim_end().len())
            .map(|diagnostic| diagnostic.message.clone())
            .collect();
        assert!(errors.is_empty(), "{text}: {errors:?}");
    }
}

#[test]
fn completion_offers_what_the_place_of_the_cursor_takes() {
    // The value columns of `UNPIVOT` are new names, and its items the input's columns.
    let values = at_end("FROM t |> UNPIVOT (");
    assert!(
        values.contains(&Expected::Name(NameRole::Alias)),
        "{values:?}"
    );
    assert!(
        !values.contains(&Expected::Name(NameRole::Column)),
        "{values:?}"
    );
    assert!(at_end("FROM t |> UNPIVOT (v FOR k IN (").contains(&Expected::Name(NameRole::Column)));
    // The column of a `PIVOT` binds tighter than `IN`, so no comparison follows it.
    let column = words("FROM t |> PIVOT (SUM(x) FOR s ");
    assert!(column.contains(&"IN".to_string()), "{column:?}");
    assert!(!column.contains(&"AND".to_string()) && !column.contains(&"NOT IN".to_string()));
    // A table parameter belongs to a table function.
    assert!(!words("CREATE TEMP FUNCTION f(x ").contains(&"ANY TABLE".to_string()));
    assert!(words("CREATE TEMP TABLE FUNCTION f(x ").contains(&"ANY TABLE".to_string()));
    // A set operation's mode goes with `BY NAME` or `CORRESPONDING`.
    let mode = words("FROM t |> FULL UNION ");
    assert!(mode.contains(&"ALL BY NAME".to_string()), "{mode:?}");
    assert!(!mode.contains(&"ALL".to_string()), "{mode:?}");
}
