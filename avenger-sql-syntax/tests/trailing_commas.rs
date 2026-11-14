//! avenger-sql's one extension of GoogleSQL's syntax: a trailing comma in every comma-separated
//! list.

mod common;

use avenger_sql_syntax::{parse, SyntaxKind::*};
use common::*;

#[test]
fn every_list_takes_a_trailing_comma() {
    for text in TRAILING_COMMAS {
        accepts(text);
    }
}

#[test]
fn a_tuple_takes_a_trailing_comma_and_a_value_in_parentheses_does_not() {
    let root = accepts("FROM t |> SELECT (a, b,) AS p");
    assert_eq!(nodes(&root, TUPLE_EXPR).len(), 1);
    rejects("FROM t |> SELECT (x,) AS a", "takes no trailing comma", ",");
}

#[test]
fn the_names_of_a_declare_read_a_name_after_their_last_comma() {
    // A type can follow the names, so a word after the last comma is a name, not a type.
    let root = parse("DECLARE a, b, INT64").syntax();
    assert_eq!(nodes(&root, NAME).len(), 3);
}
