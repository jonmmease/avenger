mod aggregates;
mod expressions;
pub(crate) mod predicates;

use std::{cmp::Ordering, fmt, sync::Arc};

use datafusion::{
    common::{
        tree_node::{Transformed, TreeNode, TreeNodeRecursion},
        DFSchemaRef, Result as DFResult,
    },
    logical_expr::{
        col, Aggregate, Expr, ExprSchemable, Extension, LogicalPlan, LogicalPlanBuilder,
        Projection, UserDefinedLogicalNodeCore, Volatility,
    },
};

use crate::{
    query::{selection_site, with_predicate, SelectionSite},
    DirectReason, Error, Result,
};
use aggregates::AggregateRewrite;

pub(crate) type Eligibility<T> = std::result::Result<T, DirectReason>;

/// Prepared logical templates. Binding substitutes predicates and relations,
/// without rediscovering aggregate states or rewriting expression lineage.
#[derive(Clone, Debug)]
pub(crate) struct Preaggregation {
    materialization: LogicalPlan,
    aggregate: LogicalPlan,
    site: u64,
    pub interaction_keys: Vec<Expr>,
}

impl Preaggregation {
    pub fn analyze(plan: &LogicalPlan, site: u64, keys: Vec<Expr>) -> Result<Eligibility<Self>> {
        let mut sites = 0;
        plan.apply_with_subqueries(|p| {
            if selection_site(p).is_some_and(|s| s.id == site) {
                sites += 1;
            }
            Ok(TreeNodeRecursion::Continue)
        })?;
        if sites != 1 {
            return Ok(Err(DirectReason::UnsupportedQueryShape));
        }
        let original = match recognize_aggregate(plan, site) {
            Ok(aggregate) => aggregate,
            Err(reason) => return Ok(Err(reason)),
        };
        if !immutable_plan(plan)? {
            return Ok(Err(DirectReason::NonImmutableQuery));
        }
        if original
            .group_expr
            .iter()
            .any(|e| matches!(e, Expr::GroupingSet(_)))
        {
            return Ok(Err(DirectReason::UnsupportedQueryShape));
        }
        if !original
            .group_expr
            .iter()
            .chain(&keys)
            .all(|expr| expressions::safe_grouping(expr, original.input.schema()))
        {
            return Ok(Err(DirectReason::UnsupportedGroupingExpression));
        }
        let mut rewrites = Vec::with_capacity(original.aggr_expr.len());
        for (i, expr) in original.aggr_expr.iter().enumerate() {
            match AggregateRewrite::analyze(expr, i, original.input.schema())? {
                Ok(rewrite) => rewrites.push(rewrite),
                Err(reason) => return Ok(Err(reason)),
            }
        }
        if rewrites.is_empty() {
            return Ok(Err(DirectReason::UnsupportedAggregate));
        }
        let display: Vec<_> = (0..original.group_expr.len())
            .map(|i| format!("__selection_display_{i}"))
            .collect();
        let interaction: Vec<_> = (0..keys.len())
            .map(|i| format!("__selection_key_{i}"))
            .collect();
        let groups = original
            .group_expr
            .iter()
            .cloned()
            .zip(&display)
            .chain(keys.into_iter().zip(&interaction))
            .map(|(expr, name)| expr.alias(name))
            .collect::<Vec<_>>();
        let materialization = LogicalPlanBuilder::from(original.input.as_ref().clone())
            .aggregate(
                groups,
                rewrites
                    .iter()
                    .map(|rewrite| rewrite.state.clone())
                    .collect::<Vec<_>>(),
            )?
            .build()?;
        let materialized = LogicalPlan::Extension(Extension {
            node: Arc::new(MaterializationSite {
                id: site,
                schema: materialization.schema().clone(),
            }),
        });
        let selected = LogicalPlan::Extension(Extension {
            node: Arc::new(SelectionSite {
                id: site,
                input: materialized,
            }),
        });
        let merged = LogicalPlanBuilder::from(selected)
            .aggregate(
                display.iter().map(col).collect::<Vec<_>>(),
                rewrites
                    .iter()
                    .map(|rewrite| rewrite.merge.clone())
                    .collect::<Vec<_>>(),
            )?
            .build()?;
        let result_columns: Vec<_> = display
            .iter()
            .map(col)
            .chain((0..rewrites.len()).map(|i| col(format!("__selection_merge_{i}"))))
            .collect();
        for (expr, (_, expected)) in result_columns.iter().zip(original.schema.iter()) {
            let (_, actual) = expr.to_field(merged.schema())?;
            if actual.data_type() != expected.data_type()
                || actual.is_nullable() != expected.is_nullable()
            {
                return Ok(Err(DirectReason::UnsupportedAggregate));
            }
        }
        let exprs = result_columns
            .into_iter()
            .zip(original.schema.iter())
            .map(|(expr, (qualifier, field))| {
                expr.alias_qualified(qualifier.cloned(), field.name())
            })
            .collect();
        let restored = LogicalPlan::Projection(Projection::try_new_with_schema(
            exprs,
            Arc::new(merged),
            original.schema.clone(),
        )?);
        let aggregate = replace_aggregate(plan, restored)?;
        Ok(Ok(Self {
            materialization,
            aggregate,
            site,
            interaction_keys: interaction.iter().map(col).collect(),
        }))
    }

    pub fn materialization(&self, fixed: Expr) -> Result<LogicalPlan> {
        with_predicate(&self.materialization, self.site, fixed)
    }

    pub fn aggregate(&self, changing: Expr) -> Result<AggregateStep> {
        Ok(AggregateStep {
            plan: Arc::new(with_predicate(&self.aggregate, self.site, changing)?),
            schema: self.materialization.schema().clone(),
            site: self.site,
        })
    }
}

/// The final filter, aggregate-state merge, and unchanged query suffix.
/// Supply the materialization's relation with `over` to get an executable plan.
#[derive(Clone, Debug)]
pub struct AggregateStep {
    plan: Arc<LogicalPlan>,
    schema: DFSchemaRef,
    site: u64,
}

impl AggregateStep {
    /// Return the expected materialization schema, including retained keys and state.
    pub fn schema(&self) -> &DFSchemaRef {
        &self.schema
    }

    /// Substitute a native relation or named dataflow reference. Field names,
    /// order, types, and nullability must match the materialization. Relation
    /// qualifiers may differ. No data is read and no functions are evaluated.
    pub fn over(&self, materialized: LogicalPlan) -> Result<LogicalPlan> {
        if materialized.schema().as_arrow() != self.schema.as_arrow() {
            return Err(Error::InvalidQuery(
                "materialized relation does not match the aggregate step's schema".into(),
            ));
        }
        // Remove the caller's qualifiers while retaining the checked field schema.
        let relation = LogicalPlan::Projection(Projection::try_new_with_schema(
            materialized
                .schema()
                .columns()
                .into_iter()
                .map(Expr::Column)
                .collect(),
            Arc::new(materialized),
            self.schema.clone(),
        )?);
        Ok(self
            .plan
            .as_ref()
            .clone()
            .transform_up_with_subqueries(|plan| {
                if let LogicalPlan::Extension(e) = &plan {
                    if e.node
                        .as_any()
                        .downcast_ref::<MaterializationSite>()
                        .is_some_and(|m| m.id == self.site)
                    {
                        return Ok(Transformed::yes(relation.clone()));
                    }
                }
                Ok(Transformed::no(plan))
            })?
            .data)
    }
}

fn recognize_aggregate(plan: &LogicalPlan, site: u64) -> Eligibility<&Aggregate> {
    match plan {
        LogicalPlan::Aggregate(a) => {
            let mut input = a.input.as_ref();
            // Fixed application filters can remain at their original position.
            while let LogicalPlan::Filter(f) = input {
                input = f.input.as_ref();
            }
            if selection_site(input).is_some_and(|s| s.id == site) {
                Ok(a)
            } else {
                Err(DirectReason::UnsupportedQueryShape)
            }
        }
        LogicalPlan::Projection(p) => recognize_aggregate(&p.input, site),
        LogicalPlan::Filter(f) => recognize_aggregate(&f.input, site),
        LogicalPlan::Sort(s) if s.fetch.is_none() => recognize_aggregate(&s.input, site),
        LogicalPlan::SubqueryAlias(a) => recognize_aggregate(&a.input, site),
        _ => Err(DirectReason::UnsupportedQueryShape),
    }
}

fn replace_aggregate(plan: &LogicalPlan, replacement: LogicalPlan) -> DFResult<LogicalPlan> {
    if matches!(plan, LogicalPlan::Aggregate(_)) {
        return Ok(replacement);
    }
    let input = replace_aggregate(plan.inputs()[0], replacement)?;
    plan.with_new_exprs(plan.expressions(), vec![input])
}

fn immutable_plan(plan: &LogicalPlan) -> DFResult<bool> {
    let mut immutable = true;
    plan.apply_with_subqueries(|p| {
        for expr in p.expressions() {
            expr.apply(|e| {
                let volatility = match e {
                    Expr::ScalarFunction(f) => Some(f.func.signature().volatility),
                    Expr::AggregateFunction(f) => Some(f.func.signature().volatility),
                    Expr::WindowFunction(f) => Some(f.fun.signature().volatility),
                    Expr::HigherOrderFunction(f) => Some(f.func.signature().volatility),
                    _ => None,
                };
                if volatility.is_some_and(|v| v != Volatility::Immutable) {
                    immutable = false;
                }
                Ok(TreeNodeRecursion::Continue)
            })?;
        }
        Ok(TreeNodeRecursion::Continue)
    })?;
    Ok(immutable)
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct MaterializationSite {
    id: u64,
    schema: DFSchemaRef,
}
impl PartialOrd for MaterializationSite {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.id
            .partial_cmp(&other.id)
            .filter(|order| *order != Ordering::Equal || self == other)
    }
}
impl UserDefinedLogicalNodeCore for MaterializationSite {
    fn name(&self) -> &str {
        "SelectionMaterializationSite"
    }
    fn inputs(&self) -> Vec<&LogicalPlan> {
        vec![]
    }
    fn schema(&self) -> &DFSchemaRef {
        &self.schema
    }
    fn expressions(&self) -> Vec<Expr> {
        vec![]
    }
    fn fmt_for_explain(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SelectionMaterializationSite: {}", self.id)
    }
    fn with_exprs_and_inputs(&self, exprs: Vec<Expr>, inputs: Vec<LogicalPlan>) -> DFResult<Self> {
        if !exprs.is_empty() || !inputs.is_empty() {
            return datafusion::common::internal_err!(
                "materialization site is a leaf without expressions"
            );
        }
        Ok(self.clone())
    }
}
