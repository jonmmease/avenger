//! Predicates over selections: the rows that one selection selects for the options of its test,
//! or a combination of predicates.

use crate::SelectedOptions;

/// How the clauses of a selection's owners combine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Resolution {
    /// The selection holds one clause at a time, and a row matches it.
    #[default]
    Global,
    /// A row matches when it matches the clause of some owner.
    Union,
    /// A row matches when it matches the clause of every owner.
    Intersect,
}

/// A predicate over selections: the rows that one selection selects, or a combination of
/// predicates. An `Or` of no predicates selects no row, and an `And` of none selects every row.
#[derive(Clone, Debug, PartialEq)]
pub enum Predicate {
    Selected(Selected),
    And(Vec<Predicate>),
    Or(Vec<Predicate>),
    Not(Box<Predicate>),
}

/// The rows that the selection named `selection` selects for `options`.
#[derive(Clone, Debug, PartialEq)]
pub struct Selected {
    pub selection: String,
    pub options: SelectedOptions,
}

impl Predicate {
    /// The rows that the selection named `selection` selects for `options`.
    pub fn selected(selection: impl Into<String>, options: SelectedOptions) -> Self {
        Predicate::Selected(Selected {
            selection: selection.into(),
            options,
        })
    }
}
