//! The trees of scripts, a module and a data script with every kind of node, against baselines.

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

#[test]
fn the_tree_of_a_data_script() {
    assert_baseline("data", &accepts_data(DATA));
}
