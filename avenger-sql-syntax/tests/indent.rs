//! The indentation of a line by the style: on every line of the formatter's output, it gives the
//! line's indentation, and on unfinished text, it gives the depth of the most likely next line.

mod common;

use std::path::PathBuf;

use avenger_sql_syntax::{format::format, indent::depth, parse};

/// The lines of `formatted`, a formatted text, whose depth differs from their indentation.
fn mismatches(formatted: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut start = 0;
    for line in formatted.split_inclusive('\n') {
        let content = line.trim_end_matches('\n');
        if !content.trim().is_empty() {
            let indentation = content.len() - content.trim_start().len();
            if let Some(depth) = depth(formatted, start) {
                if depth != indentation {
                    found.push(format!("{depth} for {content:?}"));
                }
            }
        }
        start += line.len();
    }
    found
}

fn check(text: &str) {
    if !parse(text).diagnostics().is_empty() {
        return;
    }
    let formatted = format(text);
    let found = mismatches(&formatted);
    assert!(found.is_empty(), "{}\n\nin\n{formatted}", found.join("\n"));
}

#[test]
fn every_line_of_the_formatted_corpora_has_its_depth() {
    let mut texts: Vec<&str> = vec![common::OPERATORS, common::STATEMENTS, common::SOURCES];
    texts.extend(common::EXPRESSIONS.iter().copied());
    texts.extend(common::QUERIES.iter().copied());
    texts.extend(common::TRAILING_COMMAS.iter().copied());
    for text in texts {
        check(text);
    }
    // The formatter's snapshots, statement by statement, since one holds a statement with an
    // error.
    let baselines = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/baselines");
    for name in ["comments", "lists", "expressions", "statements"] {
        let text = std::fs::read_to_string(baselines.join(format!("format-{name}.txt"))).unwrap();
        for statement in text.split(";\n") {
            check(statement);
        }
    }
}

/// The inputs of GoogleSQL's parser tests in the subset, as in the formatter's ignored test.
#[test]
#[ignore]
fn every_line_of_the_formatted_googlesql_corpus_has_its_depth() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../scratch/tools/googlesql/parser-corpus.txt");
    let Ok(corpus) = std::fs::read_to_string(&path) else {
        eprintln!("no corpus at {}", path.display());
        return;
    };
    let mut input = String::new();
    for line in corpus.lines().skip(1).chain(["## end"]) {
        match line.starts_with("## ") {
            true => {
                check(&input);
                input.clear();
            }
            false => {
                input.push_str(line);
                input.push('\n');
            }
        }
    }
}

/// The depth of a new line after `before`, as on Enter at the end of `before`.
fn after(before: &str) -> usize {
    let text = format!("{before}\n");
    depth(&text, text.len()).unwrap()
}

#[test]
fn a_new_line_takes_the_depth_of_what_most_likely_comes_next() {
    // The next pipe operator, after a source and after an operator.
    assert_eq!(after("FROM t"), 0);
    assert_eq!(after("FROM t\n|> WHERE a > 0"), 0);
    // An item, after a comma, after a keyword that starts a list, and after an opener.
    assert_eq!(after("FROM t\n|> AGGREGATE count(*) AS n,"), 5);
    assert_eq!(after("FROM t\n|> AGGREGATE"), 5);
    assert_eq!(after("FROM t\n|> AGGREGATE mark_bar("), 5);
    assert_eq!(after("FROM t\n|> AGGREGATE mark_bar(\n     x => x("), 7);
    assert_eq!(
        after("FROM t\n|> AGGREGATE mark_bar(\n     x => species,"),
        5
    );
    // A query in parentheses.
    assert_eq!(after("FROM t\n|> JOIN ("), 5);
    assert_eq!(after("FROM t\n|> JOIN (\n     FROM u"), 5);
    // A blank line before a closer takes the depth of the items, not of the closer.
    let text = "FROM t\n|> AGGREGATE mark_bar(\n     angle => 23,\n\n   )";
    assert_eq!(depth(text, text.find("\n\n").unwrap() + 1), Some(5));
    // A new statement.
    assert_eq!(after("FROM t;"), 0);
    assert_eq!(after(""), 0);
}

/// The depth of the last line of `text`.
fn last(text: &str) -> usize {
    depth(text, text.rfind('\n').map_or(0, |end| end + 1)).unwrap()
}

#[test]
fn the_first_token_of_a_line_decides_its_depth() {
    assert_eq!(
        last("FROM t\n|> AGGREGATE mark_bar(\n     x => species,\n)"),
        3
    );
    assert_eq!(
        last("FROM t\n|> AGGREGATE mark_bar(x => species) AS chart\nGROUP BY island"),
        3
    );
    assert_eq!(last("FROM t\n|> WHERE a > 0\nAND b > 0"), 5);
    assert_eq!(last("FROM t\n|> JOIN (\n     FROM u\n|> WHERE b"), 5);
    assert_eq!(
        last("FROM t\n|> EXTEND CASE\n     WHEN a THEN b\nEND AS c"),
        3
    );
    // Inside a string, the line keeps its text.
    assert_eq!(depth("FROM t |> SELECT '''\n  x'''", 21), None);
}
