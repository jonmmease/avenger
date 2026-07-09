//! Frontier walking, execution, and splice construction.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::time::SystemTime;

use arrow::datatypes::{Field, Schema, SchemaRef};
use arrow::record_batch::{RecordBatch, RecordBatchOptions};
use datafusion::catalog::default_table_source::provider_as_source;
use datafusion::dataframe::DataFrame;
use datafusion::datasource::{MemTable, TableProvider};
use datafusion::logical_expr::expr::Alias;
use datafusion::logical_expr::{Expr, LogicalPlan, LogicalPlanBuilder};
use datafusion::prelude::SessionContext;
use datafusion_common::tree_node::TreeNode;
use datafusion_common::{Column, DFSchema, Result, TableReference, internal_err};
use futures::future::{BoxFuture, FutureExt};
use futures::stream::StreamExt;

use crate::analyze::{
    expr_contains_placeholder, expr_contains_temporal_function, expr_is_volatile_deep,
    node_local_foldable,
};
use crate::optimize::prepare;
use crate::params::{assert_no_markers, collect_placeholder_ids, unwrap_held_predicates};
use crate::{
    BakeReport, BakedSubtree, PartialEvalOutput, PartialEvalPolicy, SkipReason, SkippedSubtree,
};

struct FoldResult {
    plan: LogicalPlan,
    foldable: bool,
}

struct FoldSession<'a> {
    ctx: &'a SessionContext,
    policy: &'a PartialEvalPolicy,
    registry: HashMap<LogicalPlan, usize>,
    total_bytes: usize,
    next_table_id: usize,
    baked: Vec<BakedSubtree>,
    skipped: Vec<SkippedSubtree>,
    source_tables: BTreeSet<String>,
}

impl<'a> FoldSession<'a> {
    fn new(ctx: &'a SessionContext, policy: &'a PartialEvalPolicy) -> Self {
        Self {
            ctx,
            policy,
            registry: HashMap::new(),
            total_bytes: 0,
            next_table_id: 0,
            baked: Vec::new(),
            skipped: Vec::new(),
            source_tables: BTreeSet::new(),
        }
    }

    fn fold(&mut self, plan: LogicalPlan) -> BoxFuture<'_, Result<FoldResult>> {
        async move {
            let (rebuilt, child_results) = self.fold_children(plan).await?;
            let node_ok = node_local_foldable(&rebuilt, self.policy);

            if !node_ok {
                self.record_local_skip(&rebuilt);
            }

            if node_ok && child_results.iter().all(|child| child.foldable) {
                return Ok(FoldResult {
                    plan: rebuilt,
                    foldable: true,
                });
            }

            let plan = self.bake_foldable_children(rebuilt, child_results).await?;
            Ok(FoldResult {
                plan,
                foldable: false,
            })
        }
        .boxed()
    }

    fn bake(&mut self, subtree: LogicalPlan) -> BoxFuture<'_, Result<LogicalPlan>> {
        async move {
            if let Some(index) = self.registry.get(&subtree).copied() {
                self.baked[index].occurrences += 1;
                return replacement_for(&self.baked[index], subtree.schema());
            }

            if self.total_bytes >= self.policy.max_baked_bytes_total {
                self.record_skip(&subtree, SkipReason::TotalBudgetExhausted);
                return Ok(subtree);
            }

            let display = subtree.display_indent().to_string();
            let schema = subtree.schema().clone();
            let mut stream = match DataFrame::new(self.ctx.state(), subtree.clone())
                .execute_stream()
                .await
            {
                Ok(stream) => stream,
                Err(err) => {
                    self.skipped.push(SkippedSubtree {
                        reason: SkipReason::ExecutionError {
                            message: err.to_string(),
                        },
                        subtree_display: display,
                    });
                    return Ok(subtree);
                }
            };

            let mut batches = Vec::new();
            let mut bytes = 0;
            let mut rows = 0;
            while let Some(batch) = stream.next().await {
                let batch = match batch {
                    Ok(batch) => batch,
                    Err(err) => {
                        self.skipped.push(SkippedSubtree {
                            reason: SkipReason::ExecutionError {
                                message: err.to_string(),
                            },
                            subtree_display: display,
                        });
                        return Ok(subtree);
                    }
                };
                let observed_bytes = bytes + batch.get_array_memory_size();
                if observed_bytes > self.policy.max_baked_bytes_per_subtree {
                    self.record_skip(&subtree, SkipReason::OverBudget { observed_bytes });
                    return self.fold_children_after_skip(subtree).await;
                }
                if self.total_bytes + observed_bytes > self.policy.max_baked_bytes_total {
                    self.record_skip(&subtree, SkipReason::TotalBudgetExhausted);
                    return Ok(subtree);
                }

                bytes = observed_bytes;
                rows += batch.num_rows();
                batches.push(batch);
            }

            let (physical_schema, physical_names) = physical_schema_for(schema.as_ref());
            let batches = rename_batches(&physical_schema, batches)?;
            let mem_table = Arc::new(MemTable::try_new(
                Arc::clone(&physical_schema),
                vec![batches.clone()],
            )?);

            let table_name = format!("__pe_baked_{}", self.next_table_id);
            self.next_table_id += 1;
            self.total_bytes += bytes;
            self.source_tables.extend(source_tables(&subtree));

            let baked = BakedSubtree {
                table_name,
                mem_table,
                schema: physical_schema,
                batches,
                rows,
                bytes,
                occurrences: 1,
                subtree_display: display,
            };
            let replacement = replacement_with_names(&baked, schema.as_ref(), &physical_names)?;
            let index = self.baked.len();
            self.baked.push(baked);
            self.registry.insert(subtree, index);
            Ok(replacement)
        }
        .boxed()
    }

    fn finish_report(
        self,
        residuals: &[LogicalPlan],
        fixed_params_applied: &BTreeSet<String>,
    ) -> BakeReport {
        let remaining_params = residuals
            .iter()
            .flat_map(collect_placeholder_ids)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let fixed_param_pairs = self
            .policy
            .fixed_params
            .iter()
            .filter_map(|(name, value)| {
                let normalized = normalize_param_name(name);
                fixed_params_applied
                    .contains(&normalized)
                    .then(|| (name.clone(), value.clone()))
            })
            .collect();
        let unused_fixed_params = self
            .policy
            .fixed_params
            .iter()
            .filter_map(|(name, _)| {
                let normalized = normalize_param_name(name);
                (!fixed_params_applied.contains(&normalized)).then(|| name.clone())
            })
            .collect();

        BakeReport {
            baked: self.baked,
            skipped: self.skipped,
            source_tables: self.source_tables.into_iter().collect(),
            fixed_params_applied: fixed_param_pairs,
            unused_fixed_params,
            remaining_params,
            as_of: SystemTime::now(),
        }
    }

    fn fold_children(
        &mut self,
        plan: LogicalPlan,
    ) -> BoxFuture<'_, Result<(LogicalPlan, Vec<FoldResult>)>> {
        async move {
            let exprs = plan.expressions();
            let children = plan
                .inputs()
                .into_iter()
                .map(Clone::clone)
                .collect::<Vec<_>>();
            let mut child_results = Vec::with_capacity(children.len());

            for child in children {
                child_results.push(self.fold(child).await?);
            }

            let rebuilt_inputs = child_results
                .iter()
                .map(|child| child.plan.clone())
                .collect::<Vec<_>>();
            let rebuilt = plan.with_new_exprs(exprs, rebuilt_inputs)?;
            Ok((rebuilt, child_results))
        }
        .boxed()
    }

    fn fold_children_after_skip(
        &mut self,
        plan: LogicalPlan,
    ) -> BoxFuture<'_, Result<LogicalPlan>> {
        async move {
            let (rebuilt, child_results) = self.fold_children(plan).await?;
            self.bake_foldable_children(rebuilt, child_results).await
        }
        .boxed()
    }

    fn bake_foldable_children(
        &mut self,
        plan: LogicalPlan,
        child_results: Vec<FoldResult>,
    ) -> BoxFuture<'_, Result<LogicalPlan>> {
        async move {
            if child_results.is_empty() {
                return Ok(plan);
            }

            let exprs = plan.expressions();
            let mut inputs = Vec::with_capacity(child_results.len());
            for child in child_results {
                let input = if child.foldable {
                    self.bake(child.plan).await?
                } else {
                    child.plan
                };
                inputs.push(input);
            }
            plan.with_new_exprs(exprs, inputs)
        }
        .boxed()
    }

    fn record_local_skip(&mut self, plan: &LogicalPlan) {
        if let Some(reason) = local_skip_reason(plan, self.policy) {
            self.record_skip(plan, reason);
        }
    }

    fn record_skip(&mut self, plan: &LogicalPlan, reason: SkipReason) {
        self.skipped.push(SkippedSubtree {
            reason,
            subtree_display: plan.display_indent().to_string(),
        });
    }
}

pub(crate) async fn partial_evaluate_impl(
    plan: LogicalPlan,
    ctx: &SessionContext,
    policy: &PartialEvalPolicy,
) -> Result<PartialEvalOutput> {
    let (mut residuals, report) = partial_evaluate_set_impl(vec![plan], ctx, policy).await?;
    Ok(PartialEvalOutput {
        residual: residuals.remove(0),
        report,
    })
}

pub(crate) async fn partial_evaluate_set_impl(
    plans: Vec<LogicalPlan>,
    ctx: &SessionContext,
    policy: &PartialEvalPolicy,
) -> Result<(Vec<LogicalPlan>, BakeReport)> {
    let mut session = FoldSession::new(ctx, policy);
    let mut residuals = Vec::with_capacity(plans.len());
    let mut fixed_params_applied = BTreeSet::new();

    for plan in plans {
        let prepared = prepare(plan, ctx, policy).await?;
        fixed_params_applied.extend(prepared.fixed_params_applied);
        let folded = session.fold(prepared.plan).await?;
        let residual = if folded.foldable {
            session.bake(folded.plan).await?
        } else {
            folded.plan
        };
        let residual = unwrap_held_predicates(residual)?;
        assert_no_markers(&residual);
        residuals.push(residual);
    }

    let report = session.finish_report(&residuals, &fixed_params_applied);
    Ok((residuals, report))
}

fn replacement_for(baked: &BakedSubtree, original_schema: &DFSchema) -> Result<LogicalPlan> {
    let physical_names = baked
        .schema
        .fields()
        .iter()
        .map(|field| field.name().clone())
        .collect::<Vec<_>>();
    replacement_with_names(baked, original_schema, &physical_names)
}

fn replacement_with_names(
    baked: &BakedSubtree,
    original_schema: &DFSchema,
    physical_names: &[String],
) -> Result<LogicalPlan> {
    let provider: Arc<dyn TableProvider> = baked.mem_table.clone();
    let scan = LogicalPlanBuilder::scan(
        TableReference::bare(baked.table_name.clone()),
        provider_as_source(provider),
        None,
    )?
    .build()?;
    let projection = original_schema
        .fields()
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let (qualifier, _) = original_schema.qualified_field(index);
            Expr::Alias(Alias {
                expr: Box::new(Expr::Column(Column::new_unqualified(
                    physical_names[index].clone(),
                ))),
                relation: qualifier.cloned(),
                name: field.name().clone(),
                metadata: Some(field.metadata().clone().into()),
            })
        })
        .collect::<Vec<_>>();
    let replacement = LogicalPlanBuilder::from(scan)
        .project(projection)?
        .build()?;
    let replacement = match replacement {
        LogicalPlan::Projection(mut projection) => {
            // The overwrite below restores schema-level metadata and
            // functional dependencies that Projection construction does not
            // carry. The fields themselves must already agree, or the
            // overwrite would mask a real divergence between what the
            // projection produces and what the plan claims.
            if !schemas_field_equivalent(projection.schema.as_ref(), original_schema) {
                return internal_err!(
                    "baked replacement projection schema diverged from the \
                     original subtree schema: computed {:?}, original {:?}",
                    projection.schema,
                    original_schema
                );
            }
            projection.schema = Arc::new(original_schema.clone());
            LogicalPlan::Projection(projection)
        }
        other => other,
    };
    debug_assert_eq!(replacement.schema().as_ref(), original_schema);
    Ok(replacement)
}

fn schemas_field_equivalent(computed: &DFSchema, original: &DFSchema) -> bool {
    if computed.fields().len() != original.fields().len() {
        return false;
    }
    (0..original.fields().len()).all(|index| {
        let (computed_qualifier, computed_field) = computed.qualified_field(index);
        let (original_qualifier, original_field) = original.qualified_field(index);
        computed_qualifier == original_qualifier
            && computed_field.name() == original_field.name()
            && computed_field.data_type() == original_field.data_type()
            && computed_field.is_nullable() == original_field.is_nullable()
            && computed_field.metadata() == original_field.metadata()
    })
}

fn physical_schema_for(schema: &DFSchema) -> (SchemaRef, Vec<String>) {
    let mut used = HashSet::new();
    let mut physical_names = Vec::with_capacity(schema.fields().len());
    let fields = schema
        .fields()
        .iter()
        .map(|field| {
            let physical_name = unique_field_name(field.name(), &mut used);
            physical_names.push(physical_name.clone());
            Arc::new(Field::clone(field).with_name(physical_name))
        })
        .collect::<Vec<_>>();
    (
        Arc::new(Schema::new_with_metadata(
            fields,
            schema.as_arrow().metadata().clone(),
        )),
        physical_names,
    )
}

fn unique_field_name(base: &str, used: &mut HashSet<String>) -> String {
    if used.insert(base.to_string()) {
        return base.to_string();
    }

    let mut suffix = 1;
    loop {
        let candidate = format!("{base}_{suffix}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        suffix += 1;
    }
}

fn rename_batches(schema: &SchemaRef, batches: Vec<RecordBatch>) -> Result<Vec<RecordBatch>> {
    batches
        .into_iter()
        .map(|batch| {
            if schema.fields().is_empty() {
                let options = RecordBatchOptions::new().with_row_count(Some(batch.num_rows()));
                RecordBatch::try_new_with_options(Arc::clone(schema), Vec::new(), &options)
                    .map_err(Into::into)
            } else {
                RecordBatch::try_new(Arc::clone(schema), batch.columns().to_vec())
                    .map_err(Into::into)
            }
        })
        .collect()
}

fn source_tables(plan: &LogicalPlan) -> BTreeSet<String> {
    let mut tables = BTreeSet::new();
    let _ = plan.apply(|node| {
        if let LogicalPlan::TableScan(scan) = node {
            tables.insert(scan.table_name.to_string());
        }
        Ok(datafusion_common::tree_node::TreeNodeRecursion::Continue)
    });
    tables
}

fn local_skip_reason(plan: &LogicalPlan, policy: &PartialEvalPolicy) -> Option<SkipReason> {
    if let LogicalPlan::TableScan(scan) = plan
        && (policy
            .unfoldable_tables
            .contains(&scan.table_name.to_string())
            || policy.unfoldable_tables.contains(scan.table_name.table()))
    {
        return Some(SkipReason::ExcludedTable);
    }

    let exprs = plan.expressions();
    if exprs.iter().any(expr_contains_placeholder) {
        Some(SkipReason::ContainsPlaceholders)
    } else if exprs.iter().any(expr_contains_temporal_function) {
        Some(SkipReason::TemporalFunction)
    } else if exprs.iter().any(expr_is_volatile_deep) {
        Some(SkipReason::Volatile)
    } else {
        None
    }
}

fn normalize_param_name(name: &str) -> String {
    name.trim_start_matches('$').to_string()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::collections::HashSet;

    use arrow::array::{Float64Array, Int64Array, StringArray, TimestampNanosecondArray};
    use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
    use arrow::util::display::array_value_to_string;
    use datafusion_common::ScalarValue;

    use super::*;

    fn table_batch() -> RecordBatch {
        RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("x", DataType::Float64, false),
                Field::new("region", DataType::Utf8, false),
            ])),
            vec![
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0])) as _,
                Arc::new(StringArray::from(vec!["EU", "NA", "EU", "APAC"])) as _,
            ],
        )
        .unwrap()
    }

    async fn ctx_and_plan(sql: &str) -> (SessionContext, LogicalPlan) {
        let ctx = SessionContext::new();
        ctx.register_batch("t", table_batch()).unwrap();
        ctx.register_batch("u", table_batch()).unwrap();
        let plan = ctx.sql(sql).await.unwrap().logical_plan().clone();
        (ctx, plan)
    }

    async fn rows(ctx: &SessionContext, plan: LogicalPlan) -> usize {
        DataFrame::new(ctx.state(), plan)
            .collect()
            .await
            .unwrap()
            .iter()
            .map(|batch| batch.num_rows())
            .sum()
    }

    async fn collect(ctx: &SessionContext, plan: LogicalPlan) -> Vec<RecordBatch> {
        DataFrame::new(ctx.state(), plan).collect().await.unwrap()
    }

    async fn census_ctx_and_plan(sql: &str) -> (SessionContext, LogicalPlan) {
        let ctx = SessionContext::new();
        ctx.register_batch("a", census_a_batch()).unwrap();
        ctx.register_batch("b", census_b_batch()).unwrap();
        let plan = ctx.sql(sql).await.unwrap().logical_plan().clone();
        (ctx, plan)
    }

    fn census_a_batch() -> RecordBatch {
        let ids = (0..200).map(i64::from).collect::<Vec<_>>();
        let groups = (0..200)
            .map(|i| match i % 4 {
                0 => "A",
                1 => "B",
                2 => "C",
                _ => "D",
            })
            .collect::<Vec<_>>();
        let cats = (0..200)
            .map(|i| if i % 2 == 0 { "x" } else { "y" })
            .collect::<Vec<_>>();
        let values = (0..200)
            .map(|i| {
                if i % 17 == 0 {
                    None
                } else {
                    Some((i % 31) as f64 + 0.25)
                }
            })
            .collect::<Vec<_>>();
        let timestamps = (0..200)
            .map(|i| 1_700_000_000_000_000_000_i64 + i64::from(i) * 1_000_000_000)
            .collect::<Vec<_>>();

        RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("id", DataType::Int64, false),
                Field::new("grp", DataType::Utf8, false),
                Field::new("cat", DataType::Utf8, false),
                Field::new("v", DataType::Float64, true),
                Field::new("ts", DataType::Timestamp(TimeUnit::Nanosecond, None), false),
            ])),
            vec![
                Arc::new(Int64Array::from(ids)) as _,
                Arc::new(StringArray::from(groups)) as _,
                Arc::new(StringArray::from(cats)) as _,
                Arc::new(Float64Array::from(values)) as _,
                Arc::new(TimestampNanosecondArray::from(timestamps)) as _,
            ],
        )
        .unwrap()
    }

    fn census_b_batch() -> RecordBatch {
        let groups = (0..40)
            .map(|i| match i % 4 {
                0 => "A",
                1 => "B",
                2 => "C",
                _ => "D",
            })
            .collect::<Vec<_>>();
        let bonuses = (0..40).map(|i| (i % 9) as f64 + 0.5).collect::<Vec<_>>();

        RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("grp", DataType::Utf8, false),
                Field::new("bonus", DataType::Float64, false),
            ])),
            vec![
                Arc::new(StringArray::from(groups)) as _,
                Arc::new(Float64Array::from(bonuses)) as _,
            ],
        )
        .unwrap()
    }

    async fn assert_bake_equivalence(
        sql: &str,
        policy: PartialEvalPolicy,
        param_sets: &[Vec<(&str, ScalarValue)>],
    ) -> BakeReport {
        let (ctx, plan) = census_ctx_and_plan(sql).await;
        let output = partial_evaluate_impl(plan.clone(), &ctx, &policy)
            .await
            .unwrap();

        for param_set in param_sets {
            let original = plan
                .clone()
                .with_param_values(combined_params(&policy, param_set))
                .unwrap();
            let residual = output
                .residual
                .clone()
                .with_param_values(param_values(param_set))
                .unwrap();
            assert_eq!(
                original.schema().as_arrow(),
                residual.schema().as_arrow(),
                "{sql}"
            );
            assert_eq!(
                original.schema().field_names(),
                residual.schema().field_names(),
                "{sql}"
            );

            let expected = collect(&ctx, original).await;
            let actual = collect(&ctx, residual).await;
            assert_eq!(sorted_rows(&expected), sorted_rows(&actual), "{sql}");
        }

        output.report
    }

    fn combined_params(
        policy: &PartialEvalPolicy,
        param_set: &[(&str, ScalarValue)],
    ) -> Vec<(String, ScalarValue)> {
        let mut values = BTreeMap::new();
        for (name, value) in &policy.fixed_params {
            values.insert(name.trim_start_matches('$').to_string(), value.clone());
        }
        for (name, value) in param_set {
            values.insert(name.trim_start_matches('$').to_string(), value.clone());
        }
        values.into_iter().collect()
    }

    fn param_values(param_set: &[(&str, ScalarValue)]) -> Vec<(String, ScalarValue)> {
        param_set
            .iter()
            .map(|(name, value)| (name.trim_start_matches('$').to_string(), value.clone()))
            .collect()
    }

    fn sorted_rows(batches: &[RecordBatch]) -> Vec<String> {
        let mut rows = Vec::new();
        for batch in batches {
            for row in 0..batch.num_rows() {
                let values = batch
                    .columns()
                    .iter()
                    .map(|array| array_value_to_string(array.as_ref(), row).unwrap())
                    .collect::<Vec<_>>();
                rows.push(values.join("\u{1f}"));
            }
        }
        rows.sort();
        rows
    }

    #[tokio::test]
    async fn aggregate_below_held_param_filter_folds() {
        let (ctx, plan) = ctx_and_plan(
            "SELECT * FROM (SELECT region, SUM(x) AS total FROM t GROUP BY region) q WHERE total < $max",
        )
        .await;

        let output = partial_evaluate_impl(plan, &ctx, &PartialEvalPolicy::default())
            .await
            .unwrap();
        let display = output.residual.display_indent().to_string();

        assert_eq!(output.report.baked.len(), 1);
        assert!(display.contains("Filter"), "{display}");
        assert!(display.contains("__pe_baked_0"), "{display}");
        assert!(!display.contains("__pe_hold"), "{display}");
        assert_eq!(output.report.remaining_params, vec!["$max".to_string()]);

        let bound = output
            .residual
            .with_param_values(vec![("max", ScalarValue::Float64(Some(10.0)))])
            .unwrap();
        assert_eq!(rows(&ctx, bound).await, 3);
    }

    #[tokio::test]
    async fn mixed_conjunction_bakes_after_free_filter_pushdown() {
        let (ctx, plan) = ctx_and_plan(
            "SELECT * FROM (SELECT region, SUM(x) AS total FROM t GROUP BY region) q WHERE total < $max AND region = 'EU'",
        )
        .await;

        let output = partial_evaluate_impl(plan, &ctx, &PartialEvalPolicy::default())
            .await
            .unwrap();

        assert_eq!(output.report.baked.len(), 1);
        assert_eq!(output.report.baked[0].rows, 1);
    }

    #[tokio::test]
    async fn fully_param_free_plan_root_bakes() {
        let (ctx, plan) =
            ctx_and_plan("SELECT region, SUM(x) AS total FROM t GROUP BY region").await;

        let output = partial_evaluate_impl(plan, &ctx, &PartialEvalPolicy::default())
            .await
            .unwrap();
        let display = output.residual.display_indent().to_string();

        assert_eq!(output.report.baked.len(), 1);
        assert!(display.contains("__pe_baked_0"), "{display}");
        assert!(!display.contains("Aggregate"), "{display}");
        assert_eq!(rows(&ctx, output.residual).await, 3);
    }

    #[tokio::test]
    async fn volatile_projection_stays_symbolic_but_input_folds() {
        let (ctx, plan) = ctx_and_plan("SELECT random() AS r FROM t").await;

        let output = partial_evaluate_impl(plan, &ctx, &PartialEvalPolicy::default())
            .await
            .unwrap();
        let display = output.residual.display_indent().to_string();

        assert!(display.contains("random()"), "{display}");
        assert_eq!(output.report.baked.len(), 1);
        assert!(
            output
                .report
                .skipped
                .iter()
                .any(|skip| matches!(skip.reason, SkipReason::Volatile))
        );
        assert_eq!(rows(&ctx, output.residual).await, 4);
    }

    #[tokio::test]
    async fn scalar_subquery_param_free_folds_with_enclosing_filter() {
        let (ctx, plan) = ctx_and_plan("SELECT * FROM t WHERE x < (SELECT AVG(x) FROM t)").await;

        let output = partial_evaluate_impl(plan, &ctx, &PartialEvalPolicy::default())
            .await
            .unwrap();

        assert_eq!(output.report.baked.len(), 1);
        assert_eq!(output.report.baked[0].rows, 2);
        assert_eq!(rows(&ctx, output.residual).await, 2);
    }

    #[tokio::test]
    async fn scalar_subquery_with_placeholder_leaves_enclosing_filter_symbolic() {
        let (ctx, plan) = ctx_and_plan("SELECT * FROM t WHERE x < (SELECT $limit)").await;

        let output = partial_evaluate_impl(plan, &ctx, &PartialEvalPolicy::default())
            .await
            .unwrap();
        let display = output.residual.display_indent().to_string();

        assert!(display.contains("$limit"), "{display}");
        assert!(display.contains("__pe_baked_0"), "{display}");
        assert_eq!(output.report.remaining_params, vec!["$limit".to_string()]);
    }

    #[tokio::test]
    async fn join_with_one_param_side_keeps_param_filter_symbolic() {
        let (ctx, plan) = ctx_and_plan(
            "SELECT * FROM (SELECT * FROM t WHERE region = 'EU') a \
             JOIN (SELECT * FROM u WHERE x < $max) b ON a.region = b.region",
        )
        .await;

        let output = partial_evaluate_impl(plan, &ctx, &PartialEvalPolicy::default())
            .await
            .unwrap();
        let display = output.residual.display_indent().to_string();

        assert!(display.contains("$max"), "{display}");
        assert!(display.contains("__pe_baked_"), "{display}");
        assert_eq!(output.report.remaining_params, vec!["$max".to_string()]);

        let bound = output
            .residual
            .with_param_values(vec![("max", ScalarValue::Float64(Some(10.0)))])
            .unwrap();
        assert_eq!(rows(&ctx, bound).await, 4);
    }

    #[tokio::test]
    async fn empty_result_bakes_and_executes() {
        let (ctx, plan) = ctx_and_plan("SELECT * FROM t WHERE region = 'ZZ'").await;

        let output = partial_evaluate_impl(plan, &ctx, &PartialEvalPolicy::default())
            .await
            .unwrap();

        assert_eq!(output.report.baked.len(), 1);
        assert_eq!(output.report.baked[0].rows, 0);
        assert_eq!(rows(&ctx, output.residual).await, 0);
    }

    #[tokio::test]
    async fn folded_join_replacement_preserves_schema() {
        let (ctx, plan) =
            ctx_and_plan("SELECT a.x, b.x FROM t a JOIN t b ON a.region = b.region").await;
        let original_schema = plan.schema().clone();

        let output = partial_evaluate_impl(plan, &ctx, &PartialEvalPolicy::default())
            .await
            .unwrap();

        assert_eq!(output.residual.schema(), &original_schema);
        assert_eq!(output.report.baked[0].schema.fields()[0].name(), "x");
        assert_eq!(output.report.baked[0].schema.fields()[1].name(), "x_1");
        assert_eq!(rows(&ctx, output.residual).await, 6);
    }

    #[tokio::test]
    async fn partial_evaluate_set_deduplicates_identical_root_bakes() {
        let (ctx, plan_a) =
            ctx_and_plan("SELECT region, SUM(x) AS total FROM t GROUP BY region").await;
        let plan_b = ctx
            .sql("SELECT region, SUM(x) AS total FROM t GROUP BY region")
            .await
            .unwrap()
            .logical_plan()
            .clone();

        let (residuals, report) =
            partial_evaluate_set_impl(vec![plan_a, plan_b], &ctx, &PartialEvalPolicy::default())
                .await
                .unwrap();

        assert_eq!(residuals.len(), 2);
        assert_eq!(report.baked.len(), 1);
        assert_eq!(report.baked[0].occurrences, 2);
    }

    #[tokio::test]
    async fn total_budget_exhaustion_stops_later_plan_in_set() {
        let (ctx, first_plan) = ctx_and_plan("SELECT * FROM t WHERE region = 'EU'").await;
        let first_output =
            partial_evaluate_impl(first_plan.clone(), &ctx, &PartialEvalPolicy::default())
                .await
                .unwrap();
        let second_plan = ctx
            .sql("SELECT * FROM t WHERE region = 'NA'")
            .await
            .unwrap()
            .logical_plan()
            .clone();
        let policy = PartialEvalPolicy {
            max_baked_bytes_total: first_output.report.baked[0].bytes,
            ..PartialEvalPolicy::default()
        };

        let (_residuals, report) =
            partial_evaluate_set_impl(vec![first_plan, second_plan], &ctx, &policy)
                .await
                .unwrap();

        assert_eq!(report.baked.len(), 1);
        assert!(
            report
                .skipped
                .iter()
                .any(|skip| matches!(skip.reason, SkipReason::TotalBudgetExhausted))
        );
    }

    #[tokio::test]
    async fn limit_inside_foldable_subtree_bakes_limited_result() {
        let (ctx, plan) = ctx_and_plan("SELECT * FROM t LIMIT 2").await;

        let output = partial_evaluate_impl(plan, &ctx, &PartialEvalPolicy::default())
            .await
            .unwrap();

        assert_eq!(output.report.baked.len(), 1);
        assert_eq!(output.report.baked[0].rows, 2);
        assert_eq!(rows(&ctx, output.residual).await, 2);
    }

    #[tokio::test]
    async fn tiny_per_subtree_budget_records_skip() {
        let (ctx, plan) = ctx_and_plan("SELECT * FROM t").await;
        let policy = PartialEvalPolicy {
            max_baked_bytes_per_subtree: 1,
            ..PartialEvalPolicy::default()
        };

        let output = partial_evaluate_impl(plan, &ctx, &policy).await.unwrap();

        assert!(output.report.baked.is_empty());
        assert!(
            output
                .report
                .skipped
                .iter()
                .any(|skip| matches!(skip.reason, SkipReason::OverBudget { .. }))
        );
        assert_eq!(rows(&ctx, output.residual).await, 4);
    }

    #[tokio::test]
    async fn per_subtree_budget_abort_descends_to_smaller_child() {
        let (ctx, small_plan) = ctx_and_plan("SELECT * FROM t").await;
        let small_output = partial_evaluate_impl(small_plan, &ctx, &PartialEvalPolicy::default())
            .await
            .unwrap();
        let big_plan = ctx
            .sql(
                "SELECT x, region, x + 1 AS x1, x + 2 AS x2, x + 3 AS x3, \
                 x + 4 AS x4, x + 5 AS x5, x + 6 AS x6, x + 7 AS x7, x + 8 AS x8 FROM t",
            )
            .await
            .unwrap()
            .logical_plan()
            .clone();
        let policy = PartialEvalPolicy {
            max_baked_bytes_per_subtree: small_output.report.baked[0].bytes,
            ..PartialEvalPolicy::default()
        };

        let output = partial_evaluate_impl(big_plan, &ctx, &policy)
            .await
            .unwrap();

        assert!(
            output
                .report
                .skipped
                .iter()
                .any(|skip| matches!(skip.reason, SkipReason::OverBudget { .. }))
        );
        assert_eq!(output.report.baked.len(), 1);
        assert_eq!(rows(&ctx, output.residual).await, 4);
    }

    #[tokio::test]
    async fn temporal_predicate_stays_symbolic_and_is_reported() {
        let (ctx, plan) = ctx_and_plan(
            "SELECT * FROM t WHERE now() > timestamp '2000-01-01 00:00:00' AND region = 'EU'",
        )
        .await;

        let output = partial_evaluate_impl(plan, &ctx, &PartialEvalPolicy::default())
            .await
            .unwrap();
        let display = output.residual.display_indent().to_string();

        assert!(display.contains("now()"), "{display}");
        assert!(!display.contains("__pe_hold"), "{display}");
        assert!(
            output
                .report
                .skipped
                .iter()
                .any(|skip| matches!(skip.reason, SkipReason::TemporalFunction))
        );
    }

    #[tokio::test]
    async fn fixed_param_inventory_distinguishes_remaining_and_unused() {
        let (ctx, plan) = ctx_and_plan("SELECT * FROM t WHERE x > $min AND region = $region").await;
        let policy = PartialEvalPolicy {
            fixed_params: vec![
                ("min".to_string(), ScalarValue::Float64(Some(1.5))),
                (
                    "unused".to_string(),
                    ScalarValue::Utf8(Some("x".to_string())),
                ),
            ],
            ..PartialEvalPolicy::default()
        };

        let output = partial_evaluate_impl(plan, &ctx, &policy).await.unwrap();

        assert_eq!(
            output.report.fixed_params_applied,
            vec![("min".to_string(), ScalarValue::Float64(Some(1.5)))]
        );
        assert_eq!(
            output.report.unused_fixed_params,
            vec!["unused".to_string()]
        );
        assert_eq!(output.report.remaining_params, vec!["$region".to_string()]);
    }

    #[tokio::test]
    async fn excluded_table_stays_symbolic_while_other_input_folds() {
        let (ctx, plan) = ctx_and_plan("SELECT * FROM t JOIN u ON t.region = u.region").await;
        let policy = PartialEvalPolicy {
            unfoldable_tables: HashSet::from(["u".to_string()]),
            ..PartialEvalPolicy::default()
        };

        let output = partial_evaluate_impl(plan, &ctx, &policy).await.unwrap();
        let display = output.residual.display_indent().to_string();

        assert!(display.contains("__pe_baked_0"), "{display}");
        assert!(display.contains("TableScan: u"), "{display}");
        assert!(
            output
                .report
                .skipped
                .iter()
                .any(|skip| matches!(skip.reason, SkipReason::ExcludedTable))
        );
        assert_eq!(rows(&ctx, output.residual).await, 6);
    }

    #[tokio::test]
    async fn bake_equivalence_census() {
        let float_sets = [
            vec![("min_v", ScalarValue::Float64(Some(10.0)))],
            vec![("min_v", ScalarValue::Float64(Some(24.0)))],
        ];
        let total_sets = [
            vec![("min_total", ScalarValue::Float64(Some(500.0)))],
            vec![("min_total", ScalarValue::Float64(Some(650.0)))],
        ];

        assert_bake_equivalence(
            "SELECT id, grp, v FROM a WHERE v > $min_v",
            PartialEvalPolicy::default(),
            &float_sets,
        )
        .await;

        assert_bake_equivalence(
            "SELECT * FROM (SELECT grp, SUM(v) AS total FROM a GROUP BY grp) q WHERE total > $min_total",
            PartialEvalPolicy::default(),
            &total_sets,
        )
        .await;

        assert_bake_equivalence(
            "SELECT id, grp, v FROM a WHERE v > $min_v AND grp = 'A'",
            PartialEvalPolicy::default(),
            &float_sets,
        )
        .await;

        assert_bake_equivalence(
            "SELECT * FROM (SELECT a.grp, SUM(a.v + b.bonus) AS total \
             FROM a JOIN b ON a.grp = b.grp GROUP BY a.grp) q WHERE total > $min_total",
            PartialEvalPolicy::default(),
            &total_sets,
        )
        .await;

        assert_bake_equivalence(
            "SELECT * FROM (SELECT grp, total, RANK() OVER (ORDER BY total DESC) AS rnk \
             FROM (SELECT grp, SUM(v) AS total FROM a GROUP BY grp) s) q WHERE total > $min_total",
            PartialEvalPolicy::default(),
            &total_sets,
        )
        .await;

        assert_bake_equivalence(
            "SELECT id, grp, v FROM a WHERE v > (SELECT AVG(v) FROM a) AND id < $max_id",
            PartialEvalPolicy::default(),
            &[
                vec![("max_id", ScalarValue::Int64(Some(80)))],
                vec![("max_id", ScalarValue::Int64(Some(150)))],
            ],
        )
        .await;

        let fixed_policy = PartialEvalPolicy {
            fixed_params: vec![("grp".to_string(), ScalarValue::Utf8(Some("A".to_string())))],
            ..PartialEvalPolicy::default()
        };
        let fixed_report = assert_bake_equivalence(
            "SELECT * FROM (SELECT grp, SUM(v) AS total FROM a WHERE grp = $grp GROUP BY grp) q \
             WHERE total > $min_total",
            fixed_policy,
            &total_sets,
        )
        .await;
        let unfixed_report = assert_bake_equivalence(
            "SELECT * FROM (SELECT grp, SUM(v) AS total FROM a WHERE grp = $grp GROUP BY grp) q \
             WHERE total > $min_total",
            PartialEvalPolicy::default(),
            &[
                vec![
                    ("grp", ScalarValue::Utf8(Some("A".to_string()))),
                    ("min_total", ScalarValue::Float64(Some(500.0))),
                ],
                vec![
                    ("grp", ScalarValue::Utf8(Some("A".to_string()))),
                    ("min_total", ScalarValue::Float64(Some(650.0))),
                ],
            ],
        )
        .await;
        assert_eq!(
            fixed_report.fixed_params_applied,
            vec![("grp".to_string(), ScalarValue::Utf8(Some("A".to_string())))]
        );
        assert_eq!(
            fixed_report.remaining_params,
            vec!["$min_total".to_string()]
        );
        assert!(fixed_report.baked[0].rows < unfixed_report.baked[0].rows);

        assert_bake_equivalence(
            "SELECT id, grp, v FROM a WHERE grp = 'A' \
             UNION ALL SELECT id, grp, v FROM a WHERE v > $min_v",
            PartialEvalPolicy::default(),
            &float_sets,
        )
        .await;

        assert_bake_equivalence(
            "SELECT DISTINCT grp FROM a ORDER BY grp LIMIT 3",
            PartialEvalPolicy::default(),
            &[vec![], vec![]],
        )
        .await;

        assert_bake_equivalence(
            "SELECT grp, SUM(v) AS total FROM a GROUP BY grp HAVING SUM(v) > $min_total",
            PartialEvalPolicy::default(),
            &total_sets,
        )
        .await;

        assert_bake_equivalence(
            "SELECT x.id, x.grp, x.v, y.bonus FROM \
             (SELECT id, grp, v FROM a WHERE v > $min_v) x \
             JOIN (SELECT grp, bonus FROM b WHERE bonus < $max_bonus) y ON x.grp = y.grp",
            PartialEvalPolicy::default(),
            &[
                vec![
                    ("min_v", ScalarValue::Float64(Some(8.0))),
                    ("max_bonus", ScalarValue::Float64(Some(5.0))),
                ],
                vec![
                    ("min_v", ScalarValue::Float64(Some(20.0))),
                    ("max_bonus", ScalarValue::Float64(Some(3.0))),
                ],
            ],
        )
        .await;

        assert_bake_equivalence(
            "SELECT id, grp, COALESCE(v, 0.0) AS vv FROM a WHERE COALESCE(v, 0.0) >= $min_v",
            PartialEvalPolicy::default(),
            &float_sets,
        )
        .await;
    }

    #[tokio::test]
    async fn temporal_census_keeps_now_symbolic() {
        let (ctx, plan) = census_ctx_and_plan(
            "SELECT id, grp, v FROM a \
             WHERE now() > timestamp '2000-01-01 00:00:00' AND grp = 'A'",
        )
        .await;

        let output = partial_evaluate_impl(plan, &ctx, &PartialEvalPolicy::default())
            .await
            .unwrap();
        let display = output.residual.display_indent().to_string();

        assert!(display.contains("now()"), "{display}");
        assert!(
            output
                .report
                .skipped
                .iter()
                .any(|skip| matches!(skip.reason, SkipReason::TemporalFunction))
        );
        assert!(!output.report.baked.is_empty());
        assert_eq!(rows(&ctx, output.residual).await, 50);
    }

    #[tokio::test]
    #[ignore]
    async fn print_bake_reports() {
        for sql in [
            "SELECT id, grp, v FROM a WHERE v > $min_v",
            "SELECT * FROM (SELECT grp, SUM(v) AS total FROM a GROUP BY grp) q WHERE total > $min_total",
            "SELECT id, grp, COALESCE(v, 0.0) AS vv FROM a WHERE COALESCE(v, 0.0) >= $min_v",
        ] {
            let (ctx, plan) = census_ctx_and_plan(sql).await;
            let output = partial_evaluate_impl(plan, &ctx, &PartialEvalPolicy::default())
                .await
                .unwrap();
            println!("{sql}\n{}", output.residual.display_indent());
            println!(
                "baked: {}, skipped: {}",
                output.report.baked.len(),
                output.report.skipped.len()
            );
            for baked in &output.report.baked {
                println!(
                    "  {} rows={} bytes={} occurrences={}",
                    baked.table_name, baked.rows, baked.bytes, baked.occurrences
                );
            }
            for skipped in &output.report.skipped {
                println!("  skipped {:?}", skipped.reason);
            }
        }
    }
}
