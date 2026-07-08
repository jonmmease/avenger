use crate::CompiledSqlTransform;
use avenger_chart_core::{
    AvengerChartError, CompiledDataTransform, DataTransformExecutionContext, DataTransformStage,
    ExecutionShape, TimeContext, collect_derived_scalar_ids,
};
use datafusion::{
    arrow::datatypes::SchemaRef,
    dataframe::DataFrame,
    logical_expr::{LogicalPlan, LogicalPlanBuilder, builder::LogicalTableSource},
    prelude::SessionContext,
    sql::unparser::plan_to_sql,
};
use datafusion_common::{
    TableReference,
    tree_node::{TreeNode, TreeNodeRecursion},
    utils::quote_identifier,
};
use indexmap::IndexMap;
use serde_json::Value;
use std::sync::Arc;

pub struct ExpandedStage {
    /// The generated query, addressed to the reserved relation `input`.
    pub sql: String,
    /// Ready-to-use compiled stage equivalent.
    pub compiled: CompiledSqlTransform,
}

/// Stages whose plans are known not to survive the unparser faithfully.
/// Add the typetag name plus a comment linking the failure; never remove
/// silently. Consulted before unparsing.
pub const NATIVE_ONLY_STAGES: &[&str] = &[
    // DataFusion 54 unparses JoinAggregate's self-join plan with duplicate
    // `input.category` qualifications that fail SQL planning; covered by
    // `expansion_census::native_only_stages_do_not_expand`.
    "join_aggregate",
    // DataFusion 54 unparses Lump's self-join/window plan with duplicate
    // `input.category` qualifications that fail SQL planning; covered by
    // `expansion_census::native_only_stages_do_not_expand`.
    "lump",
];

pub async fn expand_stage(
    stage: &DataTransformStage,
    input_schema: SchemaRef,
    ctx: &SessionContext,
) -> Result<Option<ExpandedStage>, AvengerChartError> {
    if stage.transform.execution_shape() != ExecutionShape::PlanRewrite {
        return Ok(None);
    }

    let Some(tag) = transform_tag(stage)? else {
        return Ok(None);
    };
    if NATIVE_ONLY_STAGES.contains(&tag.as_str()) {
        return Ok(None);
    }

    let mut references_derived_scalars = false;
    stage.transform.map_exprs(&mut |expr| {
        if !collect_derived_scalar_ids(&expr)?.is_empty() {
            references_derived_scalars = true;
        }
        Ok(expr)
    })?;
    if references_derived_scalars {
        return Ok(None);
    }

    let source = Arc::new(LogicalTableSource::new(input_schema));
    let plan = LogicalPlanBuilder::scan("input", source, None)?.build()?;
    let dataframe = DataFrame::new(ctx.state(), plan);
    let params = IndexMap::new();
    let synthetic_ctx = DataTransformExecutionContext {
        session_context: ctx,
        params: &params,
        time_context: TimeContext::default(),
        facet_context: None,
    };

    let result = stage.transform.apply(dataframe, &synthetic_ctx).await?;
    if !result.derived_scalars.is_empty() {
        return Ok(None);
    }

    debug_assert!(
        input_scan_count(result.dataframe.logical_plan(), ctx) > 0,
        "expanded plan should retain an input scan"
    );
    let output_names = result
        .dataframe
        .schema()
        .fields()
        .iter()
        .map(|field| field.name().to_string())
        .collect::<Vec<_>>();
    let statement = plan_to_sql(result.dataframe.logical_plan())?;
    let sql = project_sql_in_schema_order(statement.to_string(), &output_names);

    Ok(Some(ExpandedStage {
        compiled: CompiledSqlTransform { query: sql.clone() },
        sql,
    }))
}

fn project_sql_in_schema_order(sql: String, output_names: &[String]) -> String {
    if output_names.is_empty() {
        return sql;
    }
    let projection = output_names
        .iter()
        .map(|name| quote_identifier(name).into_owned())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "SELECT {projection} FROM ({sql}) AS {}",
        quote_identifier("__avenger_expanded")
    )
}

fn transform_tag(stage: &DataTransformStage) -> Result<Option<String>, AvengerChartError> {
    let value = serde_json::to_value(&stage.transform)
        .map_err(|err| AvengerChartError::SerializationError(err.to_string()))?;
    Ok(match value {
        Value::Object(map) => map
            .get("type")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        _ => None,
    })
}

fn input_scan_count(plan: &LogicalPlan, ctx: &SessionContext) -> usize {
    let state = ctx.state();
    let catalog = &state.config_options().catalog;
    let input_ref =
        TableReference::bare("input").resolve(&catalog.default_catalog, &catalog.default_schema);
    let mut count = 0usize;
    let _ = plan.apply(|node| {
        if let LogicalPlan::TableScan(scan) = node {
            let resolved = scan
                .table_name
                .clone()
                .resolve(&catalog.default_catalog, &catalog.default_schema);
            if resolved == input_ref {
                count += 1;
            }
        }
        Ok(TreeNodeRecursion::Continue)
    });
    count
}

#[doc(hidden)]
pub mod test_support {
    use super::*;
    use avenger_chart_core::params_to_datafusion;
    use datafusion::arrow::{record_batch::RecordBatch, util::pretty::pretty_format_batches};
    use datafusion_common::ScalarValue;

    pub async fn assert_stage_roundtrip(
        stage: &DataTransformStage,
        sample: RecordBatch,
        ctx: &SessionContext,
    ) {
        assert_stage_roundtrip_with_params(stage, sample, ctx, &IndexMap::new()).await;
    }

    pub async fn assert_stage_roundtrip_with_params(
        stage: &DataTransformStage,
        sample: RecordBatch,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) {
        let input_schema = sample.schema();
        let execution_ctx = DataTransformExecutionContext {
            session_context: ctx,
            params,
            time_context: TimeContext::default(),
            facet_context: None,
        };

        let original_result = stage
            .transform
            .apply(
                ctx.read_batch(sample.clone()).expect("sample dataframe"),
                &execution_ctx,
            )
            .await
            .expect("apply original stage");
        let original_plan = original_result.dataframe.logical_plan().clone();
        let original = collect_with_params(original_result.dataframe, params)
            .await
            .expect("collect original stage");

        let expanded = expand_stage(stage, input_schema, ctx)
            .await
            .expect("expand stage")
            .expect("stage should expand");
        let expanded_result = expanded
            .compiled
            .apply(
                ctx.read_batch(sample).expect("sample dataframe"),
                &execution_ctx,
            )
            .await
            .expect("apply expanded stage");
        let expanded_plan = expanded_result.dataframe.logical_plan().clone();
        let expanded_batches = collect_with_params(expanded_result.dataframe, params)
            .await
            .expect("collect expanded stage");

        if let Err(message) = assert_sorted_batches_equal(&original, &expanded_batches) {
            panic!(
                "stage roundtrip failed\ntransform: {}\ngenerated SQL:\n{}\n\noriginal plan:\n{}\n\nexpanded plan:\n{}\n\n{}",
                transform_tag(stage)
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| "<unknown>".to_string()),
                expanded.sql,
                original_plan.display_indent(),
                expanded_plan.display_indent(),
                message
            );
        }
    }

    async fn collect_with_params(
        dataframe: DataFrame,
        params: &IndexMap<String, ScalarValue>,
    ) -> datafusion_common::Result<Vec<RecordBatch>> {
        let dataframe = if let Some(param_values) = params_to_datafusion(params) {
            dataframe.with_param_values(param_values)?
        } else {
            dataframe
        };
        dataframe.collect().await
    }

    fn assert_sorted_batches_equal(
        left: &[RecordBatch],
        right: &[RecordBatch],
    ) -> Result<(), String> {
        let left_schema = left.first().map(|batch| batch.schema());
        let right_schema = right.first().map(|batch| batch.schema());
        if left_schema != right_schema {
            return Err(format!(
                "schemas differ\noriginal: {:?}\nexpanded: {:?}",
                left_schema, right_schema
            ));
        }

        let mut left_lines = pretty_lines(left)?;
        let mut right_lines = pretty_lines(right)?;
        left_lines.sort();
        right_lines.sort();

        if left_lines != right_lines {
            return Err(format!(
                "batches differ\noriginal:\n{}\n\nexpanded:\n{}",
                pretty_format_batches(left).map_err(|err| err.to_string())?,
                pretty_format_batches(right).map_err(|err| err.to_string())?
            ));
        }
        Ok(())
    }

    fn pretty_lines(batches: &[RecordBatch]) -> Result<Vec<String>, String> {
        Ok(pretty_format_batches(batches)
            .map_err(|err| err.to_string())?
            .to_string()
            .lines()
            .map(ToString::to_string)
            .collect())
    }
}
