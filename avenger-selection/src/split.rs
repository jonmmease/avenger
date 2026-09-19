use datafusion::logical_expr::{lit, Expr};

use crate::{
    predicate::{combine, contribution, tuples_predicate},
    resolve::{ResolvedFilter, SelectionStatus},
    ConsumerFilter, EmptySelection, ProducerDefinition, Resolution, Result, SelectionMode,
    SelectionSet, ViewId,
};

/// Why a valid selection predicate has no supported fixed/changing split.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SplitReason {
    /// The consumer excludes or does not use the focused producer.
    FocusNotUsed,
    /// A focus-dependent Boolean operation has no supported conjunction proof.
    UnsupportedComposition,
    /// The active focused contribution differs from the supplied definition or grid.
    IncompatibleFocus,
}
impl std::fmt::Display for SplitReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::FocusNotUsed => "consumer excludes or does not use the focus",
            Self::UnsupportedComposition => "no supported fixed/changing conjunction",
            Self::IncompatibleFocus => "focused definition or pixel grid changed",
        })
    }
}

/// Complete membership and optional focus factorization from one immutable snapshot.
#[derive(Clone, Debug)]
pub struct SelectionPredicates {
    full: Expr,
    split: std::result::Result<PredicateSplit, SplitReason>,
}
impl SelectionPredicates {
    /// Return the complete consumer predicate, including when splitting is unavailable.
    pub fn full(&self) -> &Expr {
        &self.full
    }

    /// Return a proved conjunction or its selection-specific limitation.
    pub fn split(&self) -> std::result::Result<&PredicateSplit, SplitReason> {
        self.split.as_ref().map_err(|reason| *reason)
    }
}

/// Source-row predicates whose conjunction preserves complete selection membership.
/// The query planner checks types and grouping. Expressions must also be valid
/// over the entire warm-up dataset.
#[derive(Clone, Debug)]
pub struct PredicateSplit {
    fixed: Expr,
    changing: Expr,
    dimensions: Vec<Expr>,
}
impl PredicateSplit {
    /// Return the effective predicates of other producers and non-focused branches.
    pub fn fixed(&self) -> &Expr {
        &self.fixed
    }
    /// Return focused membership in source-row space, before stored-column rewriting.
    pub fn changing(&self) -> &Expr {
        &self.changing
    }
    /// Return mapped interaction expressions, including pixel cells where configured.
    /// These cover possible focus values even before the first active contribution.
    /// The target query's own grouping dimensions are separate.
    pub fn dimensions(&self) -> &[Expr] {
        &self.dimensions
    }
}

impl ConsumerFilter {
    /// Resolve full membership and, when supported, factor it around one producer.
    ///
    /// The focus can be inactive, but its named selection must exist. Actual active
    /// contributions determine full membership even if their configuration differs
    /// from the supplied focus. Invalid names and mappings return errors.
    /// Splitting reads no data, invokes no UDF, and requires no source schema.
    pub fn predicates(
        &self,
        selections: &SelectionSet,
        focus: &ProducerDefinition,
    ) -> Result<SelectionPredicates> {
        let tree = self.resolve(selections)?;
        selections.get(&focus.address().selection)?;
        let split = split(&tree, focus, self.view(), &self.interaction_keys(focus));
        Ok(SelectionPredicates {
            full: tree.predicate(),
            split,
        })
    }
}

fn uses_focus(tree: &ResolvedFilter, focus: &ProducerDefinition, view: &ViewId) -> bool {
    match tree {
        ResolvedFilter::Selection(s) => {
            s.id == focus.address().selection
                && !(s.mode == SelectionMode::CrossFilter && &focus.address().origin == view)
        }
        ResolvedFilter::All(xs) | ResolvedFilter::Any(xs) => {
            xs.iter().any(|x| uses_focus(x, focus, view))
        }
        ResolvedFilter::Not(x) => uses_focus(x, focus, view),
    }
}

fn split(
    tree: &ResolvedFilter,
    focus: &ProducerDefinition,
    view: &ViewId,
    keys: &[Expr],
) -> std::result::Result<PredicateSplit, SplitReason> {
    if !uses_focus(tree, focus, view) {
        return Err(SplitReason::FocusNotUsed);
    }
    factor(tree, focus, view, keys)
}

fn factor(
    tree: &ResolvedFilter,
    focus: &ProducerDefinition,
    view: &ViewId,
    keys: &[Expr],
) -> std::result::Result<PredicateSplit, SplitReason> {
    if !uses_focus(tree, focus, view) {
        return Ok(PredicateSplit {
            dimensions: keys.to_vec(),
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
                dimensions: keys.to_vec(),
                fixed: combine(parts.iter().map(|p| p.fixed.clone()), true),
                changing: combine(parts.into_iter().map(|p| p.changing), true),
            })
        }
        ResolvedFilter::Selection(s) => {
            // Global toggles can retain several origins. A single current
            // contribution does not prove future conjunction semantics.
            if s.resolution != Resolution::Intersect {
                return Err(SplitReason::UnsupportedComposition);
            }
            let mut fixed = Vec::new();
            let mut changing = lit(true);
            for c in &s.contributions {
                if c.contribution.producer().address() == focus.address() {
                    if c.contribution.producer() != focus {
                        return Err(SplitReason::IncompatibleFocus);
                    }
                    changing = tuples_predicate(c.contribution.effective_value().as_tuples(), keys);
                } else {
                    fixed.push(contribution(c));
                }
            }
            // Inactivity affects this read, not the coverage of the summary.
            // In particular, MatchNone can warm all cells before the first brush.
            if s.status == SelectionStatus::Inactive {
                changing = lit(s.empty == EmptySelection::MatchAll);
            }
            Ok(PredicateSplit {
                dimensions: keys.to_vec(),
                fixed: combine(fixed, true),
                changing,
            })
        }
        ResolvedFilter::Any(_) | ResolvedFilter::Not(_) => Err(SplitReason::UnsupportedComposition),
    }
}
