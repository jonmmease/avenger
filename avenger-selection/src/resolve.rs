use crate::{
    definitions::row_expr, Contribution, Error, ProducerAddress, ProjectionId, Result, RowIdentity,
    SelectionDefinition, SelectionId, SelectionSet, SelectionValue, ViewAddress,
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
pub struct SelectionConsumer {
    view: ViewAddress,
    projections: BTreeMap<(ProducerAddress, ProjectionId), Expr>,
    identities: BTreeMap<RowIdentity, Expr>,
}
impl SelectionConsumer {
    /// Use producer expressions directly when the consumer shares their row relation.
    pub fn new(view: ViewAddress) -> Self {
        Self {
            view,
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
}

/// Constructs reusable consumer filters without capturing current selection values.
#[derive(Clone, Copy, Debug, Default)]
pub struct SelectionCompiler;
impl SelectionCompiler {
    /// Create a stateless compiler for consumer-specific predicates.
    pub fn new() -> Self {
        Self
    }
    /// Capture a consumer and named-use tree for repeated snapshot resolution.
    pub fn filter(
        &self,
        consumer: &SelectionConsumer,
        filter: SelectionFilter,
    ) -> Result<ConsumerFilter> {
        Ok(ConsumerFilter {
            consumer: consumer.clone(),
            filter,
        })
    }
}
/// A consumer and filter definition that can be rebound to immutable selection sets.
#[derive(Clone, Debug)]
pub struct ConsumerFilter {
    consumer: SelectionConsumer,
    filter: SelectionFilter,
}
impl ConsumerFilter {
    /// Resolve all named uses, including branches that currently determine no rows.
    pub fn resolve(&self, selections: &SelectionSet) -> Result<ResolvedFilter> {
        resolve(&self.filter, &self.consumer, selections)
    }
    /// Produce a non-null Boolean expression for use in a DataFusion filter or Expr input.
    pub fn predicate(&self, selections: &SelectionSet) -> Result<Expr> {
        Ok(self.resolve(selections)?.predicate())
    }
}

/// A named-use tree with explicit exclusions and effective contributions.
#[derive(Clone, Debug)]
pub enum ResolvedFilter {
    Selection(ResolvedSelection),
    All(Vec<Self>),
    Any(Vec<Self>),
    Not(Box<Self>),
}
impl ResolvedFilter {
    /// Lower the resolved tree using the same semantics as ConsumerFilter::predicate.
    pub fn predicate(&self) -> Expr {
        crate::predicate::resolved(self)
    }
}
/// Distinguish inactivity, self-exclusion, and active (possibly empty) values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionStatus {
    Inactive,
    AllExcluded,
    Active,
}
/// One resolved use of a named selection.
#[derive(Clone, Debug)]
pub struct ResolvedSelection {
    definition: Arc<SelectionDefinition>,
    usage: SelectionUse,
    status: SelectionStatus,
    contributions: Vec<ResolvedContribution>,
    excluded: Vec<ProducerAddress>,
}
impl ResolvedSelection {
    /// Return the shared definition and resolution policy.
    pub fn definition(&self) -> &SelectionDefinition {
        &self.definition
    }
    /// Return this leaf's membership and empty policies.
    pub fn usage(&self) -> SelectionUse {
        self.usage
    }
    /// Return why the leaf is active, inactive, or unrestricted by self-exclusion.
    pub fn status(&self) -> SelectionStatus {
        self.status
    }
    /// Return retained contributions with their consumer expressions.
    pub fn contributions(&self) -> &[ResolvedContribution] {
        &self.contributions
    }
    /// Return origins removed by this use's cross-filter policy.
    pub fn excluded(&self) -> &[ProducerAddress] {
        &self.excluded
    }
}
/// A retained contribution and its checked mappings into the consumer relation.
#[derive(Clone, Debug)]
pub struct ResolvedContribution {
    contribution: Arc<Contribution>,
    projections: Vec<ResolvedProjection>,
    identity_expr: Option<Expr>,
}
impl ResolvedContribution {
    /// Return the original immutable definition and selected values.
    pub fn contribution(&self) -> &Contribution {
        &self.contribution
    }
    /// Return effective projections in producer-local ID order.
    pub fn projections(&self) -> &[ResolvedProjection] {
        &self.projections
    }
    /// Return the verified ID expression for an identity contribution.
    pub fn identity_expr(&self) -> Option<&Expr> {
        self.identity_expr.as_ref()
    }
}
/// One producer-local dimension mapped into a consumer's row relation.
#[derive(Clone, Debug)]
pub struct ResolvedProjection {
    id: ProjectionId,
    expr: Expr,
    raw_expr: Expr,
}
impl ResolvedProjection {
    /// Return the producer-local projection ID.
    pub fn id(&self) -> &ProjectionId {
        &self.id
    }
    /// Return the comparison expression, including cell mapping for pixel dimensions.
    pub fn expr(&self) -> &Expr {
        &self.expr
    }
    /// Return the consumer's row expression before any pixel mapping.
    pub fn raw_expr(&self) -> &Expr {
        &self.raw_expr
    }
}

fn resolve(
    filter: &SelectionFilter,
    consumer: &SelectionConsumer,
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
            let mut excluded = Vec::new();
            for (address, contribution) in snapshot.contributions.iter() {
                if usage.mode == SelectionMode::CrossFilter && address.origin == consumer.view {
                    excluded.push(address.clone());
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
                        let expr = match contribution.producer().pixel_grid(p.id()) {
                            Some(grid) => grid.cell_expr(raw_expr.clone()),
                            None => raw_expr.clone(),
                        };
                        ResolvedProjection {
                            id: p.id().clone(),
                            expr,
                            raw_expr,
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
                definition: snapshot.definition.clone(),
                usage: *usage,
                status,
                contributions,
                excluded,
            })
        }
    })
}
