//! The trees of scripts and a module with every kind of node, against baselines.

mod common;

use common::*;

#[test]
fn the_tree_of_every_operator() {
    assert_baseline("operators", &accepts(OPERATORS));
}

#[test]
fn the_tree_of_every_statement() {
    assert_baseline("statements", &accepts(STATEMENTS));
}

#[test]
fn the_tree_of_every_source() {
    assert_baseline("sources", &accepts(SOURCES));
}

#[test]
fn the_tree_of_a_module() {
    assert_baseline("module", &accepts_module(MODULE));
}
