use crate::{
    aggregates::AggregateRewrite,
    expressions,
    markers::{self, MaterializationSite},
    DirectReason, Eligibility, ExpressionProperties, FilterQuery,
};
use datafusion::{
    common::{
        tree_node::{Transformed, TreeNode, TreeNodeRecursion},
        DFSchema, DFSchemaRef, Result,
    },
    logical_expr::{
        col, Aggregate, Expr, ExprSchemable, Extension, LogicalPlan, LogicalPlanBuilder, Projection,
    },
};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(crate) struct Preaggregation {
    pub materialization: LogicalPlan,
    aggregate: LogicalPlan,
    site: u64,
    dimensions: Vec<(Expr, Expr)>,
    pub grouping: Vec<Expr>,
    pub states: Vec<Expr>,
}
impl Preaggregation {
    pub fn analyze(
        query: &FilterQuery,
        keys: &[Expr],
        properties: &Arc<dyn ExpressionProperties>,
    ) -> Result<Eligibility<Self>> {
        let mut sites = 0;
        let mut immutable = true;
        query.plan.apply_with_subqueries(|p| {
            if markers::site(p).is_some_and(|s| s.id == query.site) {
                sites += 1;
            }
            for expr in p.expressions() {
                immutable &= expressions::immutable(&expr)?;
            }
            Ok(TreeNodeRecursion::Continue)
        })?;
        if sites != 1 {
            return Ok(Err(DirectReason::UnsupportedQueryShape));
        }
        if !immutable {
            return Ok(Err(DirectReason::NonImmutableQuery));
        }
        let (original, suffix) = match target(&query.plan, query.site) {
            Ok(v) => v,
            Err(r) => return Ok(Err(r)),
        };
        let schema = original.input.schema();
        let mut input = original.input.as_ref();
        while markers::site(input).is_none() {
            match input {
                LogicalPlan::Filter(f) => {
                    let predicate =
                        expressions::boolean(f.predicate.clone(), f.input.schema(), false)?;
                    if !expressions::properties(&predicate, f.input.schema(), properties).total {
                        return Ok(Err(DirectReason::UnsafeMovedExpression));
                    }
                    input = &f.input;
                }
                LogicalPlan::SubqueryAlias(a) => input = &a.input,
                _ => return Ok(Err(DirectReason::UnsupportedQueryShape)),
            }
        }
        let grouping = original
            .group_expr
            .iter()
            .cloned()
            .map(|e| expressions::coerce(e, schema))
            .collect::<Result<Vec<_>>>()?;
        if grouping.iter().any(|e| matches!(e, Expr::GroupingSet(_))) {
            return Ok(Err(DirectReason::UnsupportedQueryShape));
        }
        let keys = keys
            .iter()
            .cloned()
            .map(|e| expressions::requalify(e, &query.source_schema, schema))
            .collect::<Result<Vec<_>>>()?;
        if !grouping
            .iter()
            .chain(&keys)
            .all(|e| expressions::properties(e, schema, properties).total)
        {
            return Ok(Err(DirectReason::UnsupportedGroupingExpression));
        }
        let mut rewrites = Vec::new();
        for (i, expr) in original.aggr_expr.iter().enumerate() {
            match AggregateRewrite::analyze(expr, i, schema, properties)? {
                Ok(r) => rewrites.push(r),
                Err(r) => return Ok(Err(r)),
            }
        }
        if rewrites.is_empty() {
            return Ok(Err(DirectReason::UnsupportedAggregate));
        }
        let mut dimensions: Vec<(Expr, Expr)> = Vec::new();
        let mut groups = Vec::new();
        let mut display = Vec::new();
        for expr in grouping.iter().chain(&keys) {
            let normalized = expressions::canonical(expr.clone(), schema)?;
            let stored =
                if let Some((_, column)) = dimensions.iter().find(|(e, _)| *e == normalized) {
                    column.clone()
                } else {
                    let name = format!("__preagg_dimension_{}", dimensions.len());
                    let column = col(&name);
                    dimensions.push((normalized, column.clone()));
                    // Strip presentation aliases before assigning private storage names.
                    let expr = match expr {
                        Expr::Alias(a) => a.expr.as_ref().clone(),
                        e => e.clone(),
                    };
                    groups.push(expr.alias(name));
                    column
                };
            if display.len() < grouping.len() {
                display.push(stored);
            }
        }
        let states = rewrites.iter().map(|r| r.state.clone()).collect::<Vec<_>>();
        let materialization =
            LogicalPlanBuilder::from(markers::substitute(&original.input, query.site, None)?)
                .aggregate(groups, states.clone())?
                .build()?;
        let materialized = LogicalPlan::Extension(Extension {
            node: Arc::new(MaterializationSite {
                id: query.site,
                schema: materialization.schema().clone(),
            }),
        });
        let selected = markers::rows(query.site, materialized);
        let merged = LogicalPlanBuilder::from(selected)
            .aggregate(
                display.clone(),
                rewrites.iter().map(|r| r.merge.clone()).collect::<Vec<_>>(),
            )?
            .build()?;
        let result_columns = display
            .into_iter()
            .chain((0..rewrites.len()).map(|i| col(format!("__preagg_merge_{i}"))))
            .collect::<Vec<_>>();
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
            .map(|(e, (q, f))| e.alias_qualified(q.cloned(), f.name()))
            .collect();
        let mut aggregate = LogicalPlan::Projection(Projection::try_new_with_schema(
            exprs,
            Arc::new(merged),
            original.schema.clone(),
        )?);
        for ancestor in suffix.into_iter().rev() {
            aggregate = ancestor.with_new_exprs(ancestor.expressions(), vec![aggregate])?;
            if aggregate.schema() != ancestor.schema() {
                return Ok(Err(DirectReason::UnsupportedQueryShape));
            }
        }
        Ok(Ok(Self {
            materialization,
            aggregate,
            site: query.site,
            dimensions,
            grouping,
            states,
        }))
    }
    pub fn rollup(&self, changing: Expr) -> Result<RollupQuery> {
        Ok(RollupQuery {
            plan: Arc::new(markers::substitute(
                &self.aggregate,
                self.site,
                Some(changing),
            )?),
            schema: self.materialization.schema().clone(),
            site: self.site,
        })
    }
    pub fn predicate(
        &self,
        source: &Expr,
        schema: &DFSchema,
        policy: &Arc<dyn ExpressionProperties>,
    ) -> Result<Eligibility<Expr>> {
        if !expressions::immutable(source)? {
            return Ok(Err(DirectReason::NonImmutableQuery));
        }
        let source = expressions::canonical(source.clone(), schema)?;
        let mut missing = false;
        fn replace(expr: Expr, dimensions: &[(Expr, Expr)], missing: &mut bool) -> Result<Expr> {
            if let Some((_, stored)) = dimensions.iter().find(|(e, _)| *e == expr) {
                return Ok(stored.clone());
            }
            if matches!(expr, Expr::Column(_)) {
                *missing = true;
            }
            Ok(expr
                .map_children(|e| Ok(Transformed::yes(replace(e, dimensions, missing)?)))?
                .data)
        }
        let retained = replace(source, &self.dimensions, &mut missing)?;
        if missing {
            return Ok(Err(DirectReason::PredicateNeedsUnretainedExpression));
        }
        let p = expressions::properties(&retained, self.materialization.schema(), policy);
        if !p.total || !p.respects_grouping_equality {
            return Ok(Err(DirectReason::UnsupportedPredicate));
        }
        Ok(Ok(retained))
    }
}

// The chain is collected from root to marker, then searched from the marker upward.
// This preserves the nearest aggregate even when an outer aggregate has a state recipe.
fn target(plan: &LogicalPlan, site: u64) -> Eligibility<(&Aggregate, Vec<&LogicalPlan>)> {
    let mut chain = Vec::new();
    let mut current = plan;
    loop {
        if markers::site(current).is_some_and(|s| s.id == site) {
            break;
        }
        let inputs = current.inputs();
        if inputs.len() != 1 {
            return Err(DirectReason::UnsupportedQueryShape);
        }
        match current {
            LogicalPlan::Projection(_)
            | LogicalPlan::Filter(_)
            | LogicalPlan::Sort(_)
            | LogicalPlan::SubqueryAlias(_)
            | LogicalPlan::Aggregate(_)
            | LogicalPlan::Window(_)
            | LogicalPlan::Limit(_) => {}
            _ => return Err(DirectReason::UnsupportedQueryShape),
        }
        // Expression subqueries above the source need separate lineage analysis.
        if current.expressions().iter().any(|e| {
            e.exists(|e| {
                Ok(matches!(
                    e,
                    Expr::ScalarSubquery(_) | Expr::InSubquery(_) | Expr::Exists(_)
                ))
            })
            .unwrap_or(true)
        }) {
            return Err(DirectReason::UnsupportedQueryShape);
        }
        chain.push(current);
        current = inputs[0];
    }
    let Some(index) = chain
        .iter()
        .rposition(|p| matches!(p, LogicalPlan::Aggregate(_)))
    else {
        return Err(DirectReason::UnsupportedQueryShape);
    };
    if chain[index + 1..]
        .iter()
        .any(|p| !matches!(p, LogicalPlan::Filter(_) | LogicalPlan::SubqueryAlias(_)))
    {
        return Err(DirectReason::UnsupportedQueryShape);
    }
    let LogicalPlan::Aggregate(aggregate) = chain[index] else {
        unreachable!()
    };
    Ok((aggregate, chain[..index].to_vec()))
}

/// The cell filter, state merge, and original finishing query.
#[derive(Clone, Debug)]
pub struct RollupQuery {
    plan: Arc<LogicalPlan>,
    schema: DFSchemaRef,
    site: u64,
}
impl RollupQuery {
    /// Return the required input schema of the stored materialization.
    pub fn materialization_schema(&self) -> &DFSchemaRef {
        &self.schema
    }
    /// Substitute a relation with matching Arrow fields and metadata. Qualifiers
    /// may differ. This validates schema, not data provenance or source versions.
    /// Passing a construction plan here recomputes that plan during execution.
    pub fn with_materialization(&self, relation: LogicalPlan) -> Result<LogicalPlan> {
        if relation.schema().as_arrow() != self.schema.as_arrow() {
            return datafusion::common::plan_err!(
                "materialized relation does not match the rollup input schema"
            );
        }
        let relation = LogicalPlan::Projection(Projection::try_new_with_schema(
            relation
                .schema()
                .columns()
                .into_iter()
                .map(Expr::Column)
                .collect(),
            Arc::new(relation),
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
