mod aggregate;
mod bin;
mod common;
mod stack;

pub use aggregate::{
    Aggregate, AggregateGroupKeySpec, AggregateMeasureSpec, AggregateOp, AggregateOutput,
    CompiledAggregateTransform,
};
pub use bin::{Bin, BinExtentSpec, BinOutput, CompiledBinTransform};
pub use stack::{CompiledStackTransform, Stack, StackOffset, StackOutput, TransformSortSpec};

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::{
        array::{Array, Float64Array, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    };
    use avenger_chart_core::{
        ChannelValue, DataTransform, DataTransformCompileContext, DataTransformExecutionContext,
        DataTransformStage, Sharing, collect_derived_scalar_ids,
    };
    use datafusion::dataframe::DataFrame;
    use datafusion::logical_expr::col;
    use datafusion::prelude::SessionContext;
    use std::sync::Arc;

    fn sample_dataframe(ctx: &SessionContext) -> DataFrame {
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("category", DataType::Utf8, false),
                Field::new("series", DataType::Utf8, false),
                Field::new("value", DataType::Float64, false),
            ])),
            vec![
                Arc::new(StringArray::from(vec!["A", "A", "A", "B"])) as _,
                Arc::new(StringArray::from(vec!["s1", "s2", "s3", "s1"])) as _,
                Arc::new(Float64Array::from(vec![1.0, 2.0, -3.0, 4.0])) as _,
            ],
        )
        .unwrap();
        ctx.read_batch(batch).unwrap()
    }

    fn bin_dataframe(ctx: &SessionContext) -> DataFrame {
        bin_dataframe_from_values(
            ctx,
            vec![Some(1.0), Some(2.0), Some(3.0), Some(4.0), Some(5.0), None],
        )
    }

    fn bin_dataframe_from_values(ctx: &SessionContext, values: Vec<Option<f64>>) -> DataFrame {
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new(
                "value",
                DataType::Float64,
                true,
            )])),
            vec![Arc::new(Float64Array::from(values)) as _],
        )
        .unwrap();
        ctx.read_batch(batch).unwrap()
    }

    async fn transformed_batches(
        ctx: &SessionContext,
        dataframe: DataFrame,
        transforms: Vec<DataTransformStage>,
    ) -> Vec<RecordBatch> {
        avenger_chart_core::apply_compiled_data_transforms(
            dataframe,
            &transforms,
            &DataTransformExecutionContext {
                session_context: ctx,
            },
        )
        .await
        .unwrap()
        .dataframe
        .collect()
        .await
        .unwrap()
    }

    fn compile_transform<T: DataTransform>(transform: T) -> (DataTransformStage, T::Output) {
        compile_transform_with_scope(Sharing::Free, transform)
    }

    fn compile_transform_with_scope<T: DataTransform>(
        scope: Sharing,
        transform: T,
    ) -> (DataTransformStage, T::Output) {
        let (compiled, output) = transform
            .into_compiled_and_output(DataTransformCompileContext::new(scope))
            .unwrap();
        (DataTransformStage::new(scope, compiled), output)
    }

    fn bin_rows(batch: &RecordBatch) -> Vec<(Option<f64>, Option<f64>, Option<f64>, Option<i64>)> {
        let value = batch
            .column_by_name("value")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        let start = batch
            .column_by_name("value_bin_start")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        let end = batch
            .column_by_name("value_bin_end")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        let index = batch
            .column_by_name("value_bin_index")
            .unwrap()
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        (0..batch.num_rows())
            .map(|row| {
                (
                    (!value.is_null(row)).then(|| value.value(row)),
                    (!start.is_null(row)).then(|| start.value(row)),
                    (!end.is_null(row)).then(|| end.value(row)),
                    (!index.is_null(row)).then(|| index.value(row)),
                )
            })
            .collect()
    }

    fn bin_rows_from_batches(
        batches: &[RecordBatch],
    ) -> Vec<(Option<f64>, Option<f64>, Option<f64>, Option<i64>)> {
        batches.iter().flat_map(bin_rows).collect()
    }

    fn stack_rows(batch: &RecordBatch) -> Vec<(String, String, f64, f64)> {
        let category = batch
            .column_by_name("category")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let series = batch
            .column_by_name("series")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let start = batch
            .column_by_name("value_stack_start")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        let end = batch
            .column_by_name("value_stack_end")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        (0..batch.num_rows())
            .map(|index| {
                (
                    category.value(index).to_string(),
                    series.value(index).to_string(),
                    start.value(index),
                    end.value(index),
                )
            })
            .collect()
    }

    fn stack_rows_from_batches(batches: &[RecordBatch]) -> Vec<(String, String, f64, f64)> {
        batches.iter().flat_map(stack_rows).collect()
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() <= 1e-9,
            "expected {expected}, got {actual}"
        );
    }

    fn assert_stack_rows(
        mut actual: Vec<(String, String, f64, f64)>,
        expected: &[(&str, &str, f64, f64)],
    ) {
        actual.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(expected) {
            assert_eq!(actual.0, expected.0);
            assert_eq!(actual.1, expected.1);
            assert_close(actual.2, expected.2);
            assert_close(actual.3, expected.3);
        }
    }

    #[tokio::test]
    async fn aggregate_groups_and_sums() {
        let ctx = SessionContext::new();
        let dataframe = sample_dataframe(&ctx);
        let transform = Aggregate::new()
            .group_by([col("category"), col("series")])
            .sum("total_value", col("value"));
        let (compiled_transform, output) = compile_transform(transform);
        assert_eq!(output.output("total_value").to_string(), "total_value");
        let result = avenger_chart_core::apply_compiled_data_transforms(
            dataframe,
            &[compiled_transform],
            &DataTransformExecutionContext {
                session_context: &ctx,
            },
        )
        .await
        .unwrap();
        let batches = result.dataframe.collect().await.unwrap();
        let rows: usize = batches.iter().map(|batch| batch.num_rows()).sum();
        assert_eq!(rows, 4);
    }

    #[tokio::test]
    async fn bin_output_names_and_derived_scalar_refs() {
        let output = Bin::new(col("value"))
            .maxbins(4)
            .name("custom_bin")
            .into_compiled_and_output(DataTransformCompileContext::new(Sharing::Free))
            .unwrap()
            .1;

        assert_eq!(output.index().to_string(), "custom_bin_index");
        let start = output.start();
        assert!(matches!(start, ChannelValue::Scaled { .. }));
        let scale = start.get_scale_config().expect("scale config");
        let mut ids = Vec::new();
        for expr in scale.all_exprs(&SessionContext::new()) {
            ids.extend(collect_derived_scalar_ids(&expr).unwrap());
        }
        let axis = start.get_axis_config().expect("axis config");
        for expr in axis.all_exprs(&SessionContext::new()) {
            ids.extend(collect_derived_scalar_ids(&expr).unwrap());
        }
        assert!(
            ids.contains(&"custom_bin_domain_start".to_string()),
            "{ids:?}"
        );
        assert!(
            ids.contains(&"custom_bin_domain_end".to_string()),
            "{ids:?}"
        );
        assert!(
            ids.contains(&"custom_bin_tick_spacing".to_string()),
            "{ids:?}"
        );
    }

    #[tokio::test]
    async fn bin_output_uses_stage_scope_as_default_scale_sharing() {
        let output = Bin::new(col("value"))
            .maxbins(4)
            .into_compiled_and_output(DataTransformCompileContext::new(Sharing::Level(1)))
            .unwrap()
            .1;

        let start = output.start();
        let end = output.end();
        assert_eq!(start.get_share_mode(), Some(Sharing::Level(1)));
        assert_eq!(start.get_transform_scope(), Some(Sharing::Level(1)));
        assert_eq!(end.get_share_mode(), Some(Sharing::Level(1)));
        assert_eq!(end.get_transform_scope(), Some(Sharing::Level(1)));
    }

    #[tokio::test]
    async fn bin_maxbins_zero_errors() {
        let err = match Bin::new(col("value"))
            .maxbins(0)
            .into_compiled_and_output(DataTransformCompileContext::new(Sharing::Free))
        {
            Ok(_) => panic!("maxbins zero should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("maxbins"), "{err}");
    }

    #[tokio::test]
    async fn bin_exact_count_clamps_max_and_preserves_nulls() {
        let ctx = SessionContext::new();
        let dataframe = bin_dataframe(&ctx);
        let (compiled_transform, output) =
            compile_transform(Bin::new(col("value")).maxbins(4).exact());
        assert_eq!(output.index().to_string(), "value_bin_index");

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let rows = bin_rows_from_batches(&batches);
        assert_eq!(
            rows,
            vec![
                (Some(1.0), Some(1.0), Some(2.0), Some(0)),
                (Some(2.0), Some(2.0), Some(3.0), Some(1)),
                (Some(3.0), Some(3.0), Some(4.0), Some(2)),
                (Some(4.0), Some(4.0), Some(5.0), Some(3)),
                (Some(5.0), Some(4.0), Some(5.0), Some(3)),
                (None, None, None, None),
            ]
        );
    }

    #[tokio::test]
    async fn bin_nice_default_uses_friendly_edges() {
        let ctx = SessionContext::new();
        let dataframe =
            bin_dataframe_from_values(&ctx, vec![Some(0.2), Some(1.4), Some(2.8), Some(9.7), None]);
        let (compiled_transform, _) = compile_transform(Bin::new(col("value")).maxbins(5));

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let rows = bin_rows_from_batches(&batches);
        assert_eq!(
            rows,
            vec![
                (Some(0.2), Some(0.0), Some(2.0), Some(0)),
                (Some(1.4), Some(0.0), Some(2.0), Some(0)),
                (Some(2.8), Some(2.0), Some(4.0), Some(1)),
                (Some(9.7), Some(8.0), Some(10.0), Some(4)),
                (None, None, None, None),
            ]
        );
    }

    #[tokio::test]
    async fn bin_exact_preserves_messy_edges() {
        let ctx = SessionContext::new();
        let dataframe =
            bin_dataframe_from_values(&ctx, vec![Some(0.2), Some(1.4), Some(2.8), Some(9.7), None]);
        let (compiled_transform, _) = compile_transform(Bin::new(col("value")).maxbins(5).exact());

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let rows = bin_rows_from_batches(&batches);
        assert_eq!(
            rows,
            vec![
                (Some(0.2), Some(0.2), Some(2.1), Some(0)),
                (Some(1.4), Some(0.2), Some(2.1), Some(0)),
                (Some(2.8), Some(2.1), Some(4.0), Some(1)),
                (Some(9.7), Some(7.8), Some(9.7), Some(4)),
                (None, None, None, None),
            ]
        );
    }

    #[tokio::test]
    async fn bin_steps_choose_smallest_step_under_maxbins() {
        let ctx = SessionContext::new();
        let dataframe = bin_dataframe_from_values(&ctx, vec![Some(0.2), Some(2.8), Some(9.7)]);
        let (compiled_transform, _) =
            compile_transform(Bin::new(col("value")).maxbins(4).steps([1.0, 2.0, 5.0]));

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let rows = bin_rows_from_batches(&batches);
        assert_eq!(
            rows,
            vec![
                (Some(0.2), Some(0.0), Some(5.0), Some(0)),
                (Some(2.8), Some(0.0), Some(5.0), Some(0)),
                (Some(9.7), Some(5.0), Some(10.0), Some(1)),
            ]
        );
    }

    #[tokio::test]
    async fn bin_minstep_prevents_over_refinement() {
        let ctx = SessionContext::new();
        let dataframe = bin_dataframe_from_values(&ctx, vec![Some(0.2), Some(2.8), Some(9.7)]);
        let (compiled_transform, _) =
            compile_transform(Bin::new(col("value")).maxbins(10).minstep(5.0));

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let rows = bin_rows_from_batches(&batches);
        assert_eq!(
            rows,
            vec![
                (Some(0.2), Some(0.0), Some(5.0), Some(0)),
                (Some(2.8), Some(0.0), Some(5.0), Some(0)),
                (Some(9.7), Some(5.0), Some(10.0), Some(1)),
            ]
        );
    }

    #[tokio::test]
    async fn bin_single_value_uses_fallback_step() {
        let ctx = SessionContext::new();
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new(
                "value",
                DataType::Float64,
                false,
            )])),
            vec![Arc::new(Float64Array::from(vec![7.0, 7.0])) as _],
        )
        .unwrap();
        let dataframe = ctx.read_batch(batch).unwrap();
        let (compiled_transform, _) = compile_transform(Bin::new(col("value")).maxbins(3));

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let rows = bin_rows_from_batches(&batches);
        assert_eq!(
            rows,
            vec![
                (Some(7.0), Some(7.0), Some(8.0), Some(0)),
                (Some(7.0), Some(7.0), Some(8.0), Some(0)),
            ]
        );
    }

    #[tokio::test]
    async fn bin_returns_derived_scalars() {
        let ctx = SessionContext::new();
        let dataframe = bin_dataframe(&ctx);
        let (compiled_transform, _) = compile_transform(Bin::new(col("value")).maxbins(4));
        let result = avenger_chart_core::apply_compiled_data_transforms(
            dataframe,
            &[compiled_transform],
            &DataTransformExecutionContext {
                session_context: &ctx,
            },
        )
        .await
        .unwrap();
        assert!(
            result
                .derived_scalars
                .contains_key("value_bin_domain_start")
        );
        assert!(result.derived_scalars.contains_key("value_bin_domain_end"));
        assert!(
            result
                .derived_scalars
                .contains_key("value_bin_tick_spacing")
        );
    }

    #[tokio::test]
    async fn stack_zero_adds_start_and_end_columns() {
        let ctx = SessionContext::new();
        let dataframe = sample_dataframe(&ctx);
        let transform = Stack::new(col("value"))
            .group_by([col("category")])
            .sort_by_exprs([col("series")])
            .name("value_stack");
        let (compiled_transform, output) = compile_transform(transform);
        assert!(matches!(output.start(), ChannelValue::Scaled { .. }));
        let result = avenger_chart_core::apply_compiled_data_transforms(
            dataframe,
            &[compiled_transform],
            &DataTransformExecutionContext {
                session_context: &ctx,
            },
        )
        .await
        .unwrap();
        let schema = result.dataframe.schema();
        assert!(schema.field_with_name(None, "value_stack_start").is_ok());
        assert!(schema.field_with_name(None, "value_stack_end").is_ok());
    }

    #[tokio::test]
    async fn stack_zero_computes_positive_and_negative_extents() {
        let ctx = SessionContext::new();
        let dataframe = sample_dataframe(&ctx);
        let (compiled_transform, _) = compile_transform(
            Stack::new(col("value"))
                .group_by([col("category")])
                .sort_by_exprs([col("series")])
                .name("value_stack"),
        );
        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        assert_stack_rows(
            stack_rows_from_batches(&batches),
            &[
                ("A", "s1", 2.0, 3.0),
                ("A", "s2", 0.0, 2.0),
                ("A", "s3", 0.0, -3.0),
                ("B", "s1", 0.0, 4.0),
            ],
        );
    }

    #[tokio::test]
    async fn stack_normalize_uses_absolute_group_totals() {
        let ctx = SessionContext::new();
        let dataframe = sample_dataframe(&ctx);
        let (compiled_transform, _) = compile_transform(
            Stack::new(col("value"))
                .group_by([col("category")])
                .sort_by_exprs([col("series")])
                .offset(StackOffset::Normalize)
                .name("value_stack"),
        );
        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        assert_stack_rows(
            stack_rows_from_batches(&batches),
            &[
                ("A", "s1", 5.0 / 6.0, 1.0),
                ("A", "s2", 3.0 / 6.0, 5.0 / 6.0),
                ("A", "s3", 0.0, 3.0 / 6.0),
                ("B", "s1", 0.0, 1.0),
            ],
        );
    }

    #[tokio::test]
    async fn stack_center_offsets_smaller_groups() {
        let ctx = SessionContext::new();
        let dataframe = sample_dataframe(&ctx);
        let (compiled_transform, _) = compile_transform(
            Stack::new(col("value"))
                .group_by([col("category")])
                .sort_by_exprs([col("series")])
                .offset(StackOffset::Center)
                .name("value_stack"),
        );
        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        assert_stack_rows(
            stack_rows_from_batches(&batches),
            &[
                ("A", "s1", 5.0, 6.0),
                ("A", "s2", 3.0, 5.0),
                ("A", "s3", 0.0, 3.0),
                ("B", "s1", 1.0, 5.0),
            ],
        );
    }

    #[tokio::test]
    async fn aggregate_output_feeds_stack_transform() {
        let ctx = SessionContext::new();
        let dataframe = sample_dataframe(&ctx);
        let (aggregate_transform, aggregate) = compile_transform(
            Aggregate::new()
                .group_by([col("category"), col("series")])
                .sum("total_value", col("value")),
        );
        let (stack_transform, _) = compile_transform(
            Stack::new(aggregate.output("total_value"))
                .group_by([col("category")])
                .sort_by_exprs([col("series")])
                .name("value_stack"),
        );
        let batches =
            transformed_batches(&ctx, dataframe, vec![aggregate_transform, stack_transform]).await;
        assert_stack_rows(
            stack_rows_from_batches(&batches),
            &[
                ("A", "s1", 2.0, 3.0),
                ("A", "s2", 0.0, 2.0),
                ("A", "s3", 0.0, -3.0),
                ("B", "s1", 0.0, 4.0),
            ],
        );
    }
}
