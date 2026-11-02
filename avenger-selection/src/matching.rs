//! Which rows selections select: the predicate of a plot over the clauses of its selections, as
//! a [`Matching`] whose leaves are the conditions of the clauses. The renderer
//! evaluates a matching to masks, and a query translates it to a filter, so both select the same
//! rows.

use std::collections::BTreeMap;

use crate::{
    clause::{and, or},
    Clause, Condition,
};

/// How the clauses of a selection's owners combine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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

/// The rows that the selection named `selection` selects.
#[derive(Clone, Debug, PartialEq)]
pub struct Selected {
    pub selection: String,
    /// Whether a selection without clauses selects no row, rather than every row.
    pub empty_selects_none: bool,
    /// Whether the predicate skips the clauses that the row's own plot wrote.
    pub exclude_own: bool,
}

/// The rows that a predicate selects, as a tree whose leaves are the conditions of clauses.
#[derive(Clone, Debug)]
pub enum Matching<'a> {
    /// Every row.
    All,
    /// No row.
    Nothing,
    /// The rows that every part matches.
    Every(Vec<Matching<'a>>),
    /// The rows that some part matches.
    Any(Vec<Matching<'a>>),
    /// The rows that the part does not match.
    Not(Box<Matching<'a>>),
    /// The rows that match a condition of a clause of `selection`.
    Condition {
        selection: &'a str,
        condition: &'a Condition,
    },
}

/// The rows that `predicate` selects, over `clauses`, the clauses of the selections by name, and
/// `resolutions`, the resolution of each selection, which is global when a selection has none.
/// `owned` says whether the row's plot wrote a clause, which a [`Selected`] with `exclude_own`
/// skips.
///
/// A selection without clauses selects every row, or none with `empty_selects_none`. A selection
/// whose clauses the predicate skips, all of them, selects every row. Otherwise a row matches an
/// intersect selection when it matches the clause of every owner, and a global or a union
/// selection when it matches the clause of some owner. A row matches a clause when it matches
/// every condition of the clause.
pub fn matching<'a>(
    predicate: &Predicate,
    resolutions: &BTreeMap<String, Resolution>,
    clauses: &'a BTreeMap<String, Vec<Clause>>,
    owned: &dyn Fn(&Clause) -> bool,
) -> Matching<'a> {
    let parts = |predicates: &[Predicate]| -> Vec<Matching<'a>> {
        (predicates.iter())
            .map(|inner| matching(inner, resolutions, clauses, owned))
            .collect()
    };
    match predicate {
        Predicate::Selected(selected) => selection_matching(selected, resolutions, clauses, owned),
        Predicate::And(predicates) => Matching::Every(parts(predicates)),
        Predicate::Or(predicates) => Matching::Any(parts(predicates)),
        Predicate::Not(inner) => {
            Matching::Not(Box::new(matching(inner, resolutions, clauses, owned)))
        }
    }
}

/// The rows that the selection of `selected` selects, as [`matching`] describes.
fn selection_matching<'a>(
    selected: &Selected,
    resolutions: &BTreeMap<String, Resolution>,
    clauses: &'a BTreeMap<String, Vec<Clause>>,
    owned: &dyn Fn(&Clause) -> bool,
) -> Matching<'a> {
    let Some((selection, clauses)) = clauses
        .get_key_value(&selected.selection)
        .filter(|(_, clauses)| !clauses.is_empty())
    else {
        return if selected.empty_selects_none {
            Matching::Nothing
        } else {
            Matching::All
        };
    };
    let kept: Vec<Matching<'a>> = (clauses.iter())
        .filter(|clause| !(selected.exclude_own && owned(clause)))
        .map(|clause| {
            Matching::Every(
                (clause.conditions.iter())
                    .map(|condition| Matching::Condition {
                        selection,
                        condition,
                    })
                    .collect(),
            )
        })
        .collect();
    if kept.is_empty() {
        return Matching::All;
    }
    if resolutions.get(selection) == Some(&Resolution::Intersect) {
        Matching::Every(kept)
    } else {
        Matching::Any(kept)
    }
}

impl Matching<'_> {
    /// Whether each of `len` rows matches, with `matches` giving whether each row matches a
    /// condition of a clause of a selection, such as by [`Condition::matches`] over the rows'
    /// columns.
    pub fn mask<E, F>(&self, len: usize, matches: &mut F) -> std::result::Result<Vec<bool>, E>
    where
        F: FnMut(&str, &Condition) -> std::result::Result<Vec<bool>, E>,
    {
        Ok(match self {
            Matching::All => vec![true; len],
            Matching::Nothing => vec![false; len],
            Matching::Every(parts) => {
                let mut mask = vec![true; len];
                for part in parts {
                    and(&mut mask, &part.mask(len, matches)?);
                }
                mask
            }
            Matching::Any(parts) => {
                let mut mask = vec![false; len];
                for part in parts {
                    or(&mut mask, &part.mask(len, matches)?);
                }
                mask
            }
            Matching::Not(part) => (part.mask(len, matches)?.into_iter())
                .map(|matched| !matched)
                .collect(),
            Matching::Condition {
                selection,
                condition,
            } => matches(selection, condition)?,
        })
    }
}
