//! What the parser accepts and rejects, as GoogleSQL's grammar and its parser tests settle it.

mod common;

use common::*;

#[test]
fn accepts_the_queries_and_scripts_of_the_subset() {
    for text in [OPERATORS, STATEMENTS, SOURCES] {
        accepts(text);
    }
    for text in QUERIES {
        accepts(text);
    }
    accepts_module(MODULE);
}

#[test]
fn accepts_every_form_of_expression() {
    for text in EXPRESSIONS {
        accepts(&format!("FROM t |> WHERE {text}"));
    }
}

#[test]
fn rejects_what_googlesql_rejects_at_its_first_error() {
    for (text, message, at) in REJECTED {
        rejects(text, message, at);
    }
    for (text, message, at) in REJECTED_MODULES {
        rejects_module(text, message, at);
    }
}
