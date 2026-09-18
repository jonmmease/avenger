use datafusion::logical_expr::{lit, Expr};

use crate::{
    predicate::{combine, contribution, tuples_predicate},
    DirectReason, EmptySelection, ProducerDefinition, Resolution, ResolvedFilter, SelectionMode,
    SelectionStatus, SelectionValue, ViewAddress,
};

/// A proved conjunction. Only `fixed` may influence the materialization.
#[derive(Clone, Debug)]
pub(crate) struct PredicateSplit {
    pub fixed: Expr,
    pub changing: Expr,
}

fn uses_focus(tree: &ResolvedFilter, focus: &ProducerDefinition, view: &ViewAddress) -> bool {
    match tree {
        ResolvedFilter::Selection(s) => {
            s.definition().id() == &focus.address().selection
                && !(s.usage().mode == SelectionMode::CrossFilter
                    && &focus.address().origin == view)
        }
        ResolvedFilter::All(xs) | ResolvedFilter::Any(xs) => {
            xs.iter().any(|x| uses_focus(x, focus, view))
        }
        ResolvedFilter::Not(x) => uses_focus(x, focus, view),
    }
}

pub(crate) fn split(
    tree: &ResolvedFilter,
    focus: &ProducerDefinition,
    view: &ViewAddress,
    keys: &[Expr],
) -> std::result::Result<PredicateSplit, DirectReason> {
    if !uses_focus(tree, focus, view) {
        return Err(DirectReason::FocusNotUsed);
    }
    factor(tree, focus, view, keys)
}

fn factor(
    tree: &ResolvedFilter,
    focus: &ProducerDefinition,
    view: &ViewAddress,
    keys: &[Expr],
) -> std::result::Result<PredicateSplit, DirectReason> {
    if !uses_focus(tree, focus, view) {
        return Ok(PredicateSplit {
            fixed: tree.predicate(),
            changing: lit(true),
        });
    }
    match tree {
        ResolvedFilter::All(xs) => {
            let parts = xs
                .iter()
                .map(|x| factor(x, focus, view, keys))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(PredicateSplit {
                fixed: combine(parts.iter().map(|p| p.fixed.clone()), true),
                changing: combine(parts.into_iter().map(|p| p.changing), true),
            })
        }
        ResolvedFilter::Selection(s) => {
            // Global toggles can retain several origins. A single current
            // contribution does not prove future conjunction semantics.
            if s.definition().resolution() != Resolution::Intersect {
                return Err(DirectReason::UnsupportedFactorization);
            }
            let mut fixed = Vec::new();
            let mut changing = lit(true);
            for c in s.contributions() {
                if c.contribution().producer().address() == focus.address() {
                    if c.contribution().producer() != focus {
                        return Err(DirectReason::IncompatibleFocus);
                    }
                    let SelectionValue::Tuples(tuples) = c.contribution().effective_value() else {
                        return Err(DirectReason::UnsupportedInteraction);
                    };
                    changing = tuples_predicate(tuples, keys);
                } else {
                    fixed.push(contribution(c));
                }
            }
            // Inactivity affects this read, not the coverage of the summary.
            // In particular, MatchNone can warm all cells before the first brush.
            if s.status() == SelectionStatus::Inactive {
                changing = lit(s.usage().empty == EmptySelection::MatchAll);
            }
            Ok(PredicateSplit {
                fixed: combine(fixed, true),
                changing,
            })
        }
        ResolvedFilter::Any(_) | ResolvedFilter::Not(_) => {
            Err(DirectReason::UnsupportedFactorization)
        }
    }
}
