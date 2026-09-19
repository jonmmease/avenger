use crate::{
    definitions::row_expr, Contribution, Error, ProducerAddress, ProjectionId, Resolution, Result,
    RowIdentity, SelectionId, SelectionSet, SelectionValue, ViewAddress,
};
use datafusion::logical_expr::Expr;
use std::{collections::BTreeMap, sync::Arc};

/// Whether a named use retains or excludes its consuming view's producers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionMode {
    Membership,
    CrossFilter,
}
/// Membership when a named selection has no active producers anywhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmptySelection {
    MatchAll,
    MatchNone,
}
/// Consumer-local membership and inactive-selection policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectionUse {
    pub mode: SelectionMode,
    pub empty: EmptySelection,
}
/// Boolean composition of explicitly named selection uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectionFilter {
    Selection {
        id: SelectionId,
        usage: SelectionUse,
    },
    All(Vec<Self>),
    Any(Vec<Self>),
    Not(Box<Self>),
}
impl SelectionFilter {
    /// Intersect named cross-filters, treating inactive selections as unrestricted.
    pub fn cross_filter<'a>(ids: impl IntoIterator<Item = &'a SelectionId>) -> Self {
        Self::All(
            ids.into_iter()
                .map(|id| Self::Selection {
                    id: id.clone(),
                    usage: SelectionUse {
                        mode: SelectionMode::CrossFilter,
                        empty: EmptySelection::MatchAll,
                    },
                })
                .collect(),
        )
    }
    /// Apply a named selection without excluding any originating view.
    pub fn membership(id: &SelectionId, empty: EmptySelection) -> Self {
        Self::Selection {
            id: id.clone(),
            usage: SelectionUse {
                mode: SelectionMode::Membership,
                empty,
            },
        }
    }
}

/// A consuming view and compiler-verified mappings into its row relation.
#[derive(Clone, Debug)]
pub struct ConsumerFilter {
    view: ViewAddress,
    filter: SelectionFilter,
    projections: BTreeMap<(ProducerAddress, ProjectionId), Expr>,
    identities: BTreeMap<RowIdentity, Expr>,
}
impl ConsumerFilter {
    /// Use producer expressions directly when the consumer shares their row relation.
    pub fn new(view: ViewAddress, filter: SelectionFilter) -> Self {
        Self {
            view,
            filter,
            projections: BTreeMap::new(),
            identities: BTreeMap::new(),
        }
    }
    /// Return the semantic view instance used for self-filter exclusion.
    pub fn view(&self) -> &ViewAddress {
        &self.view
    }
    /// Map one producer-local projection to a verified equivalent consumer expression.
    /// The chart compiler owns lineage validation. Other projections retain their expressions.
    pub fn with_projection(
        mut self,
        producer: &ProducerAddress,
        projection: &ProjectionId,
        expr: Expr,
    ) -> Result<Self> {
        let key = (producer.clone(), projection.clone());
        if self.projections.contains_key(&key) {
            return Err(Error::InvalidMapping(format!(
                "duplicate projection mapping for {}",
                projection
            )));
        }
        self.projections.insert(key, row_expr(expr)?);
        Ok(self)
    }
    /// Declare an identity-preserving ID expression for this exact lineage.
    /// Joins, aggregates, and regenerated IDs require the compiler to verify preservation.
    pub fn with_row_identity(mut self, identity: &RowIdentity, expr: Expr) -> Result<Self> {
        if self.identities.contains_key(identity) {
            return Err(Error::InvalidMapping(
                "duplicate row-identity mapping".into(),
            ));
        }
        self.identities.insert(identity.clone(), row_expr(expr)?);
        Ok(self)
    }

    /// Resolve definitions even before the producer has an active contribution.
    pub(crate) fn interaction_keys(&self, producer: &crate::ProducerDefinition) -> Vec<Expr> {
        producer
            .projections()
            .iter()
            .map(|p| {
                let expr = self
                    .projections
                    .get(&(producer.address().clone(), p.id().clone()))
                    .unwrap_or(p.expr())
                    .clone();
                producer
                    .pixel_grid(p.id())
                    .map_or(expr.clone(), |grid| grid.cell_expr(expr))
            })
            .collect()
    }

    /// Resolve all named uses, including branches that currently determine no rows.
    pub(crate) fn resolve(&self, selections: &SelectionSet) -> Result<ResolvedFilter> {
        resolve(&self.filter, self, selections)
    }
    /// Produce a non-null Boolean expression for use in a DataFusion filter or Expr input.
    pub fn predicate(&self, selections: &SelectionSet) -> Result<Expr> {
        Ok(self.resolve(selections)?.predicate())
    }
}

/// A named-use tree after consumer mappings and self-exclusion.
#[derive(Clone, Debug)]
pub(crate) enum ResolvedFilter {
    Selection(ResolvedSelection),
    All(Vec<Self>),
    Any(Vec<Self>),
    Not(Box<Self>),
}
impl ResolvedFilter {
    /// Lower the resolved tree using the same semantics as ConsumerFilter::predicate.
    pub(crate) fn predicate(&self) -> Expr {
        crate::predicate::resolved(self)
    }
}
/// Distinguish inactivity, self-exclusion, and active (possibly empty) values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SelectionStatus {
    Inactive,
    AllExcluded,
    Active,
}
/// One resolved use of a named selection.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedSelection {
    pub(crate) id: SelectionId,
    pub(crate) resolution: Resolution,
    pub(crate) usage: SelectionUse,
    pub(crate) status: SelectionStatus,
    pub(crate) contributions: Vec<ResolvedContribution>,
}

/// A retained contribution and its checked mappings into the consumer relation.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedContribution {
    pub(crate) contribution: Arc<Contribution>,
    pub(crate) projections: Vec<Expr>,
    pub(crate) identity_expr: Option<Expr>,
}

fn resolve(
    filter: &SelectionFilter,
    consumer: &ConsumerFilter,
    selections: &SelectionSet,
) -> Result<ResolvedFilter> {
    Ok(match filter {
        SelectionFilter::All(filters) => ResolvedFilter::All(
            filters
                .iter()
                .map(|f| resolve(f, consumer, selections))
                .collect::<Result<_>>()?,
        ),
        SelectionFilter::Any(filters) => ResolvedFilter::Any(
            filters
                .iter()
                .map(|f| resolve(f, consumer, selections))
                .collect::<Result<_>>()?,
        ),
        SelectionFilter::Not(filter) => {
            ResolvedFilter::Not(Box::new(resolve(filter, consumer, selections)?))
        }
        SelectionFilter::Selection { id, usage } => {
            let snapshot = selections.get(id)?;
            let mut contributions = Vec::new();
            for (address, contribution) in snapshot.contributions.iter() {
                if usage.mode == SelectionMode::CrossFilter && address.origin == consumer.view {
                    continue;
                }
                let projections = contribution
                    .producer()
                    .projections()
                    .iter()
                    .map(|p| {
                        let raw_expr = consumer
                            .projections
                            .get(&(address.clone(), p.id().clone()))
                            .unwrap_or(p.expr())
                            .clone();
                        match contribution.producer().pixel_grid(p.id()) {
                            Some(grid) => grid.cell_expr(raw_expr),
                            None => raw_expr,
                        }
                    })
                    .collect();
                let identity_expr = if let SelectionValue::RowIds(ids) = contribution.value() {
                    Some(
                        consumer
                            .identities
                            .get(ids.identity())
                            .ok_or_else(|| {
                                Error::InvalidMapping(format!(
                                    "no identity-preserving mapping for producer {}",
                                    address.producer
                                ))
                            })?
                            .clone(),
                    )
                } else {
                    None
                };
                contributions.push(ResolvedContribution {
                    contribution: contribution.clone(),
                    projections,
                    identity_expr,
                });
            }
            let status = if snapshot.contributions.is_empty() {
                SelectionStatus::Inactive
            } else if contributions.is_empty() {
                SelectionStatus::AllExcluded
            } else {
                SelectionStatus::Active
            };
            ResolvedFilter::Selection(ResolvedSelection {
                id: id.clone(),
                resolution: snapshot.resolution,
                usage: *usage,
                status,
                contributions,
            })
        }
    })
}
