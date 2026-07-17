mod aggregate;
mod bin;
mod calculate;
mod common;
pub mod expand;
mod filter;
mod fold;
mod impute;
mod join_aggregate;
mod kde;
pub mod language;
pub mod lump;
mod pipeline;
mod rasterize_2d;
mod scalar_aggregate;
mod select;
pub mod sql;
mod stack;
mod time_fill;
mod time_levels;
mod time_unit;
mod window;

pub use aggregate::{
    Aggregate, AggregateGroupKeySpec, AggregateMeasureSpec, AggregateOp, AggregateOutput,
    CompiledAggregateTransform,
};
pub use bin::{Bin, BinExtentSpec, BinOutput, CompiledBinTransform};
pub use calculate::{Calculate, CalculateExprSpec, CompiledCalculateTransform};
pub use filter::{CompiledFilterTransform, Filter};
pub use fold::{CompiledFoldTransform, Fold, FoldFieldSpec, FoldOutput};
pub use impute::{CompiledImputeTransform, Impute, ImputeMethodSpec, ImputeOutput};
pub use join_aggregate::{CompiledJoinAggregateTransform, JoinAggregate};
pub use kde::{CompiledKdeTransform, Kde, KdeOutput, KdeResolve};
pub use lump::{CompiledLumpTransform, Lump, LumpOtherMode, LumpOutput};
pub use pipeline::{CompiledPipelineTransform, Pipeline, PipelineOutput, PipelineOutputSpec};
pub use rasterize_2d::{
    CompiledRasterize2DTransform, RASTERIZE_2D_MATERIALIZATION_KIND, Rasterize2D, Rasterize2DAgg,
    Rasterize2DDimension, Rasterize2DDimensionSpec, Rasterize2DExecutor, Rasterize2DExtentSpec,
    Rasterize2DMaterializationSpec, Rasterize2DOutput,
};
pub use scalar_aggregate::{
    CompiledScalarAggregateTransform, SCALAR_AGGREGATE_MATERIALIZATION_KIND, ScalarAggregate,
    ScalarAggregateEvaluation, ScalarAggregateExecutor, ScalarAggregateMaterializationSpec,
    ScalarAggregateOutput, scalar_batch_from_literals, scalar_literals_from_batch,
};
pub use select::{CompiledSelectTransform, Select, SelectExprSpec};
pub use sql::{CompiledSqlTransform, Sql, SqlOutput};
pub use stack::{CompiledStackTransform, Stack, StackOffset, StackOutput, TransformSortSpec};
pub use time_fill::{CompiledTimeFillTransform, TimeFill, TimeFillExtentSpec, TimeFillOutput};
pub use time_levels::{
    CompiledTimeLevelsTransform, TimeLevel, TimeLevelConfig, TimeLevelKey, TimeLevelKeys,
    TimeLevelLabel, TimeLevels, TimeLevelsOutput,
};
pub use time_unit::{CompiledTimeUnitTransform, TimeUnit, TimeUnitOutput, TimeUnitPart};
pub use window::{CompiledWindowTransform, Window, WindowExprSpec, WindowSortSpec};

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::{
        array::{
            Array, BooleanArray, Float64Array, Int32Array, Int64Array, ListArray, StringArray,
            StructArray, TimestampMillisecondArray, UInt32Array, UInt64Array,
        },
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    };
    use avenger_chart_core::{
        AvengerChartError, ChannelValue, CoordinationScope, DataTransform,
        DataTransformCompileContext, DataTransformExecutionContext, DataTransformFacetContext,
        DataTransformStage, DefaultLogicalExprNodeExt, MaterializationExecutionContext,
        MaterializationExecutor, MaterializationOutputKind, MaterializationPolicy,
        MaterializationResult, Param, SharingLevel, TimeContext, ViewMaterializationContext,
        WeekStart, collect_derived_scalar_ids, eval_to_scalars,
    };
    use datafusion::common::ScalarValue;
    use datafusion::dataframe::DataFrame;
    use datafusion::functions_aggregate::{expr_fn::sum, sum::sum_udaf};
    use datafusion::functions_window::expr_fn::{
        dense_rank, lag, lead, ntile, percent_rank, rank, row_number,
    };
    use datafusion::logical_expr::{
        Expr, WindowFunctionDefinition, col, expr::WindowFunction, lit,
    };
    use datafusion::prelude::SessionContext;
    use indexmap::IndexMap;
    use std::{collections::HashSet, sync::Arc};

    type BinRow = (Option<f64>, Option<f64>, Option<f64>, Option<i64>);
    type LumpRow = (Option<String>, f64, Option<String>, Option<f64>, bool);

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

    fn stats_dataframe(ctx: &SessionContext) -> DataFrame {
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("category", DataType::Utf8, false),
                Field::new("value", DataType::Float64, false),
            ])),
            vec![
                Arc::new(StringArray::from(vec![
                    "A", "A", "A", "A", "B", "B", "B", "B",
                ])) as _,
                Arc::new(Float64Array::from(vec![
                    1.0, 2.0, 3.0, 4.0, 10.0, 20.0, 30.0, 40.0,
                ])) as _,
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

    fn kde_dataframe(ctx: &SessionContext) -> DataFrame {
        kde_dataframe_from_values(
            ctx,
            vec![
                Some("A"),
                Some("A"),
                Some("A"),
                Some("B"),
                Some("B"),
                Some("B"),
            ],
            vec![
                Some(0.0),
                Some(1.0),
                Some(2.0),
                Some(10.0),
                Some(11.0),
                Some(12.0),
            ],
        )
    }

    fn kde_dataframe_from_values(
        ctx: &SessionContext,
        series: Vec<Option<&str>>,
        values: Vec<Option<f64>>,
    ) -> DataFrame {
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("series", DataType::Utf8, true),
                Field::new("value", DataType::Float64, true),
            ])),
            vec![
                Arc::new(StringArray::from(series)) as _,
                Arc::new(Float64Array::from(values)) as _,
            ],
        )
        .unwrap();
        ctx.read_batch(batch).unwrap()
    }

    fn lump_dataframe(
        ctx: &SessionContext,
        categories: Vec<Option<&str>>,
        values: Vec<f64>,
    ) -> DataFrame {
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("category", DataType::Utf8, true),
                Field::new("value", DataType::Float64, false),
            ])),
            vec![
                Arc::new(StringArray::from(categories)) as _,
                Arc::new(Float64Array::from(values)) as _,
            ],
        )
        .unwrap();
        ctx.read_batch(batch).unwrap()
    }

    fn fold_dataframe(ctx: &SessionContext) -> DataFrame {
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("region", DataType::Utf8, false),
                Field::new("gold", DataType::Float64, false),
                Field::new("silver", DataType::Float64, false),
                Field::new("bronze", DataType::Float64, false),
            ])),
            vec![
                Arc::new(StringArray::from(vec!["North", "South"])) as _,
                Arc::new(Float64Array::from(vec![3.0, 1.0])) as _,
                Arc::new(Float64Array::from(vec![2.0, 4.0])) as _,
                Arc::new(Float64Array::from(vec![5.0, 2.0])) as _,
            ],
        )
        .unwrap();
        ctx.read_batch(batch).unwrap()
    }

    fn time_dataframe(ctx: &SessionContext, values: Vec<Option<i64>>) -> DataFrame {
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new(
                "timestamp",
                DataType::Timestamp(datafusion::arrow::datatypes::TimeUnit::Millisecond, None),
                true,
            )])),
            vec![Arc::new(TimestampMillisecondArray::from(values)) as _],
        )
        .unwrap();
        ctx.read_batch(batch).unwrap()
    }

    fn timestamp_ms(year: i32, month: u32, day: u32) -> i64 {
        chrono::NaiveDate::from_ymd_opt(year, month, day)
            .expect("valid test date")
            .and_hms_opt(0, 0, 0)
            .expect("valid test time")
            .and_utc()
            .timestamp_millis()
    }

    fn temporal_sales_dataframe(ctx: &SessionContext) -> DataFrame {
        let rows = [
            (timestamp_ms(2024, 1, 5), "A", 10.0),
            (timestamp_ms(2024, 1, 20), "A", 5.0),
            (timestamp_ms(2024, 3, 2), "A", 30.0),
            (timestamp_ms(2024, 1, 7), "B", 4.0),
            (timestamp_ms(2024, 3, 7), "B", 6.0),
        ];
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new(
                    "timestamp",
                    DataType::Timestamp(datafusion::arrow::datatypes::TimeUnit::Millisecond, None),
                    false,
                ),
                Field::new("segment", DataType::Utf8, false),
                Field::new("value", DataType::Float64, false),
            ])),
            vec![
                Arc::new(TimestampMillisecondArray::from(
                    rows.iter().map(|row| row.0).collect::<Vec<_>>(),
                )) as _,
                Arc::new(StringArray::from(
                    rows.iter().map(|row| row.1).collect::<Vec<_>>(),
                )) as _,
                Arc::new(Float64Array::from(
                    rows.iter().map(|row| row.2).collect::<Vec<_>>(),
                )) as _,
            ],
        )
        .unwrap();
        ctx.read_batch(batch).unwrap()
    }

    fn window_dataframe(ctx: &SessionContext) -> DataFrame {
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("series", DataType::Utf8, false),
                Field::new("day", DataType::Int64, false),
                Field::new("value", DataType::Float64, false),
            ])),
            vec![
                Arc::new(StringArray::from(vec!["A", "A", "A", "B", "B"])) as _,
                Arc::new(Int64Array::from(vec![1, 2, 3, 1, 2])) as _,
                Arc::new(Float64Array::from(vec![2.0, 4.0, 1.0, 5.0, 3.0])) as _,
            ],
        )
        .unwrap();
        ctx.read_batch(batch).unwrap()
    }

    fn window_peer_dataframe(ctx: &SessionContext) -> DataFrame {
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("series", DataType::Utf8, false),
                Field::new("day", DataType::Int64, false),
                Field::new("value", DataType::Float64, false),
            ])),
            vec![
                Arc::new(StringArray::from(vec!["A", "A", "A", "B", "B"])) as _,
                Arc::new(Int64Array::from(vec![1, 2, 3, 1, 2])) as _,
                Arc::new(Float64Array::from(vec![10.0, 10.0, 5.0, 1.0, 1.0])) as _,
            ],
        )
        .unwrap();
        ctx.read_batch(batch).unwrap()
    }

    fn impute_dataframe(ctx: &SessionContext) -> DataFrame {
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("series", DataType::Utf8, false),
                Field::new("month", DataType::Int64, false),
                Field::new("value", DataType::Float64, true),
            ])),
            vec![
                Arc::new(StringArray::from(vec!["A", "A", "B", "B"])) as _,
                Arc::new(Int64Array::from(vec![1, 3, 1, 2])) as _,
                Arc::new(Float64Array::from(vec![
                    Some(2.0),
                    Some(6.0),
                    Some(1.0),
                    None,
                ])) as _,
            ],
        )
        .unwrap();
        ctx.read_batch(batch).unwrap()
    }

    async fn transformed_batches(
        ctx: &SessionContext,
        dataframe: DataFrame,
        transforms: Vec<DataTransformStage>,
    ) -> Vec<RecordBatch> {
        transformed_batches_with_params(ctx, dataframe, transforms, &IndexMap::new()).await
    }

    async fn transformed_batches_with_params(
        ctx: &SessionContext,
        dataframe: DataFrame,
        transforms: Vec<DataTransformStage>,
        params: &IndexMap<String, ScalarValue>,
    ) -> Vec<RecordBatch> {
        transformed_batches_with_params_and_time_context(
            ctx,
            dataframe,
            transforms,
            params,
            TimeContext::default(),
        )
        .await
    }

    async fn transformed_batches_with_time_context(
        ctx: &SessionContext,
        dataframe: DataFrame,
        transforms: Vec<DataTransformStage>,
        time_context: TimeContext,
    ) -> Vec<RecordBatch> {
        transformed_batches_with_params_and_time_context(
            ctx,
            dataframe,
            transforms,
            &IndexMap::new(),
            time_context,
        )
        .await
    }

    async fn transformed_batches_with_params_and_time_context(
        ctx: &SessionContext,
        dataframe: DataFrame,
        transforms: Vec<DataTransformStage>,
        params: &IndexMap<String, ScalarValue>,
        time_context: TimeContext,
    ) -> Vec<RecordBatch> {
        let result = avenger_chart_core::apply_compiled_data_transforms(
            dataframe,
            &transforms,
            &DataTransformExecutionContext {
                session_context: ctx,
                params,
                time_context,
                facet_context: None,
            },
        )
        .await
        .unwrap();
        if let Some(param_values) = avenger_chart_core::params_to_datafusion(params) {
            result
                .dataframe
                .with_param_values(param_values)
                .unwrap()
                .collect()
                .await
                .unwrap()
        } else {
            result.dataframe.collect().await.unwrap()
        }
    }

    fn compile_transform<T: DataTransform>(transform: T) -> (DataTransformStage, T::Output) {
        compile_transform_with_scope(CoordinationScope::Free, transform)
    }

    fn compile_transform_with_scope<T: DataTransform>(
        scope: CoordinationScope,
        transform: T,
    ) -> (DataTransformStage, T::Output) {
        let (compiled, output) = transform
            .into_compiled_and_output(DataTransformCompileContext::new(scope))
            .unwrap();
        (DataTransformStage::new(scope, compiled), output)
    }

    fn rasterize_dataframe(
        ctx: &SessionContext,
        groups: Vec<Option<&str>>,
        xs: Vec<Option<f64>>,
        ys: Vec<Option<f64>>,
        values: Vec<Option<f64>>,
    ) -> DataFrame {
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("group", DataType::Utf8, true),
                Field::new("x", DataType::Float64, true),
                Field::new("y", DataType::Float64, true),
                Field::new("value", DataType::Float64, true),
            ])),
            vec![
                Arc::new(StringArray::from(groups)) as _,
                Arc::new(Float64Array::from(xs)) as _,
                Arc::new(Float64Array::from(ys)) as _,
                Arc::new(Float64Array::from(values)) as _,
            ],
        )
        .unwrap();
        ctx.read_batch(batch).unwrap()
    }

    fn raster_struct(batch: &RecordBatch) -> &StructArray {
        batch
            .column_by_name("raster")
            .unwrap()
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap()
    }

    fn raster_count_values(batch: &RecordBatch, row: usize) -> Vec<u64> {
        let values = raster_struct(batch)
            .column_by_name("values")
            .unwrap()
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();
        let data = values
            .column_by_name("data")
            .unwrap()
            .as_any()
            .downcast_ref::<ListArray>()
            .unwrap();
        let row_values = data.value(row);
        row_values
            .as_any()
            .downcast_ref::<UInt64Array>()
            .unwrap()
            .values()
            .to_vec()
    }

    fn raster_f64_values(batch: &RecordBatch, row: usize) -> Vec<Option<f64>> {
        let values = raster_struct(batch)
            .column_by_name("values")
            .unwrap()
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();
        let data = values
            .column_by_name("data")
            .unwrap()
            .as_any()
            .downcast_ref::<ListArray>()
            .unwrap();
        let row_values = data.value(row);
        let row_values = row_values.as_any().downcast_ref::<Float64Array>().unwrap();
        (0..row_values.len())
            .map(|index| {
                if row_values.is_null(index) {
                    None
                } else {
                    Some(row_values.value(index))
                }
            })
            .collect()
    }

    async fn rasterize_executor_batch(
        ctx: &SessionContext,
        dataframe: DataFrame,
        transform: Rasterize2D,
        params: IndexMap<String, ScalarValue>,
    ) -> RecordBatch {
        let (stage, _) = compile_transform(transform);
        let materialization_ctx = ViewMaterializationContext {
            session_context: ctx,
            params: &params,
            time_context: TimeContext::default(),
            facet_context: None,
            policy: MaterializationPolicy::default(),
            priority: 0.0,
        };
        let materialization = stage
            .transform
            .view_materialization_request(&dataframe, &materialization_ctx)
            .unwrap()
            .expect("Rasterize2D should opt into view materialization");
        assert_eq!(
            materialization.request.kind.as_ref(),
            RASTERIZE_2D_MATERIALIZATION_KIND
        );
        assert_eq!(
            materialization.request.output_kind,
            MaterializationOutputKind::RecordBatch
        );
        assert!(materialization.empty_dataframe.is_some());
        let result = Rasterize2DExecutor
            .run(
                materialization.request,
                MaterializationExecutionContext {
                    session_context: ctx,
                    params: &IndexMap::new(),
                },
            )
            .await
            .unwrap();
        match result {
            MaterializationResult::RecordBatch(batch) => batch,
            other => panic!("expected record batch result, got {other:?}"),
        }
    }

    fn assert_option_f64_close(actual: &[Option<f64>], expected: &[Option<f64>]) {
        assert_eq!(actual.len(), expected.len());
        for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            match (actual, expected) {
                (Some(actual), Some(expected)) => assert!(
                    (actual - expected).abs() < 1e-12,
                    "index {index}: expected {expected}, got {actual}"
                ),
                (None, None) => {}
                other => panic!("index {index}: expected {expected:?}, got {other:?}"),
            }
        }
    }

    fn raster_uniform_dimension(
        batch: &RecordBatch,
        row: usize,
        dimension: usize,
    ) -> (String, String, f64, f64, u32) {
        let geometry = raster_struct(batch)
            .column_by_name("geometry")
            .unwrap()
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();
        let dimensions = geometry
            .column_by_name("dimensions")
            .unwrap()
            .as_any()
            .downcast_ref::<ListArray>()
            .unwrap();
        let row_dimensions = dimensions.value(row);
        let row_dimensions = row_dimensions
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();
        let names = row_dimensions
            .column_by_name("name")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let coords = row_dimensions
            .column_by_name("coords")
            .unwrap()
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();
        let samplings = coords
            .column_by_name("sampling")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let starts = coords
            .column_by_name("start")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        let stops = coords
            .column_by_name("stop")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        let counts = coords
            .column_by_name("count")
            .unwrap()
            .as_any()
            .downcast_ref::<UInt32Array>()
            .unwrap();
        (
            names.value(dimension).to_string(),
            samplings.value(dimension).to_string(),
            starts.value(dimension),
            stops.value(dimension),
            counts.value(dimension),
        )
    }

    fn bin_rows(batch: &RecordBatch) -> Vec<BinRow> {
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

    fn bin_rows_from_batches(batches: &[RecordBatch]) -> Vec<BinRow> {
        batches.iter().flat_map(bin_rows).collect()
    }

    fn kde_rows(batch: &RecordBatch, value_name: &str, density_name: &str) -> Vec<(f64, f64)> {
        let value = batch
            .column_by_name(value_name)
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        let density = batch
            .column_by_name(density_name)
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        (0..batch.num_rows())
            .map(|row| (value.value(row), density.value(row)))
            .collect()
    }

    fn kde_rows_from_batches(
        batches: &[RecordBatch],
        value_name: &str,
        density_name: &str,
    ) -> Vec<(f64, f64)> {
        batches
            .iter()
            .flat_map(|batch| kde_rows(batch, value_name, density_name))
            .collect()
    }

    fn kde_group_rows(batch: &RecordBatch, series_name: &str) -> Vec<(String, f64, f64)> {
        let series = batch
            .column_by_name("series")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let value = batch
            .column_by_name(series_name)
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        let density = batch
            .column_by_name("density")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        (0..batch.num_rows())
            .map(|row| {
                (
                    series.value(row).to_string(),
                    value.value(row),
                    density.value(row),
                )
            })
            .collect()
    }

    fn kde_group_rows_from_batches(
        batches: &[RecordBatch],
        value_name: &str,
    ) -> Vec<(String, f64, f64)> {
        batches
            .iter()
            .flat_map(|batch| kde_group_rows(batch, value_name))
            .collect()
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

    fn lump_rows(batch: &RecordBatch) -> Vec<LumpRow> {
        let category = batch
            .column_by_name("category")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let value = batch
            .column_by_name("value")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        let lump_value = batch
            .column_by_name("category_lump_value")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let rank = batch
            .column_by_name("category_lump_rank")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        let is_other = batch
            .column_by_name("category_lump_is_other")
            .unwrap()
            .as_any()
            .downcast_ref::<BooleanArray>()
            .unwrap();
        (0..batch.num_rows())
            .map(|index| {
                (
                    (!category.is_null(index)).then(|| category.value(index).to_string()),
                    value.value(index),
                    (!lump_value.is_null(index)).then(|| lump_value.value(index).to_string()),
                    (!rank.is_null(index)).then(|| rank.value(index)),
                    is_other.value(index),
                )
            })
            .collect()
    }

    fn lump_rows_from_batches(batches: &[RecordBatch]) -> Vec<LumpRow> {
        batches.iter().flat_map(lump_rows).collect()
    }

    fn fold_rows(
        batch: &RecordBatch,
        key_name: &str,
        value_name: &str,
        index_name: Option<&str>,
    ) -> Vec<(String, String, f64, Option<i64>)> {
        let region = batch
            .column_by_name("region")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let key = batch
            .column_by_name(key_name)
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let value = batch
            .column_by_name(value_name)
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        let index = index_name.map(|name| {
            batch
                .column_by_name(name)
                .unwrap()
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
        });
        (0..batch.num_rows())
            .map(|row| {
                (
                    region.value(row).to_string(),
                    key.value(row).to_string(),
                    value.value(row),
                    index.as_ref().map(|array| array.value(row)),
                )
            })
            .collect()
    }

    fn fold_rows_from_batches(
        batches: &[RecordBatch],
        key_name: &str,
        value_name: &str,
        index_name: Option<&str>,
    ) -> Vec<(String, String, f64, Option<i64>)> {
        batches
            .iter()
            .flat_map(|batch| fold_rows(batch, key_name, value_name, index_name))
            .collect()
    }

    fn joinaggregate_rows(batch: &RecordBatch) -> Vec<(String, String, f64, f64)> {
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
        let value = batch
            .column_by_name("value")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        let total = batch
            .column_by_name("category_total")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        (0..batch.num_rows())
            .map(|index| {
                (
                    category.value(index).to_string(),
                    series.value(index).to_string(),
                    value.value(index),
                    total.value(index),
                )
            })
            .collect()
    }

    fn joinaggregate_rows_from_batches(batches: &[RecordBatch]) -> Vec<(String, String, f64, f64)> {
        batches.iter().flat_map(joinaggregate_rows).collect()
    }

    fn aggregate_stat_rows_from_batches(batches: &[RecordBatch]) -> Vec<(String, f64, f64, f64)> {
        batches
            .iter()
            .flat_map(|batch| {
                let category = batch
                    .column_by_name("category")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap();
                let median = batch
                    .column_by_name("median_value")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<Float64Array>()
                    .unwrap();
                let q1 = batch
                    .column_by_name("q1_value")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<Float64Array>()
                    .unwrap();
                let q3 = batch
                    .column_by_name("q3_value")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<Float64Array>()
                    .unwrap();
                (0..batch.num_rows())
                    .map(|index| {
                        (
                            category.value(index).to_string(),
                            median.value(index),
                            q1.value(index),
                            q3.value(index),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn timeunit_rows(batch: &RecordBatch) -> Vec<(Option<i64>, Option<i64>, Option<i64>)> {
        let value = batch
            .column_by_name("timestamp")
            .unwrap()
            .as_any()
            .downcast_ref::<TimestampMillisecondArray>()
            .unwrap();
        let start = batch
            .column_by_name("timestamp_timeunit_start")
            .unwrap()
            .as_any()
            .downcast_ref::<TimestampMillisecondArray>()
            .unwrap();
        let end = batch
            .column_by_name("timestamp_timeunit_end")
            .unwrap()
            .as_any()
            .downcast_ref::<TimestampMillisecondArray>()
            .unwrap();
        (0..batch.num_rows())
            .map(|row| {
                (
                    (!value.is_null(row)).then(|| value.value(row)),
                    (!start.is_null(row)).then(|| start.value(row)),
                    (!end.is_null(row)).then(|| end.value(row)),
                )
            })
            .collect()
    }

    fn timeunit_rows_from_batches(
        batches: &[RecordBatch],
    ) -> Vec<(Option<i64>, Option<i64>, Option<i64>)> {
        batches.iter().flat_map(timeunit_rows).collect()
    }

    fn int32_values(batch: &RecordBatch, column: &str) -> Vec<Option<i32>> {
        let values = batch
            .column_by_name(column)
            .unwrap()
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap();
        (0..batch.num_rows())
            .map(|row| (!values.is_null(row)).then(|| values.value(row)))
            .collect()
    }

    fn int32_values_from_batches(batches: &[RecordBatch], column: &str) -> Vec<Option<i32>> {
        batches
            .iter()
            .flat_map(|batch| int32_values(batch, column))
            .collect()
    }

    fn scalar_struct_field(struct_array: &StructArray, name: &str) -> ScalarValue {
        let (field_index, _) = struct_array
            .fields()
            .iter()
            .enumerate()
            .find(|(_, field)| field.name() == name)
            .unwrap();
        ScalarValue::try_from_array(struct_array.column(field_index), 0).unwrap()
    }

    fn float_values(batch: &RecordBatch, column: &str) -> Vec<Option<f64>> {
        let values = batch
            .column_by_name(column)
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        (0..batch.num_rows())
            .map(|index| (!values.is_null(index)).then(|| values.value(index)))
            .collect()
    }

    fn float_values_from_batches(batches: &[RecordBatch], column: &str) -> Vec<Option<f64>> {
        batches
            .iter()
            .flat_map(|batch| float_values(batch, column))
            .collect()
    }

    fn bool_values(batch: &RecordBatch, column: &str) -> Vec<Option<bool>> {
        let values = batch
            .column_by_name(column)
            .unwrap()
            .as_any()
            .downcast_ref::<BooleanArray>()
            .unwrap();
        (0..batch.num_rows())
            .map(|index| (!values.is_null(index)).then(|| values.value(index)))
            .collect()
    }

    fn bool_values_from_batches(batches: &[RecordBatch], column: &str) -> Vec<Option<bool>> {
        batches
            .iter()
            .flat_map(|batch| bool_values(batch, column))
            .collect()
    }

    fn string_values(batch: &RecordBatch, column: &str) -> Vec<Option<String>> {
        let values = batch
            .column_by_name(column)
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        (0..batch.num_rows())
            .map(|index| (!values.is_null(index)).then(|| values.value(index).to_string()))
            .collect()
    }

    fn string_values_from_batches(batches: &[RecordBatch], column: &str) -> Vec<Option<String>> {
        batches
            .iter()
            .flat_map(|batch| string_values(batch, column))
            .collect()
    }

    fn int_values(batch: &RecordBatch, column: &str) -> Vec<Option<i64>> {
        let values = batch
            .column_by_name(column)
            .unwrap()
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        (0..batch.num_rows())
            .map(|index| (!values.is_null(index)).then(|| values.value(index)))
            .collect()
    }

    fn int_values_from_batches(batches: &[RecordBatch], column: &str) -> Vec<Option<i64>> {
        batches
            .iter()
            .flat_map(|batch| int_values(batch, column))
            .collect()
    }

    fn impute_rows(batch: &RecordBatch) -> Vec<(String, i64, f64, bool)> {
        let series = batch
            .column_by_name("series")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let month = batch
            .column_by_name("month")
            .unwrap()
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        let value = batch
            .column_by_name("value")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        let imputed = batch
            .column_by_name("was_imputed")
            .unwrap()
            .as_any()
            .downcast_ref::<BooleanArray>()
            .unwrap();
        (0..batch.num_rows())
            .map(|row| {
                (
                    series.value(row).to_string(),
                    month.value(row),
                    value.value(row),
                    imputed.value(row),
                )
            })
            .collect()
    }

    fn impute_rows_from_batches(batches: &[RecordBatch]) -> Vec<(String, i64, f64, bool)> {
        batches.iter().flat_map(impute_rows).collect()
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

    fn period_segment_totals_from_batches(
        batches: &[RecordBatch],
        value_col: &str,
    ) -> Vec<(i32, i32, String, f64)> {
        let years = int32_values_from_batches(batches, "period_year");
        let months = int32_values_from_batches(batches, "period_month");
        let segments = string_values_from_batches(batches, "segment");
        let values = float_values_from_batches(batches, value_col);
        let mut rows = years
            .into_iter()
            .zip(months)
            .zip(segments)
            .zip(values)
            .map(|(((year, month), segment), value)| {
                (
                    year.unwrap(),
                    month.unwrap(),
                    segment.unwrap(),
                    value.unwrap(),
                )
            })
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| (a.0, a.1, &a.2).cmp(&(b.0, b.1, &b.2)));
        rows
    }

    fn period_segment_fill_rows_from_batches(
        batches: &[RecordBatch],
    ) -> Vec<(i32, i32, String, f64, bool)> {
        let years = int32_values_from_batches(batches, "period_year");
        let months = int32_values_from_batches(batches, "period_month");
        let segments = string_values_from_batches(batches, "segment");
        let values = float_values_from_batches(batches, "total");
        let filled = bool_values_from_batches(batches, "was_time_filled");
        let mut rows = years
            .into_iter()
            .zip(months)
            .zip(segments)
            .zip(values)
            .zip(filled)
            .map(|((((year, month), segment), value), filled)| {
                (
                    year.unwrap(),
                    month.unwrap(),
                    segment.unwrap(),
                    value.unwrap(),
                    filled.unwrap(),
                )
            })
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| (a.0, a.1, &a.2).cmp(&(b.0, b.1, &b.2)));
        rows
    }

    fn period_segment_stack_rows_from_batches(
        batches: &[RecordBatch],
    ) -> Vec<(i32, i32, String, f64, f64, f64)> {
        let years = int32_values_from_batches(batches, "period_year");
        let months = int32_values_from_batches(batches, "period_month");
        let segments = string_values_from_batches(batches, "segment");
        let values = float_values_from_batches(batches, "total");
        let starts = float_values_from_batches(batches, "total_stack_start");
        let ends = float_values_from_batches(batches, "total_stack_end");
        let mut rows = years
            .into_iter()
            .zip(months)
            .zip(segments)
            .zip(values)
            .zip(starts)
            .zip(ends)
            .map(|(((((year, month), segment), value), start), end)| {
                (
                    year.unwrap(),
                    month.unwrap(),
                    segment.unwrap(),
                    value.unwrap(),
                    start.unwrap(),
                    end.unwrap(),
                )
            })
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| (a.0, a.1, &a.2).cmp(&(b.0, b.1, &b.2)));
        rows
    }

    #[tokio::test]
    async fn calculate_appends_multiple_columns() {
        let ctx = SessionContext::new();
        let dataframe = sample_dataframe(&ctx);
        let (compiled_transform, _) = compile_transform(
            Calculate::new()
                .expr("double_value", col("value") * lit(2.0))
                .expr("shifted_value", col("value") + lit(10.0)),
        );

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        assert_eq!(
            float_values_from_batches(&batches, "double_value"),
            vec![Some(2.0), Some(4.0), Some(-6.0), Some(8.0)]
        );
        assert_eq!(
            float_values_from_batches(&batches, "shifted_value"),
            vec![Some(11.0), Some(12.0), Some(7.0), Some(14.0)]
        );
    }

    #[tokio::test]
    async fn calculate_can_replace_existing_column_and_use_params() {
        let ctx = SessionContext::new();
        let dataframe = sample_dataframe(&ctx);
        let offset = Param::new("offset", ScalarValue::Float64(Some(5.0)));
        let (compiled_transform, _) =
            compile_transform(Calculate::new().expr("value", col("value") + offset.expr()));
        let mut params = IndexMap::new();
        params.insert(offset.name.clone(), ScalarValue::Float64(Some(5.0)));

        let batches =
            transformed_batches_with_params(&ctx, dataframe, vec![compiled_transform], &params)
                .await;
        assert_eq!(
            float_values_from_batches(&batches, "value"),
            vec![Some(6.0), Some(7.0), Some(2.0), Some(9.0)]
        );
    }

    #[tokio::test]
    async fn calculate_repeated_expr_name_overwrites_builder_value() {
        let ctx = SessionContext::new();
        let dataframe = sample_dataframe(&ctx);
        let (compiled_transform, _) = compile_transform(
            Calculate::new()
                .expr("derived", col("value") + lit(100.0))
                .expr("derived", col("value") * lit(3.0)),
        );

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        assert_eq!(
            float_values_from_batches(&batches, "derived"),
            vec![Some(3.0), Some(6.0), Some(-9.0), Some(12.0)]
        );
    }

    #[tokio::test]
    async fn filter_threshold_removes_false_and_null_rows() {
        let ctx = SessionContext::new();
        let dataframe = bin_dataframe(&ctx);
        let (compiled_transform, _) = compile_transform(Filter::new(col("value").gt(lit(2.0))));

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        assert_eq!(
            float_values_from_batches(&batches, "value"),
            vec![Some(3.0), Some(4.0), Some(5.0)]
        );
    }

    #[tokio::test]
    async fn filter_predicate_can_use_params() {
        let ctx = SessionContext::new();
        let dataframe = bin_dataframe(&ctx);
        let threshold = Param::new("threshold", ScalarValue::Float64(Some(3.0)));
        let (compiled_transform, _) =
            compile_transform(Filter::new(col("value").gt(threshold.expr())));
        let mut params = IndexMap::new();
        params.insert(threshold.name.clone(), ScalarValue::Float64(Some(3.0)));

        let batches =
            transformed_batches_with_params(&ctx, dataframe, vec![compiled_transform], &params)
                .await;
        assert_eq!(
            float_values_from_batches(&batches, "value"),
            vec![Some(4.0), Some(5.0)]
        );
    }

    #[tokio::test]
    async fn filter_casts_predicate_to_nullable_boolean() {
        let ctx = SessionContext::new();
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new("keep", DataType::Int64, true)])),
            vec![Arc::new(Int64Array::from(vec![Some(1), Some(0), Some(-2), None])) as _],
        )
        .unwrap();
        let dataframe = ctx.read_batch(batch).unwrap();
        let (compiled_transform, _) = compile_transform(Filter::new(col("keep")));

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        assert_eq!(
            int_values_from_batches(&batches, "keep"),
            vec![Some(1), Some(-2)]
        );
    }

    #[tokio::test]
    async fn select_projects_columns_and_aliased_expressions() {
        let ctx = SessionContext::new();
        let dataframe = sample_dataframe(&ctx);
        let (compiled_transform, _) = compile_transform(
            Select::new()
                .expr(col("category"))
                .expr((col("value") - lit(1.0)).alias("residual")),
        );

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let schema = batches[0].schema();
        let names = schema
            .fields()
            .iter()
            .map(|field| field.name())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["category", "residual"]);
        assert_eq!(
            float_values_from_batches(&batches, "residual"),
            vec![Some(0.0), Some(1.0), Some(-4.0), Some(3.0)]
        );
    }

    #[tokio::test]
    async fn select_expression_can_use_params() {
        let ctx = SessionContext::new();
        let dataframe = sample_dataframe(&ctx);
        let offset = Param::new("select_offset", ScalarValue::Float64(Some(2.5)));
        let (compiled_transform, _) = compile_transform(
            Select::new().expr((col("value") + offset.expr()).alias("shifted_value")),
        );
        let mut params = IndexMap::new();
        params.insert(offset.name.clone(), ScalarValue::Float64(Some(2.5)));

        let batches =
            transformed_batches_with_params(&ctx, dataframe, vec![compiled_transform], &params)
                .await;
        assert_eq!(
            float_values_from_batches(&batches, "shifted_value"),
            vec![Some(3.5), Some(4.5), Some(-0.5), Some(6.5)]
        );
    }

    #[tokio::test]
    async fn select_rejects_unaliased_computed_expression() {
        let err = match Select::new()
            .expr(col("value") - lit(1.0))
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free))
        {
            Ok(_) => panic!("unaliased computed select expression should fail"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("must be a source column or have an explicit alias"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn select_rejects_duplicate_output_names() {
        let err = match Select::new()
            .expr(col("value"))
            .expr((col("value") + lit(1.0)).alias("value"))
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free))
        {
            Ok(_) => panic!("duplicate select output names should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("duplicated"), "{err}");
    }

    #[tokio::test]
    async fn fold_two_fields_produces_rows_per_input() {
        let ctx = SessionContext::new();
        let dataframe = fold_dataframe(&ctx);
        let (compiled_transform, output) = compile_transform(
            Fold::new()
                .field("gold", col("gold"))
                .field("silver", col("silver")),
        );
        assert_eq!(output.key().to_string(), "key");
        assert_eq!(output.value().to_string(), "value");

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let mut rows = fold_rows_from_batches(&batches, "key", "value", None);
        rows.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        assert_eq!(
            rows,
            vec![
                ("North".to_string(), "gold".to_string(), 3.0, None),
                ("North".to_string(), "silver".to_string(), 2.0, None),
                ("South".to_string(), "gold".to_string(), 1.0, None),
                ("South".to_string(), "silver".to_string(), 4.0, None),
            ]
        );
    }

    #[tokio::test]
    async fn fold_computed_values_params_names_and_index_work() {
        let ctx = SessionContext::new();
        let dataframe = fold_dataframe(&ctx);
        let bonus = Param::new("fold_bonus", ScalarValue::Float64(Some(10.0)));
        let (compiled_transform, output) = compile_transform(
            Fold::new()
                .field("medal_score", col("gold") * lit(3.0) + col("silver"))
                .field("bonus_score", col("bronze") + bonus.expr())
                .as_key("medal")
                .as_value("score")
                .index("medal_index"),
        );
        assert_eq!(output.key().to_string(), "medal");
        assert_eq!(output.value().to_string(), "score");
        assert_eq!(output.index().to_string(), "medal_index");
        let params = IndexMap::from([(bonus.name.clone(), bonus.default.clone())]);

        let batches =
            transformed_batches_with_params(&ctx, dataframe, vec![compiled_transform], &params)
                .await;
        let mut rows = fold_rows_from_batches(&batches, "medal", "score", Some("medal_index"));
        rows.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        assert_eq!(
            rows,
            vec![
                (
                    "North".to_string(),
                    "bonus_score".to_string(),
                    15.0,
                    Some(1)
                ),
                (
                    "North".to_string(),
                    "medal_score".to_string(),
                    11.0,
                    Some(0)
                ),
                (
                    "South".to_string(),
                    "bonus_score".to_string(),
                    12.0,
                    Some(1)
                ),
                ("South".to_string(), "medal_score".to_string(), 7.0, Some(0)),
            ]
        );
    }

    #[tokio::test]
    async fn fold_rejects_missing_fields_and_duplicate_outputs() {
        let err = match Fold::new()
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free))
        {
            Ok(_) => panic!("fold without fields should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("at least one field"), "{err}");

        let err = match Fold::new()
            .field("gold", col("gold"))
            .as_key("folded")
            .as_value("folded")
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free))
        {
            Ok(_) => panic!("duplicate fold output names should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("duplicated"), "{err}");
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
                params: &IndexMap::new(),
                time_context: TimeContext::default(),
                facet_context: None,
            },
        )
        .await
        .unwrap();
        let batches = result.dataframe.collect().await.unwrap();
        let rows: usize = batches.iter().map(|batch| batch.num_rows()).sum();
        assert_eq!(rows, 4);
    }

    #[tokio::test]
    async fn aggregate_computes_median_and_percentiles() {
        let ctx = SessionContext::new();
        let dataframe = stats_dataframe(&ctx);
        let (compiled_transform, output) = compile_transform(
            Aggregate::new()
                .group_by([col("category")])
                .median("median_value", col("value"))
                .approx_percentile_cont("q1_value", col("value"), 0.25)
                .approx_percentile_cont_with_centroids("q3_value", col("value"), 0.75, 200),
        );
        assert_eq!(output.output("median_value").to_string(), "median_value");
        assert_eq!(output.output("q1_value").to_string(), "q1_value");
        assert_eq!(output.output("q3_value").to_string(), "q3_value");

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let mut rows = aggregate_stat_rows_from_batches(&batches);
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(rows.len(), 2);

        assert_eq!(rows[0].0, "A");
        assert!((rows[0].1 - 2.5).abs() < 1e-9, "{rows:?}");
        assert!(rows[0].2 <= rows[0].1, "{rows:?}");
        assert!(rows[0].3 >= rows[0].1, "{rows:?}");

        assert_eq!(rows[1].0, "B");
        assert!((rows[1].1 - 25.0).abs() < 1e-9, "{rows:?}");
        assert!(rows[1].2 <= rows[1].1, "{rows:?}");
        assert!(rows[1].3 >= rows[1].1, "{rows:?}");
    }

    #[tokio::test]
    async fn aggregate_rejects_invalid_percentile_options() {
        let err = match Aggregate::new()
            .approx_percentile_cont("bad", col("value"), f64::NAN)
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free))
        {
            Ok(_) => panic!("invalid percentile should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("invalid percentile"), "{err}");

        let err = match JoinAggregate::new()
            .approx_percentile_cont_with_centroids("bad", col("value"), 0.5, 0)
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free))
        {
            Ok(_) => panic!("invalid centroid count should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("centroid count 0"), "{err}");
    }

    #[tokio::test]
    async fn joinaggregate_global_repeats_aggregate_on_each_row() {
        let ctx = SessionContext::new();
        let dataframe = sample_dataframe(&ctx);
        let (compiled_transform, _) =
            compile_transform(JoinAggregate::new().sum("category_total", col("value")));
        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let mut rows = joinaggregate_rows_from_batches(&batches);
        rows.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        assert_eq!(
            rows,
            vec![
                ("A".to_string(), "s1".to_string(), 1.0, 4.0),
                ("A".to_string(), "s2".to_string(), 2.0, 4.0),
                ("A".to_string(), "s3".to_string(), -3.0, 4.0),
                ("B".to_string(), "s1".to_string(), 4.0, 4.0),
            ]
        );
    }

    #[tokio::test]
    async fn joinaggregate_grouped_repeats_group_values() {
        let ctx = SessionContext::new();
        let dataframe = sample_dataframe(&ctx);
        let (compiled_transform, _) = compile_transform(
            JoinAggregate::new()
                .group_by([col("category")])
                .sum("category_total", col("value")),
        );
        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let mut rows = joinaggregate_rows_from_batches(&batches);
        rows.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        assert_eq!(
            rows,
            vec![
                ("A".to_string(), "s1".to_string(), 1.0, 0.0),
                ("A".to_string(), "s2".to_string(), 2.0, 0.0),
                ("A".to_string(), "s3".to_string(), -3.0, 0.0),
                ("B".to_string(), "s1".to_string(), 4.0, 4.0),
            ]
        );
    }

    #[tokio::test]
    async fn joinaggregate_groups_null_keys_into_their_own_partition() {
        // NULL group keys form a single partition that receives its own
        // aggregate value (Vega joinaggregate semantics, previously provided
        // by IsNotDistinctFrom join predicates) rather than a NULL measure.
        let ctx = SessionContext::new();
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("category", DataType::Utf8, true),
                Field::new("value", DataType::Float64, false),
            ])),
            vec![
                Arc::new(StringArray::from(vec![
                    Some("A"),
                    None,
                    Some("A"),
                    None,
                    Some("B"),
                ])) as _,
                Arc::new(Float64Array::from(vec![1.0, 10.0, 2.0, 20.0, 4.0])) as _,
            ],
        )
        .unwrap();
        let dataframe = ctx.read_batch(batch).unwrap();
        let (compiled_transform, _) = compile_transform(
            JoinAggregate::new()
                .group_by([col("category")])
                .sum("category_total", col("value")),
        );
        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let mut rows = batches
            .iter()
            .flat_map(|batch| {
                let category = batch
                    .column_by_name("category")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap();
                let total = batch
                    .column_by_name("category_total")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<Float64Array>()
                    .unwrap();
                (0..batch.num_rows())
                    .map(|index| {
                        (
                            (category.is_valid(index)).then(|| category.value(index).to_string()),
                            (total.is_valid(index)).then(|| total.value(index)),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(
            rows,
            vec![
                (None, Some(30.0)),
                (None, Some(30.0)),
                (Some("A".to_string()), Some(3.0)),
                (Some("A".to_string()), Some(3.0)),
                (Some("B".to_string()), Some(4.0)),
            ]
        );
    }

    #[tokio::test]
    async fn joinaggregate_multiple_measures_and_count_work() {
        let ctx = SessionContext::new();
        let dataframe = sample_dataframe(&ctx);
        let (compiled_transform, _) = compile_transform(
            JoinAggregate::new()
                .group_by([col("category")])
                .sum("category_total", col("value"))
                .count("category_count"),
        );
        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        assert!(
            batches
                .iter()
                .any(|batch| batch.column_by_name("category_total").is_some())
        );
        let mut counts = batches
            .iter()
            .flat_map(|batch| {
                let count = batch
                    .column_by_name("category_count")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap();
                (0..batch.num_rows())
                    .map(|index| count.value(index))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        counts.sort();
        assert_eq!(counts, vec![1, 3, 3, 3]);
    }

    #[tokio::test]
    async fn joinaggregate_repeats_median_and_percentiles_on_each_row() {
        let ctx = SessionContext::new();
        let dataframe = stats_dataframe(&ctx);
        let (compiled_transform, _) = compile_transform(
            JoinAggregate::new()
                .group_by([col("category")])
                .median("median_value", col("value"))
                .approx_percentile_cont("q1_value", col("value"), 0.25)
                .approx_percentile_cont("q3_value", col("value"), 0.75),
        );
        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let mut rows = aggregate_stat_rows_from_batches(&batches);
        rows.sort_by(|a, b| (&a.0, a.1.to_bits()).cmp(&(&b.0, b.1.to_bits())));
        assert_eq!(rows.len(), 8);
        assert!(rows.iter().take(4).all(|row| row.0 == "A"), "{rows:?}");
        assert!(
            rows.iter().take(4).all(|row| (row.1 - 2.5).abs() < 1e-9),
            "{rows:?}"
        );
        assert!(rows.iter().skip(4).all(|row| row.0 == "B"), "{rows:?}");
        assert!(
            rows.iter().skip(4).all(|row| (row.1 - 25.0).abs() < 1e-9),
            "{rows:?}"
        );
        assert!(rows.iter().all(|row| row.2 <= row.1 && row.3 >= row.1));
    }

    #[tokio::test]
    async fn joinaggregate_params_inside_measure_expressions_work() {
        let ctx = SessionContext::new();
        let dataframe = sample_dataframe(&ctx);
        let (compiled_transform, _) =
            compile_transform(JoinAggregate::new().group_by([col("category")]).sum(
                "category_total",
                col("value") * Param::new("factor", 1.0).expr(),
            ));
        let params = IndexMap::from([("factor".to_string(), ScalarValue::Float64(Some(2.0)))]);
        let batches =
            transformed_batches_with_params(&ctx, dataframe, vec![compiled_transform], &params)
                .await;
        let mut rows = joinaggregate_rows_from_batches(&batches);
        rows.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        assert_eq!(
            rows,
            vec![
                ("A".to_string(), "s1".to_string(), 1.0, 0.0),
                ("A".to_string(), "s2".to_string(), 2.0, 0.0),
                ("A".to_string(), "s3".to_string(), -3.0, 0.0),
                ("B".to_string(), "s1".to_string(), 4.0, 8.0),
            ]
        );
    }

    #[tokio::test]
    async fn window_row_number_uses_partition_and_order() {
        let ctx = SessionContext::new();
        let dataframe = window_dataframe(&ctx);
        let (compiled_transform, _) = compile_transform(
            Window::new()
                .partition_by([col("series")])
                .order_by([col("value").sort(false, false)])
                .expr("value_rank", row_number()),
        );

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let mut rows = batches
            .iter()
            .flat_map(|batch| {
                let series = batch
                    .column_by_name("series")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap();
                let day = batch
                    .column_by_name("day")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap();
                let rank = batch
                    .column_by_name("value_rank")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<UInt64Array>()
                    .unwrap();
                (0..batch.num_rows())
                    .map(|row| {
                        (
                            series.value(row).to_string(),
                            day.value(row),
                            rank.value(row),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
        assert_eq!(
            rows,
            vec![
                ("A".to_string(), 1, 2),
                ("A".to_string(), 2, 1),
                ("A".to_string(), 3, 3),
                ("B".to_string(), 1, 1),
                ("B".to_string(), 2, 2),
            ]
        );
    }

    #[tokio::test]
    async fn window_running_sum_lag_and_lead_work() {
        let ctx = SessionContext::new();
        let dataframe = window_dataframe(&ctx);
        let running_sum = Expr::from(WindowFunction::new(
            WindowFunctionDefinition::AggregateUDF(sum_udaf()),
            vec![col("value")],
        ));
        let (compiled_transform, _) = compile_transform(
            Window::new()
                .partition_by([col("series")])
                .order_by([col("day").sort(true, false)])
                .expr("running_total", running_sum)
                .expr("previous_value", lag(col("value"), Some(1), None))
                .expr("next_value", lead(col("value"), Some(1), None)),
        );

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let mut rows = batches
            .iter()
            .flat_map(|batch| {
                let series = batch
                    .column_by_name("series")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap();
                let day = batch
                    .column_by_name("day")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap();
                let total = batch
                    .column_by_name("running_total")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<Float64Array>()
                    .unwrap();
                let previous = batch
                    .column_by_name("previous_value")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<Float64Array>()
                    .unwrap();
                let next = batch
                    .column_by_name("next_value")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<Float64Array>()
                    .unwrap();
                (0..batch.num_rows())
                    .map(|row| {
                        (
                            series.value(row).to_string(),
                            day.value(row),
                            total.value(row),
                            (!previous.is_null(row)).then(|| previous.value(row)),
                            (!next.is_null(row)).then(|| next.value(row)),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
        assert_eq!(
            rows,
            vec![
                ("A".to_string(), 1, 2.0, None, Some(4.0)),
                ("A".to_string(), 2, 6.0, Some(2.0), Some(1.0)),
                ("A".to_string(), 3, 7.0, Some(4.0), None),
                ("B".to_string(), 1, 5.0, None, Some(3.0)),
                ("B".to_string(), 2, 8.0, Some(5.0), None),
            ]
        );
    }

    #[tokio::test]
    async fn window_rank_peers_use_datafusion_expression_semantics() {
        let ctx = SessionContext::new();
        let dataframe = window_peer_dataframe(&ctx);
        let (compiled_transform, _) = compile_transform(
            Window::new()
                .partition_by([col("series")])
                .order_by([col("value").sort(false, false)])
                .expr("ranked", rank())
                .expr("dense_ranked", dense_rank()),
        );
        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let mut rows = batches
            .iter()
            .flat_map(|batch| {
                let series = batch
                    .column_by_name("series")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap();
                let day = batch
                    .column_by_name("day")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap();
                let rank = batch
                    .column_by_name("ranked")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<UInt64Array>()
                    .unwrap();
                let dense_rank = batch
                    .column_by_name("dense_ranked")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<UInt64Array>()
                    .unwrap();
                (0..batch.num_rows())
                    .map(|row| {
                        (
                            series.value(row).to_string(),
                            day.value(row),
                            rank.value(row),
                            dense_rank.value(row),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        rows.sort();
        assert_eq!(
            rows,
            vec![
                ("A".to_string(), 1, 1, 1),
                ("A".to_string(), 2, 1, 1),
                ("A".to_string(), 3, 3, 2),
                ("B".to_string(), 1, 1, 1),
                ("B".to_string(), 2, 1, 1),
            ]
        );
    }

    #[tokio::test]
    async fn window_order_by_accepts_params() {
        let ctx = SessionContext::new();
        let dataframe = window_dataframe(&ctx);
        let direction = Param::new("direction", -1_i64);
        let (compiled_transform, _) = compile_transform(
            Window::new()
                .partition_by([col("series")])
                .order_by([(col("day") * direction.expr()).sort(true, false)])
                .expr("ordered", row_number()),
        );
        let params = IndexMap::from([("direction".to_string(), ScalarValue::Int64(Some(-1)))]);
        let batches =
            transformed_batches_with_params(&ctx, dataframe, vec![compiled_transform], &params)
                .await;
        let mut rows = batches
            .iter()
            .flat_map(|batch| {
                let series = batch
                    .column_by_name("series")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap();
                let day = batch
                    .column_by_name("day")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap();
                let ordered = batch
                    .column_by_name("ordered")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<UInt64Array>()
                    .unwrap();
                (0..batch.num_rows())
                    .map(|row| {
                        (
                            series.value(row).to_string(),
                            day.value(row),
                            ordered.value(row),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
        assert_eq!(
            rows,
            vec![
                ("A".to_string(), 1, 3),
                ("A".to_string(), 2, 2),
                ("A".to_string(), 3, 1),
                ("B".to_string(), 1, 2),
                ("B".to_string(), 2, 1),
            ]
        );
    }

    #[tokio::test]
    async fn window_allows_unordered_partition_aggregate() {
        let ctx = SessionContext::new();
        let dataframe = window_dataframe(&ctx);
        let series_total = Expr::from(WindowFunction::new(
            WindowFunctionDefinition::AggregateUDF(sum_udaf()),
            vec![col("value")],
        ));
        let (compiled_transform, _) = compile_transform(
            Window::new()
                .partition_by([col("series")])
                .expr("series_total", series_total),
        );

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let mut rows = batches
            .iter()
            .flat_map(|batch| {
                let series = batch
                    .column_by_name("series")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap();
                let day = batch
                    .column_by_name("day")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap();
                let total = batch
                    .column_by_name("series_total")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<Float64Array>()
                    .unwrap();
                (0..batch.num_rows())
                    .map(|row| {
                        (
                            series.value(row).to_string(),
                            day.value(row),
                            total.value(row),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
        assert_eq!(
            rows,
            vec![
                ("A".to_string(), 1, 7.0),
                ("A".to_string(), 2, 7.0),
                ("A".to_string(), 3, 7.0),
                ("B".to_string(), 1, 8.0),
                ("B".to_string(), 2, 8.0),
            ]
        );
    }

    #[tokio::test]
    async fn window_rejects_missing_order_by() {
        let ctx = SessionContext::new();
        let dataframe = window_dataframe(&ctx);
        let (compiled_transform, _) =
            compile_transform(Window::new().expr("row_number", row_number()));

        let result = avenger_chart_core::apply_compiled_data_transforms(
            dataframe,
            &[compiled_transform],
            &DataTransformExecutionContext {
                session_context: &ctx,
                params: &IndexMap::new(),
                time_context: TimeContext::default(),
                facet_context: None,
            },
        )
        .await;
        let err = match result {
            Ok(_) => panic!("window without order_by should fail"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("requires an explicit order_by"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn impute_value_fill_creates_missing_key_rows_per_group() {
        let ctx = SessionContext::new();
        let dataframe = impute_dataframe(&ctx);
        let (compiled_transform, output) = compile_transform(
            Impute::new(col("value"))
                .key(col("month"))
                .group_by([col("series")])
                .value(lit(0.0))
                .flag("was_imputed"),
        );
        assert_eq!(output.value().to_string(), "value");
        assert_eq!(output.flag().to_string(), "was_imputed");

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let mut rows = impute_rows_from_batches(&batches);
        rows.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
        assert_eq!(
            rows,
            vec![
                ("A".to_string(), 1, 2.0, false),
                ("A".to_string(), 2, 0.0, true),
                ("A".to_string(), 3, 6.0, false),
                ("B".to_string(), 1, 1.0, false),
                ("B".to_string(), 2, 0.0, false),
                ("B".to_string(), 3, 0.0, true),
            ]
        );
    }

    #[tokio::test]
    async fn impute_mean_fill_uses_group_statistic() {
        let ctx = SessionContext::new();
        let dataframe = impute_dataframe(&ctx);
        let (compiled_transform, _) = compile_transform(
            Impute::new(col("value"))
                .key(col("month"))
                .group_by([col("series")])
                .mean()
                .flag("was_imputed"),
        );

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let mut rows = impute_rows_from_batches(&batches);
        rows.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
        assert_eq!(
            rows,
            vec![
                ("A".to_string(), 1, 2.0, false),
                ("A".to_string(), 2, 4.0, true),
                ("A".to_string(), 3, 6.0, false),
                ("B".to_string(), 1, 1.0, false),
                ("B".to_string(), 2, 1.0, false),
                ("B".to_string(), 3, 1.0, true),
            ]
        );
    }

    #[tokio::test]
    async fn impute_min_and_max_fill_use_group_statistics() {
        for (method, expected_fill) in [("min", 2.0), ("max", 6.0)] {
            let ctx = SessionContext::new();
            let dataframe = impute_dataframe(&ctx);
            let impute = Impute::new(col("value"))
                .key(col("month"))
                .group_by([col("series")])
                .flag("was_imputed");
            let impute = match method {
                "min" => impute.min(),
                "max" => impute.max(),
                _ => unreachable!(),
            };
            let (compiled_transform, _) = compile_transform(impute);

            let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
            let mut rows = impute_rows_from_batches(&batches);
            rows.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
            assert_eq!(
                rows,
                vec![
                    ("A".to_string(), 1, 2.0, false),
                    ("A".to_string(), 2, expected_fill, true),
                    ("A".to_string(), 3, 6.0, false),
                    ("B".to_string(), 1, 1.0, false),
                    ("B".to_string(), 2, 1.0, false),
                    ("B".to_string(), 3, 1.0, true),
                ],
                "{method}"
            );
        }
    }

    #[tokio::test]
    async fn impute_value_fill_accepts_params() {
        let ctx = SessionContext::new();
        let dataframe = impute_dataframe(&ctx);
        let fill = Param::new("fill", 7.0);
        let (compiled_transform, _) = compile_transform(
            Impute::new(col("value"))
                .key(col("month"))
                .group_by([col("series")])
                .value(fill.expr())
                .flag("was_imputed"),
        );
        let params = IndexMap::from([("fill".to_string(), ScalarValue::Float64(Some(7.0)))]);

        let batches =
            transformed_batches_with_params(&ctx, dataframe, vec![compiled_transform], &params)
                .await;
        let mut rows = impute_rows_from_batches(&batches);
        rows.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
        assert_eq!(
            rows,
            vec![
                ("A".to_string(), 1, 2.0, false),
                ("A".to_string(), 2, 7.0, true),
                ("A".to_string(), 3, 6.0, false),
                ("B".to_string(), 1, 1.0, false),
                ("B".to_string(), 2, 7.0, false),
                ("B".to_string(), 3, 7.0, true),
            ]
        );
    }

    #[tokio::test]
    async fn impute_without_group_by_uses_global_key_domain() {
        let ctx = SessionContext::new();
        let dataframe = impute_dataframe(&ctx);
        let (compiled_transform, _) = compile_transform(
            Impute::new(col("value"))
                .key(col("month"))
                .value(lit(0.0))
                .flag("was_imputed"),
        );

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let mut rows = impute_rows_from_batches(&batches);
        rows.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
        assert_eq!(
            rows,
            vec![
                ("A".to_string(), 1, 2.0, false),
                ("A".to_string(), 3, 6.0, false),
                ("B".to_string(), 1, 1.0, false),
                ("B".to_string(), 2, 0.0, false),
            ]
        );
    }

    #[tokio::test]
    async fn impute_requires_key_and_method() {
        let err = match Impute::new(col("value"))
            .value(lit(0.0))
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free))
        {
            Ok(_) => panic!("impute without key should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("requires a key"), "{err}");

        let err = match Impute::new(col("value"))
            .key(col("month"))
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free))
        {
            Ok(_) => panic!("impute without method should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("requires a fill method"), "{err}");
    }

    #[tokio::test]
    async fn timeunit_explicit_month_collapses_to_anchor_year() {
        const DAY_MS: i64 = 86_400_000;
        const JAN_2012_MS: i64 = 1_325_376_000_000;
        const FEB_2012_MS: i64 = 1_328_054_400_000;
        const MAR_2012_MS: i64 = 1_330_560_000_000;
        let ctx = SessionContext::new();
        let dataframe = time_dataframe(&ctx, vec![Some(14 * DAY_MS), Some(32 * DAY_MS), None]);
        let (compiled_transform, output) =
            compile_transform(TimeUnit::new(col("timestamp")).unit(TimeUnitPart::Month));
        assert!(matches!(
            output.start().into_channel_value(),
            ChannelValue::Scaled { .. }
        ));
        assert!(matches!(
            output.end().into_channel_value(),
            ChannelValue::Scaled { .. }
        ));
        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let mut rows = timeunit_rows_from_batches(&batches);
        rows.sort();
        assert_eq!(
            rows,
            vec![
                (None, None, None),
                (Some(14 * DAY_MS), Some(JAN_2012_MS), Some(FEB_2012_MS)),
                (Some(32 * DAY_MS), Some(FEB_2012_MS), Some(MAR_2012_MS)),
            ]
        );
    }

    #[tokio::test]
    async fn timeunit_explicit_year_month_preserves_input_year() {
        const DAY_MS: i64 = 86_400_000;
        let ctx = SessionContext::new();
        let dataframe = time_dataframe(&ctx, vec![Some(14 * DAY_MS), Some(32 * DAY_MS)]);
        let (compiled_transform, _) = compile_transform(
            TimeUnit::new(col("timestamp")).units([TimeUnitPart::Year, TimeUnitPart::Month]),
        );
        let mut rows = timeunit_rows_from_batches(
            &transformed_batches(&ctx, dataframe, vec![compiled_transform]).await,
        );
        rows.sort();
        assert_eq!(
            rows,
            vec![
                (Some(14 * DAY_MS), Some(0), Some(31 * DAY_MS)),
                (Some(32 * DAY_MS), Some(31 * DAY_MS), Some(59 * DAY_MS)),
            ]
        );
    }

    #[tokio::test]
    async fn timeunit_maxbins_selects_lazy_month_candidate() {
        const DAY_MS: i64 = 86_400_000;
        let ctx = SessionContext::new();
        let dataframe = time_dataframe(&ctx, vec![Some(14 * DAY_MS), Some(58 * DAY_MS)]);
        let (compiled_transform, _) = compile_transform(TimeUnit::new(col("timestamp")).maxbins(2));
        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let mut rows = timeunit_rows_from_batches(&batches);
        rows.sort();
        assert_eq!(
            rows,
            vec![
                (Some(14 * DAY_MS), Some(0), Some(31 * DAY_MS)),
                (Some(58 * DAY_MS), Some(31 * DAY_MS), Some(59 * DAY_MS)),
            ]
        );
    }

    #[tokio::test]
    async fn timeunit_maxbins_accepts_param_integer() {
        const DAY_MS: i64 = 86_400_000;
        let ctx = SessionContext::new();
        let dataframe = time_dataframe(&ctx, vec![Some(14 * DAY_MS), Some(58 * DAY_MS)]);
        let maxbins = Param::new("time_maxbins", ScalarValue::Int64(Some(2)));
        let (compiled_transform, _) =
            compile_transform(TimeUnit::new(col("timestamp")).maxbins(maxbins.expr()));
        let mut params = IndexMap::new();
        params.insert(maxbins.name.clone(), ScalarValue::Int64(Some(2)));

        let batches =
            transformed_batches_with_params(&ctx, dataframe, vec![compiled_transform], &params)
                .await;
        let mut rows = timeunit_rows_from_batches(&batches);
        rows.sort();
        assert_eq!(
            rows,
            vec![
                (Some(14 * DAY_MS), Some(0), Some(31 * DAY_MS)),
                (Some(58 * DAY_MS), Some(31 * DAY_MS), Some(59 * DAY_MS)),
            ]
        );
    }

    #[tokio::test]
    async fn timeunit_maxbins_rejects_non_positive_non_integer_and_null() {
        let cases = [
            ("zero", lit(0_i64), "must evaluate to a positive integer"),
            (
                "negative",
                lit(-1_i64),
                "must evaluate to a positive integer",
            ),
            ("float", lit(2.5), "must evaluate to an integer scalar"),
            (
                "null",
                lit(ScalarValue::Int64(None)),
                "must not evaluate to null",
            ),
        ];

        for (name, maxbins_expr, expected) in cases {
            let ctx = SessionContext::new();
            let dataframe = time_dataframe(&ctx, vec![Some(0), Some(86_400_000)]);
            let (compiled_transform, _) =
                compile_transform(TimeUnit::new(col("timestamp")).maxbins(maxbins_expr));
            let err = match avenger_chart_core::apply_compiled_data_transforms(
                dataframe,
                &[compiled_transform],
                &DataTransformExecutionContext {
                    session_context: &ctx,
                    params: &IndexMap::new(),
                    time_context: TimeContext::default(),
                    facet_context: None,
                },
            )
            .await
            {
                Ok(_) => panic!("invalid maxbins should error"),
                Err(err) => err,
            };
            assert!(err.to_string().contains(expected), "{name}: {err}");
        }
    }

    #[tokio::test]
    async fn timeunit_week_start_changes_week_anchor() {
        const DAY_MS: i64 = 86_400_000;
        const SUNDAY_JAN_7_2024_MS: i64 = 1_704_585_600_000;
        let ctx = SessionContext::new();
        let starts = [
            (WeekStart::Sunday, SUNDAY_JAN_7_2024_MS),
            (WeekStart::Monday, SUNDAY_JAN_7_2024_MS - 6 * DAY_MS),
            (WeekStart::Tuesday, SUNDAY_JAN_7_2024_MS - 5 * DAY_MS),
            (WeekStart::Wednesday, SUNDAY_JAN_7_2024_MS - 4 * DAY_MS),
            (WeekStart::Thursday, SUNDAY_JAN_7_2024_MS - 3 * DAY_MS),
            (WeekStart::Friday, SUNDAY_JAN_7_2024_MS - 2 * DAY_MS),
            (WeekStart::Saturday, SUNDAY_JAN_7_2024_MS - DAY_MS),
        ];

        for (week_start, expected_start) in starts {
            let dataframe = time_dataframe(&ctx, vec![Some(SUNDAY_JAN_7_2024_MS)]);
            let (transform, _) = compile_transform(
                TimeUnit::new(col("timestamp"))
                    .unit(TimeUnitPart::Week)
                    .time_context(TimeContext::new().week_start(week_start)),
            );
            let rows = timeunit_rows_from_batches(
                &transformed_batches(&ctx, dataframe, vec![transform]).await,
            );
            assert_eq!(
                rows,
                vec![(
                    Some(SUNDAY_JAN_7_2024_MS),
                    Some(expected_start),
                    Some(expected_start + 7 * DAY_MS)
                )],
                "{week_start:?}"
            );
        }
    }

    #[tokio::test]
    async fn timeunit_inherits_week_start_from_execution_context() {
        const DAY_MS: i64 = 86_400_000;
        const SUNDAY_JAN_7_2024_MS: i64 = 1_704_585_600_000;
        let ctx = SessionContext::new();
        let dataframe = time_dataframe(&ctx, vec![Some(SUNDAY_JAN_7_2024_MS)]);
        let (transform, _) =
            compile_transform(TimeUnit::new(col("timestamp")).unit(TimeUnitPart::Week));
        let rows = timeunit_rows_from_batches(
            &transformed_batches_with_time_context(
                &ctx,
                dataframe,
                vec![transform],
                TimeContext::new().week_start(WeekStart::Monday),
            )
            .await,
        );
        assert_eq!(
            rows,
            vec![(
                Some(SUNDAY_JAN_7_2024_MS),
                Some(SUNDAY_JAN_7_2024_MS - 6 * DAY_MS),
                Some(SUNDAY_JAN_7_2024_MS + DAY_MS)
            )]
        );
    }

    #[tokio::test]
    async fn timeunit_local_week_start_overrides_execution_context() {
        const DAY_MS: i64 = 86_400_000;
        const SUNDAY_JAN_7_2024_MS: i64 = 1_704_585_600_000;
        let ctx = SessionContext::new();
        let dataframe = time_dataframe(&ctx, vec![Some(SUNDAY_JAN_7_2024_MS)]);
        let (transform, _) = compile_transform(
            TimeUnit::new(col("timestamp"))
                .unit(TimeUnitPart::Week)
                .time_context(TimeContext::new().week_start(WeekStart::Sunday)),
        );
        let rows = timeunit_rows_from_batches(
            &transformed_batches_with_time_context(
                &ctx,
                dataframe,
                vec![transform],
                TimeContext::new().week_start(WeekStart::Monday),
            )
            .await,
        );
        assert_eq!(
            rows,
            vec![(
                Some(SUNDAY_JAN_7_2024_MS),
                Some(SUNDAY_JAN_7_2024_MS),
                Some(SUNDAY_JAN_7_2024_MS + 7 * DAY_MS)
            )]
        );
    }

    #[tokio::test]
    async fn timeunit_returns_temporal_tick_spacing_derived_scalar() {
        const DAY_MS: i64 = 86_400_000;
        let ctx = SessionContext::new();
        let dataframe = time_dataframe(&ctx, vec![Some(14 * DAY_MS), Some(58 * DAY_MS)]);
        let (compiled_transform, _) =
            compile_transform(TimeUnit::new(col("timestamp")).unit(TimeUnitPart::Month));
        let result = avenger_chart_core::apply_compiled_data_transforms(
            dataframe,
            &[compiled_transform],
            &DataTransformExecutionContext {
                session_context: &ctx,
                params: &IndexMap::new(),
                time_context: TimeContext::default(),
                facet_context: None,
            },
        )
        .await
        .unwrap();
        let tick_spacing = result
            .derived_scalars
            .get("timestamp_timeunit_tick_spacing")
            .expect("tick spacing scalar")
            .clone();
        let scalars = eval_to_scalars(vec![tick_spacing], Some(&ctx), None)
            .await
            .expect("evaluate tick spacing");
        let ScalarValue::Struct(struct_array) = &scalars[0] else {
            panic!("expected struct tick spacing, got {:?}", scalars[0]);
        };
        let start = scalar_struct_field(struct_array, "start");
        let step = scalar_struct_field(struct_array, "step");
        assert_eq!(
            start,
            ScalarValue::TimestampMillisecond(Some(1_325_376_000_000), None)
        );
        let ScalarValue::IntervalMonthDayNano(Some(step)) = step else {
            panic!("expected interval step, got {step:?}");
        };
        assert_eq!(
            datafusion::arrow::array::types::IntervalMonthDayNanoType::to_parts(step),
            (1, 0, 0)
        );
    }

    #[tokio::test]
    async fn time_levels_appends_expected_key_columns() {
        const DAY_MS: i64 = 86_400_000;
        let ctx = SessionContext::new();
        let dataframe = time_dataframe(&ctx, vec![Some(45 * DAY_MS), None]);
        let (compiled_transform, output) = compile_transform(
            TimeLevels::new(col("timestamp"))
                .year()
                .quarter()
                .month()
                .name("period"),
        );

        assert_eq!(output.key_name(TimeLevel::Year), "period_year");
        assert_eq!(output.key(TimeLevel::Month).to_string(), "period_month");

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        assert_eq!(
            int32_values_from_batches(&batches, "period_year"),
            vec![Some(1970), None]
        );
        assert_eq!(
            int32_values_from_batches(&batches, "period_quarter"),
            vec![Some(1), None]
        );
        assert_eq!(
            int32_values_from_batches(&batches, "period_month"),
            vec![Some(2), None]
        );
    }

    #[test]
    fn time_levels_output_helpers_describe_grouping_and_nested_channel() {
        let (_compiled_transform, output) = compile_transform(
            TimeLevels::new(col("timestamp"))
                .year()
                .quarter()
                .month_with(|level| level.label(TimeLevelLabel::MonthName))
                .name("period"),
        );

        assert_eq!(
            output
                .keys()
                .into_iter()
                .map(|expr| expr.to_string())
                .collect::<Vec<_>>(),
            vec!["period_year", "period_quarter", "period_month"]
        );
        assert_eq!(
            output
                .keys_with([col("segment")])
                .into_iter()
                .map(|expr| expr.to_string())
                .collect::<Vec<_>>(),
            vec!["period_year", "period_quarter", "period_month", "segment"]
        );

        let levels = output.levels();
        assert_eq!(
            levels.levels,
            vec![
                TimeLevelKey {
                    level: TimeLevel::Year,
                    key_name: "period_year".to_string(),
                    label: TimeLevelLabel::Year4,
                },
                TimeLevelKey {
                    level: TimeLevel::Quarter,
                    key_name: "period_quarter".to_string(),
                    label: TimeLevelLabel::QuarterShort,
                },
                TimeLevelKey {
                    level: TimeLevel::Month,
                    key_name: "period_month".to_string(),
                    label: TimeLevelLabel::MonthName,
                },
            ]
        );

        let ctx = SessionContext::new();
        let nested = output.try_nested().expect("nested output");
        let nested_spec = nested
            .channel_value()
            .get_nested_band_config()
            .expect("nested metadata");
        assert_eq!(
            nested_spec.source_columns,
            vec!["period_year", "period_quarter", "period_month"]
        );
        assert_eq!(
            nested_spec
                .level(2)
                .and_then(|level| level.label_expr.as_ref())
                .expect("month label")
                .to_default_expr(&ctx)
                .expect("label expr")
                .to_string(),
            "to_char(make_date(Int32(2000), period_month, Int32(1)), Utf8(\"%B\"))"
        );
    }

    #[tokio::test]
    async fn time_levels_local_timezone_overrides_parent_timezone() {
        let ctx = SessionContext::new();
        let dataframe = time_dataframe(&ctx, vec![Some(1_704_070_800_000)]);
        let (compiled_transform, _) = compile_transform(
            TimeLevels::new(col("timestamp"))
                .year()
                .month()
                .time_context(TimeContext::new().timezone("America/New_York"))
                .name("period"),
        );

        let batches = transformed_batches_with_time_context(
            &ctx,
            dataframe,
            vec![compiled_transform],
            TimeContext::new().timezone("UTC"),
        )
        .await;
        assert_eq!(
            int32_values_from_batches(&batches, "period_year"),
            vec![Some(2023)]
        );
        assert_eq!(
            int32_values_from_batches(&batches, "period_month"),
            vec![Some(12)]
        );
    }

    #[tokio::test]
    async fn time_levels_parent_timezone_is_used_when_local_timezone_is_omitted() {
        let ctx = SessionContext::new();
        let dataframe = time_dataframe(&ctx, vec![Some(1_704_070_800_000)]);
        let (compiled_transform, _) = compile_transform(
            TimeLevels::new(col("timestamp"))
                .year()
                .month()
                .name("period"),
        );

        let batches = transformed_batches_with_time_context(
            &ctx,
            dataframe,
            vec![compiled_transform],
            TimeContext::new().timezone("America/New_York"),
        )
        .await;
        assert_eq!(
            int32_values_from_batches(&batches, "period_year"),
            vec![Some(2023)]
        );
        assert_eq!(
            int32_values_from_batches(&batches, "period_month"),
            vec![Some(12)]
        );
    }

    #[tokio::test]
    async fn time_levels_accepts_week_start_but_rejects_week_levels() {
        let ctx = SessionContext::new();
        let dataframe = time_dataframe(&ctx, vec![Some(0)]);
        let (compiled_transform, _) = compile_transform(
            TimeLevels::new(col("timestamp"))
                .year()
                .time_context(TimeContext::new().week_start(WeekStart::Monday))
                .name("period"),
        );
        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        assert_eq!(
            int32_values_from_batches(&batches, "period_year"),
            vec![Some(1970)]
        );

        let err = match TimeLevels::new(col("timestamp"))
            .level(TimeLevel::Week)
            .name("period")
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free))
        {
            Ok(_) => panic!("week levels should fail"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("does not support week-number or day-of-week levels"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn time_levels_generated_names_must_not_collide_with_input_columns() {
        let ctx = SessionContext::new();
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new(
                    "timestamp",
                    DataType::Timestamp(datafusion::arrow::datatypes::TimeUnit::Millisecond, None),
                    true,
                ),
                Field::new("period_year", DataType::Int32, true),
            ])),
            vec![
                Arc::new(TimestampMillisecondArray::from(vec![Some(0)])) as _,
                Arc::new(Int32Array::from(vec![Some(1970)])) as _,
            ],
        )
        .unwrap();
        let dataframe = ctx.read_batch(batch).unwrap();
        let (compiled_transform, _) = compile_transform(
            TimeLevels::new(col("timestamp"))
                .year()
                .month()
                .name("period"),
        );

        let err = match avenger_chart_core::apply_compiled_data_transforms(
            dataframe,
            &[compiled_transform],
            &DataTransformExecutionContext {
                session_context: &ctx,
                params: &IndexMap::new(),
                time_context: TimeContext::default(),
                facet_context: None,
            },
        )
        .await
        {
            Ok(_) => panic!("colliding output names should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("conflicts with an input column"));
    }

    #[test]
    fn time_levels_rejects_aggregate_values_duplicate_levels_and_invalid_nested_chains() {
        let aggregate_err = match TimeLevels::new(sum(col("value")))
            .year()
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free))
        {
            Ok(_) => panic!("aggregate value should fail"),
            Err(err) => err,
        };
        assert!(
            aggregate_err
                .to_string()
                .contains("does not accept aggregate expressions"),
            "{aggregate_err}"
        );

        let duplicate_err = match TimeLevels::new(col("timestamp"))
            .year()
            .year()
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free))
        {
            Ok(_) => panic!("duplicate level should fail"),
            Err(err) => err,
        };
        assert!(
            duplicate_err.to_string().contains("is duplicated"),
            "{duplicate_err}"
        );

        let (_compiled_transform, output) = compile_transform(
            TimeLevels::new(col("timestamp"))
                .month()
                .year()
                .name("period"),
        );
        let nested_err = output.try_nested().expect_err("invalid nested chain");
        assert!(
            nested_err
                .to_string()
                .contains("requires a supported hierarchical chain"),
            "{nested_err}"
        );
    }

    #[test]
    fn time_levels_nested_accepts_supported_hierarchies() {
        let cases = vec![
            TimeLevels::new(col("timestamp"))
                .year()
                .quarter()
                .name("period"),
            TimeLevels::new(col("timestamp"))
                .year()
                .quarter()
                .month()
                .name("period"),
            TimeLevels::new(col("timestamp"))
                .year()
                .quarter()
                .month()
                .day_of_month()
                .name("period"),
            TimeLevels::new(col("timestamp"))
                .year()
                .month()
                .name("period"),
            TimeLevels::new(col("timestamp"))
                .year()
                .month()
                .day_of_month()
                .name("period"),
            TimeLevels::new(col("timestamp"))
                .year()
                .day_of_year()
                .name("period"),
        ];

        for case in cases {
            let (_compiled_transform, output) = compile_transform(case);
            output.try_nested().expect("supported nested hierarchy");
        }
    }

    #[test]
    fn time_levels_compiled_transform_serializes() {
        let (stage, _output) = compile_transform(
            TimeLevels::new(col("timestamp"))
                .year()
                .quarter()
                .month()
                .time_context(TimeContext::new().timezone("America/New_York"))
                .name("period"),
        );

        let bytes = bincode::serialize(&stage).expect("serialize transform stage");
        let decoded: DataTransformStage = bincode::deserialize(&bytes).expect("deserialize stage");
        let json = serde_json::to_string(&decoded).expect("serialize decoded stage as json");
        assert!(json.contains("time_levels"));
        assert!(json.contains("America/New_York"));
    }

    fn period_month_dataframe(
        ctx: &SessionContext,
        rows: &[(i32, i32, Option<&str>, f64)],
    ) -> DataFrame {
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("period_year", DataType::Int32, false),
                Field::new("period_month", DataType::Int32, false),
                Field::new("segment", DataType::Utf8, true),
                Field::new("total", DataType::Float64, false),
            ])),
            vec![
                Arc::new(Int32Array::from(
                    rows.iter().map(|row| row.0).collect::<Vec<_>>(),
                )) as _,
                Arc::new(Int32Array::from(
                    rows.iter().map(|row| row.1).collect::<Vec<_>>(),
                )) as _,
                Arc::new(StringArray::from(
                    rows.iter().map(|row| row.2).collect::<Vec<_>>(),
                )) as _,
                Arc::new(Float64Array::from(
                    rows.iter().map(|row| row.3).collect::<Vec<_>>(),
                )) as _,
            ],
        )
        .unwrap();
        ctx.read_batch(batch).unwrap()
    }

    fn period_quarter_dataframe(ctx: &SessionContext, rows: &[(i32, i32, f64)]) -> DataFrame {
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("period_year", DataType::Int32, false),
                Field::new("period_quarter", DataType::Int32, false),
                Field::new("total", DataType::Float64, false),
            ])),
            vec![
                Arc::new(Int32Array::from(
                    rows.iter().map(|row| row.0).collect::<Vec<_>>(),
                )) as _,
                Arc::new(Int32Array::from(
                    rows.iter().map(|row| row.1).collect::<Vec<_>>(),
                )) as _,
                Arc::new(Float64Array::from(
                    rows.iter().map(|row| row.2).collect::<Vec<_>>(),
                )) as _,
            ],
        )
        .unwrap();
        ctx.read_batch(batch).unwrap()
    }

    fn period_quarter_month_dataframe(
        ctx: &SessionContext,
        rows: &[(i32, i32, i32, f64)],
    ) -> DataFrame {
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("period_year", DataType::Int32, false),
                Field::new("period_quarter", DataType::Int32, false),
                Field::new("period_month", DataType::Int32, false),
                Field::new("total", DataType::Float64, false),
            ])),
            vec![
                Arc::new(Int32Array::from(
                    rows.iter().map(|row| row.0).collect::<Vec<_>>(),
                )) as _,
                Arc::new(Int32Array::from(
                    rows.iter().map(|row| row.1).collect::<Vec<_>>(),
                )) as _,
                Arc::new(Int32Array::from(
                    rows.iter().map(|row| row.2).collect::<Vec<_>>(),
                )) as _,
                Arc::new(Float64Array::from(
                    rows.iter().map(|row| row.3).collect::<Vec<_>>(),
                )) as _,
            ],
        )
        .unwrap();
        ctx.read_batch(batch).unwrap()
    }

    fn period_day_dataframe(ctx: &SessionContext, rows: &[(i32, i32, i32, f64)]) -> DataFrame {
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("period_year", DataType::Int32, false),
                Field::new("period_month", DataType::Int32, false),
                Field::new("period_day", DataType::Int32, false),
                Field::new("total", DataType::Float64, false),
            ])),
            vec![
                Arc::new(Int32Array::from(
                    rows.iter().map(|row| row.0).collect::<Vec<_>>(),
                )) as _,
                Arc::new(Int32Array::from(
                    rows.iter().map(|row| row.1).collect::<Vec<_>>(),
                )) as _,
                Arc::new(Int32Array::from(
                    rows.iter().map(|row| row.2).collect::<Vec<_>>(),
                )) as _,
                Arc::new(Float64Array::from(
                    rows.iter().map(|row| row.3).collect::<Vec<_>>(),
                )) as _,
            ],
        )
        .unwrap();
        ctx.read_batch(batch).unwrap()
    }

    fn period_year_month_levels() -> TimeLevelKeys {
        let (_stage, output) = compile_transform(
            TimeLevels::new(col("timestamp"))
                .year()
                .month()
                .name("period"),
        );
        output.levels()
    }

    fn period_year_quarter_levels() -> TimeLevelKeys {
        let (_stage, output) = compile_transform(
            TimeLevels::new(col("timestamp"))
                .year()
                .quarter()
                .name("period"),
        );
        output.levels()
    }

    fn period_year_month_day_levels() -> TimeLevelKeys {
        let (_stage, output) = compile_transform(
            TimeLevels::new(col("timestamp"))
                .year()
                .month()
                .day_of_month()
                .name("period"),
        );
        output.levels()
    }

    #[tokio::test]
    async fn time_fill_month_default_extent_fills_missing_months() {
        let ctx = SessionContext::new();
        let dataframe =
            period_month_dataframe(&ctx, &[(2024, 1, None, 10.0), (2024, 3, None, 30.0)]);
        let (compiled_transform, output) = compile_transform(
            TimeFill::new(col("total"))
                .levels(period_year_month_levels())
                .fill_value(lit(0.0))
                .flag("was_time_filled"),
        );
        assert_eq!(output.value().to_string(), "total");
        assert_eq!(output.flag().to_string(), "was_time_filled");

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let mut rows = int32_values_from_batches(&batches, "period_month")
            .into_iter()
            .zip(float_values_from_batches(&batches, "total"))
            .zip(bool_values_from_batches(&batches, "was_time_filled"))
            .map(|((month, total), filled)| (month.unwrap(), total.unwrap(), filled.unwrap()))
            .collect::<Vec<_>>();
        rows.sort_by_key(|row| row.0);
        assert_eq!(
            rows,
            vec![(1, 10.0, false), (2, 0.0, true), (3, 30.0, false)]
        );
    }

    #[tokio::test]
    async fn time_fill_quarter_default_extent_fills_missing_quarters() {
        let ctx = SessionContext::new();
        let dataframe = period_quarter_dataframe(&ctx, &[(2024, 1, 4.0), (2024, 3, 12.0)]);
        let (compiled_transform, _) = compile_transform(
            TimeFill::new(col("total"))
                .levels(period_year_quarter_levels())
                .fill_value(lit(0.0)),
        );

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let mut rows = int32_values_from_batches(&batches, "period_quarter")
            .into_iter()
            .zip(float_values_from_batches(&batches, "total"))
            .map(|(quarter, total)| (quarter.unwrap(), total.unwrap()))
            .collect::<Vec<_>>();
        rows.sort_by_key(|row| row.0);
        assert_eq!(rows, vec![(1, 4.0), (2, 0.0), (3, 12.0)]);
    }

    #[tokio::test]
    async fn time_fill_component_extent_adds_leading_and_trailing_periods() {
        let ctx = SessionContext::new();
        let dataframe = period_month_dataframe(&ctx, &[(2024, 2, None, 20.0)]);
        let (compiled_transform, _) = compile_transform(
            TimeFill::new(col("total"))
                .levels(period_year_month_levels())
                .extent([lit(2024_i32), lit(1_i32)], [lit(2024_i32), lit(4_i32)])
                .fill_value(lit(0.0)),
        );

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let mut rows = int32_values_from_batches(&batches, "period_month")
            .into_iter()
            .zip(float_values_from_batches(&batches, "total"))
            .map(|(month, total)| (month.unwrap(), total.unwrap()))
            .collect::<Vec<_>>();
        rows.sort_by_key(|row| row.0);
        assert_eq!(rows, vec![(1, 0.0), (2, 20.0), (3, 0.0), (4, 0.0)]);
    }

    #[tokio::test]
    async fn time_fill_group_by_cross_joins_each_group_with_spine() {
        let ctx = SessionContext::new();
        let dataframe = period_month_dataframe(
            &ctx,
            &[
                (2024, 1, Some("A"), 10.0),
                (2024, 3, Some("A"), 30.0),
                (2024, 2, Some("B"), 20.0),
            ],
        );
        let (compiled_transform, _) = compile_transform(
            TimeFill::new(col("total"))
                .levels(period_year_month_levels())
                .group_by([col("segment")])
                .fill_value(lit(0.0)),
        );

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let segments = string_values_from_batches(&batches, "segment");
        let months = int32_values_from_batches(&batches, "period_month");
        let totals = float_values_from_batches(&batches, "total");
        let mut rows = segments
            .into_iter()
            .zip(months)
            .zip(totals)
            .map(|((segment, month), total)| (segment.unwrap(), month.unwrap(), total.unwrap()))
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
        assert_eq!(
            rows,
            vec![
                ("A".to_string(), 1, 10.0),
                ("A".to_string(), 2, 0.0),
                ("A".to_string(), 3, 30.0),
                ("B".to_string(), 1, 0.0),
                ("B".to_string(), 2, 20.0),
                ("B".to_string(), 3, 0.0),
            ]
        );
    }

    #[tokio::test]
    async fn time_fill_rejects_invalid_extents_and_components() {
        let ctx = SessionContext::new();
        let dataframe = period_quarter_month_dataframe(&ctx, &[(2024, 1, 2, 20.0)]);

        let wrong_count = TimeFill::new(col("total"))
            .levels(period_year_month_levels())
            .extent([lit(2024_i32)], [lit(2024_i32), lit(4_i32)])
            .fill_value(lit(0.0))
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free));
        assert!(matches!(
            wrong_count,
            Err(AvengerChartError::InvalidArgument(_))
        ));

        let cases = [
            (
                TimeFill::new(col("total"))
                    .levels(period_year_month_levels())
                    .extent(
                        [col("period_year"), lit(1_i32)],
                        [lit(2024_i32), lit(4_i32)],
                    )
                    .fill_value(lit(0.0)),
                "integer scalars",
            ),
            (
                TimeFill::new(col("total"))
                    .levels(period_year_month_levels())
                    .extent([lit(2024.5), lit(1_i32)], [lit(2024_i32), lit(4_i32)])
                    .fill_value(lit(0.0)),
                "integer",
            ),
        ];

        for (transform, expected) in cases {
            let (compiled_transform, _) = compile_transform(transform);
            let err = match avenger_chart_core::apply_compiled_data_transforms(
                dataframe.clone(),
                &[compiled_transform],
                &DataTransformExecutionContext {
                    session_context: &ctx,
                    params: &IndexMap::new(),
                    time_context: TimeContext::default(),
                    facet_context: None,
                },
            )
            .await
            {
                Ok(_) => panic!("invalid extent should fail"),
                Err(err) => err,
            };
            assert!(err.to_string().contains(expected), "{err}");
        }
    }

    #[tokio::test]
    async fn time_fill_rejects_inconsistent_or_invalid_calendar_components() {
        let ctx = SessionContext::new();
        let dataframe = period_quarter_month_dataframe(&ctx, &[(2024, 1, 2, 20.0)]);
        let (_stage, quarter_month) = compile_transform(
            TimeLevels::new(col("timestamp"))
                .year()
                .quarter()
                .month()
                .name("period"),
        );
        let inconsistent = TimeFill::new(col("total"))
            .levels(quarter_month.levels())
            .extent(
                [lit(2024_i32), lit(2_i32), lit(2_i32)],
                [lit(2024_i32), lit(2_i32), lit(2_i32)],
            )
            .fill_value(lit(0.0));
        let (compiled_transform, _) = compile_transform(inconsistent);
        let err = match avenger_chart_core::apply_compiled_data_transforms(
            dataframe.clone(),
            &[compiled_transform],
            &DataTransformExecutionContext {
                session_context: &ctx,
                params: &IndexMap::new(),
                time_context: TimeContext::default(),
                facet_context: None,
            },
        )
        .await
        {
            Ok(_) => panic!("inconsistent quarter/month should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("inconsistent"), "{err}");

        let observed_inconsistent = TimeFill::new(col("total"))
            .levels(quarter_month.levels())
            .fill_value(lit(0.0));
        let (compiled_transform, _) = compile_transform(observed_inconsistent);
        let err = match avenger_chart_core::apply_compiled_data_transforms(
            period_quarter_month_dataframe(&ctx, &[(2024, 2, 2, 20.0)]),
            &[compiled_transform],
            &DataTransformExecutionContext {
                session_context: &ctx,
                params: &IndexMap::new(),
                time_context: TimeContext::default(),
                facet_context: None,
            },
        )
        .await
        {
            Ok(_) => panic!("observed inconsistent quarter/month should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("inconsistent"), "{err}");

        let day_dataframe = period_day_dataframe(&ctx, &[(2024, 2, 28, 1.0)]);
        let invalid_day = TimeFill::new(col("total"))
            .levels(period_year_month_day_levels())
            .extent(
                [lit(2024_i32), lit(2_i32), lit(30_i32)],
                [lit(2024_i32), lit(2_i32), lit(30_i32)],
            )
            .fill_value(lit(0.0));
        let (compiled_transform, _) = compile_transform(invalid_day);
        let err = match avenger_chart_core::apply_compiled_data_transforms(
            day_dataframe,
            &[compiled_transform],
            &DataTransformExecutionContext {
                session_context: &ctx,
                params: &IndexMap::new(),
                time_context: TimeContext::default(),
                facet_context: None,
            },
        )
        .await
        {
            Ok(_) => panic!("invalid day should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("invalid"));
    }

    #[tokio::test]
    async fn time_fill_leap_year_day_range_is_valid() {
        let ctx = SessionContext::new();
        let dataframe = period_day_dataframe(&ctx, &[(2024, 2, 28, 1.0)]);
        let (compiled_transform, _) = compile_transform(
            TimeFill::new(col("total"))
                .levels(period_year_month_day_levels())
                .extent(
                    [lit(2024_i32), lit(2_i32), lit(28_i32)],
                    [lit(2024_i32), lit(3_i32), lit(1_i32)],
                )
                .fill_value(lit(0.0)),
        );

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        assert_eq!(
            int32_values_from_batches(&batches, "period_day"),
            vec![Some(28), Some(29), Some(1)]
        );
        assert_eq!(
            float_values_from_batches(&batches, "total"),
            vec![Some(1.0), Some(0.0), Some(0.0)]
        );
    }

    #[test]
    fn time_fill_week_levels_return_unsupported_error_and_serializes() {
        let levels = TimeLevelKeys {
            levels: vec![TimeLevelKey {
                level: TimeLevel::Week,
                key_name: "period_week".to_string(),
                label: TimeLevelLabel::Key,
            }],
        };
        let err = match TimeFill::new(col("total"))
            .levels(levels)
            .fill_value(lit(0.0))
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free))
        {
            Ok(_) => panic!("week time fill should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("week-number"));

        let (stage, _output) = compile_transform(
            TimeFill::new(col("total"))
                .levels(period_year_month_levels())
                .fill_value(lit(0.0))
                .flag("was_time_filled"),
        );
        let bytes = bincode::serialize(&stage).expect("serialize time fill stage");
        let decoded: DataTransformStage = bincode::deserialize(&bytes).expect("deserialize stage");
        let json = serde_json::to_string(&decoded).expect("serialize decoded stage");
        assert!(json.contains("time_fill"));
        assert!(json.contains("was_time_filled"));
    }

    #[tokio::test]
    async fn time_levels_aggregate_groups_by_generated_keys() {
        let ctx = SessionContext::new();
        let dataframe = temporal_sales_dataframe(&ctx);
        let (period_transform, period) = compile_transform(
            TimeLevels::new(col("timestamp"))
                .year()
                .month()
                .name("period"),
        );
        let (aggregate_transform, _) = compile_transform(
            Aggregate::new()
                .group_by(period.keys_with([col("segment")]))
                .sum("total", col("value")),
        );

        let batches =
            transformed_batches(&ctx, dataframe, vec![period_transform, aggregate_transform]).await;
        assert_eq!(
            period_segment_totals_from_batches(&batches, "total"),
            vec![
                (2024, 1, "A".to_string(), 15.0),
                (2024, 1, "B".to_string(), 4.0),
                (2024, 3, "A".to_string(), 30.0),
                (2024, 3, "B".to_string(), 6.0),
            ]
        );
    }

    #[tokio::test]
    async fn time_levels_aggregate_time_fill_produces_complete_month_grid() {
        let ctx = SessionContext::new();
        let dataframe = temporal_sales_dataframe(&ctx);
        let (period_transform, period) = compile_transform(
            TimeLevels::new(col("timestamp"))
                .year()
                .month()
                .name("period"),
        );
        let (aggregate_transform, aggregate) = compile_transform(
            Aggregate::new()
                .group_by(period.keys_with([col("segment")]))
                .sum("total", col("value")),
        );
        let (fill_transform, _) = compile_transform(
            TimeFill::new(aggregate.output("total"))
                .levels(period.levels())
                .group_by([col("segment")])
                .fill_value(lit(0.0))
                .flag("was_time_filled"),
        );

        let batches = transformed_batches(
            &ctx,
            dataframe,
            vec![period_transform, aggregate_transform, fill_transform],
        )
        .await;
        assert_eq!(
            period_segment_fill_rows_from_batches(&batches),
            vec![
                (2024, 1, "A".to_string(), 15.0, false),
                (2024, 1, "B".to_string(), 4.0, false),
                (2024, 2, "A".to_string(), 0.0, true),
                (2024, 2, "B".to_string(), 0.0, true),
                (2024, 3, "A".to_string(), 30.0, false),
                (2024, 3, "B".to_string(), 6.0, false),
            ]
        );
    }

    #[tokio::test]
    async fn time_levels_time_fill_preserves_segment_groups_for_stack() {
        let ctx = SessionContext::new();
        let dataframe = temporal_sales_dataframe(&ctx);
        let (period_transform, period) = compile_transform(
            TimeLevels::new(col("timestamp"))
                .year()
                .month()
                .name("period"),
        );
        let (aggregate_transform, aggregate) = compile_transform(
            Aggregate::new()
                .group_by(period.keys_with([col("segment")]))
                .sum("total", col("value")),
        );
        let (fill_transform, _) = compile_transform(
            TimeFill::new(aggregate.output("total"))
                .levels(period.levels())
                .group_by([col("segment")])
                .fill_value(lit(0.0))
                .flag("was_time_filled"),
        );

        let batches = transformed_batches(
            &ctx,
            dataframe,
            vec![period_transform, aggregate_transform, fill_transform],
        )
        .await;
        let rows = period_segment_fill_rows_from_batches(&batches);
        let month_segments = rows
            .iter()
            .map(|row| (row.1, row.2.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            month_segments,
            vec![(1, "A"), (1, "B"), (2, "A"), (2, "B"), (3, "A"), (3, "B"),]
        );
    }

    #[tokio::test]
    async fn time_levels_aggregate_time_fill_stack_preserves_complete_leaf_paths() {
        let ctx = SessionContext::new();
        let dataframe = temporal_sales_dataframe(&ctx);
        let (period_transform, period) = compile_transform(
            TimeLevels::new(col("timestamp"))
                .year()
                .month()
                .name("period"),
        );
        let (aggregate_transform, aggregate) = compile_transform(
            Aggregate::new()
                .group_by(period.keys_with([col("segment")]))
                .sum("total", col("value")),
        );
        let (fill_transform, filled) = compile_transform(
            TimeFill::new(aggregate.output("total"))
                .levels(period.levels())
                .group_by([col("segment")])
                .fill_value(lit(0.0)),
        );
        let (stack_transform, _) = compile_transform(
            Stack::new(filled.value())
                .group_by(period.keys())
                .sort_by_exprs([col("segment")])
                .name("total_stack"),
        );

        let batches = transformed_batches(
            &ctx,
            dataframe,
            vec![
                period_transform,
                aggregate_transform,
                fill_transform,
                stack_transform,
            ],
        )
        .await;
        let rows = period_segment_stack_rows_from_batches(&batches);
        assert_eq!(
            rows.iter()
                .map(|row| (row.0, row.1, row.2.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (2024, 1, "A"),
                (2024, 1, "B"),
                (2024, 2, "A"),
                (2024, 2, "B"),
                (2024, 3, "A"),
                (2024, 3, "B"),
            ]
        );

        for (year, month, _segment, total, start, end) in &rows {
            if *month == 2 {
                assert_eq!((*year, *total), (2024, 0.0));
                assert_close(*start, *end);
            }
        }
        let mut month_totals = rows.iter().map(|row| (row.1, (row.5 - row.4).abs())).fold(
            indexmap::IndexMap::<i32, f64>::new(),
            |mut totals, row| {
                *totals.entry(row.0).or_insert(0.0) += row.1;
                totals
            },
        );
        assert_close(month_totals.swap_remove(&1).unwrap(), 19.0);
        assert_close(month_totals.swap_remove(&2).unwrap(), 0.0);
        assert_close(month_totals.swap_remove(&3).unwrap(), 36.0);
    }

    #[tokio::test]
    async fn bin_output_names_and_derived_scalar_refs() {
        let output = Bin::new(col("value"))
            .maxbins(4)
            .name("custom_bin")
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free))
            .unwrap()
            .1;

        assert_eq!(output.index().to_string(), "custom_bin_index");
        let start = output.start();
        assert!(matches!(
            start.channel_value(),
            &ChannelValue::Scaled { .. }
        ));
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
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Level(
                1,
            )))
            .unwrap()
            .1;

        let start = output.start();
        let end = output.end();
        assert_eq!(start.get_domain_scope(), Some(CoordinationScope::Level(1)));
        assert_eq!(
            start.get_transform_scope(),
            Some(CoordinationScope::Level(1))
        );
        assert_eq!(end.get_domain_scope(), Some(CoordinationScope::Level(1)));
        assert_eq!(end.get_transform_scope(), Some(CoordinationScope::Level(1)));
    }

    #[tokio::test]
    async fn lump_value_output_carries_default_ordering_and_scope() {
        let output = Lump::top_n(col("category"), 3)
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Level(
                1,
            )))
            .unwrap()
            .1;
        let value = output.value();
        assert_eq!(value.get_domain_scope(), Some(CoordinationScope::Level(1)));
        assert_eq!(
            value.get_transform_scope(),
            Some(CoordinationScope::Level(1))
        );
        let scale = value.get_scale_config().expect("scale config");
        let ordering = scale.ordering.as_option().expect("ordering");
        assert!(ordering.has_order_expr());
        assert!(!ordering.order_descending());
    }

    #[tokio::test]
    async fn lump_default_string_other_groups_rows() {
        let ctx = SessionContext::new();
        let dataframe = lump_dataframe(
            &ctx,
            vec![
                Some("Alpha"),
                Some("Alpha"),
                Some("Beta"),
                Some("Gamma"),
                Some("Delta"),
            ],
            vec![50.0, 40.0, 30.0, 20.0, 10.0],
        );
        let (compiled_transform, _) = compile_transform(
            Lump::top_n(col("category"), 2)
                .order_by(sum(col("value")))
                .name("category_lump"),
        );

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let mut rows = lump_rows_from_batches(&batches);
        rows.sort_by(|a, b| a.1.total_cmp(&b.1));
        assert_eq!(
            rows,
            vec![
                (
                    Some("Delta".to_string()),
                    10.0,
                    Some("Other".to_string()),
                    None,
                    true
                ),
                (
                    Some("Gamma".to_string()),
                    20.0,
                    Some("Other".to_string()),
                    None,
                    true
                ),
                (
                    Some("Beta".to_string()),
                    30.0,
                    Some("Beta".to_string()),
                    Some(2.0),
                    false
                ),
                (
                    Some("Alpha".to_string()),
                    40.0,
                    Some("Alpha".to_string()),
                    Some(1.0),
                    false
                ),
                (
                    Some("Alpha".to_string()),
                    50.0,
                    Some("Alpha".to_string()),
                    Some(1.0),
                    false
                ),
            ]
        );
    }

    #[tokio::test]
    async fn lump_top_n_accepts_param_expr() {
        let ctx = SessionContext::new();
        let dataframe = lump_dataframe(
            &ctx,
            vec![
                Some("Alpha"),
                Some("Alpha"),
                Some("Beta"),
                Some("Gamma"),
                Some("Delta"),
            ],
            vec![50.0, 40.0, 30.0, 20.0, 10.0],
        );
        let top_n = Param::new("lump_top_n", ScalarValue::Int64(Some(3)));
        let params = IndexMap::from([(top_n.name.clone(), top_n.default.clone())]);
        let (compiled_transform, _) = compile_transform(
            Lump::top_n(col("category"), top_n.expr())
                .order_by(sum(col("value")))
                .name("category_lump"),
        );

        let batches =
            transformed_batches_with_params(&ctx, dataframe, vec![compiled_transform], &params)
                .await;
        let rows = lump_rows_from_batches(&batches);
        let retained = rows
            .iter()
            .filter(|row| !row.4)
            .map(|row| row.0.clone().unwrap())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            retained,
            ["Alpha", "Beta", "Gamma"]
                .into_iter()
                .map(String::from)
                .collect()
        );
    }

    #[tokio::test]
    async fn lump_top_n_zero_errors_at_runtime() {
        let ctx = SessionContext::new();
        let dataframe = lump_dataframe(&ctx, vec![Some("Alpha"), Some("Beta")], vec![50.0, 40.0]);
        let (compiled_transform, _) = compile_transform(
            Lump::top_n(col("category"), 0)
                .order_by(sum(col("value")))
                .name("category_lump"),
        );

        let err = match avenger_chart_core::apply_compiled_data_transforms(
            dataframe,
            &[compiled_transform],
            &DataTransformExecutionContext {
                session_context: &ctx,
                params: &IndexMap::new(),
                time_context: TimeContext::default(),
                facet_context: None,
            },
        )
        .await
        {
            Ok(_) => panic!("top_n zero should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("positive integer"), "{err}");
    }

    #[tokio::test]
    async fn lump_top_n_float_errors_at_runtime() {
        let ctx = SessionContext::new();
        let dataframe = lump_dataframe(&ctx, vec![Some("Alpha"), Some("Beta")], vec![50.0, 40.0]);
        let (compiled_transform, _) = compile_transform(
            Lump::top_n(col("category"), lit(2.5))
                .order_by(sum(col("value")))
                .name("category_lump"),
        );

        let err = match avenger_chart_core::apply_compiled_data_transforms(
            dataframe,
            &[compiled_transform],
            &DataTransformExecutionContext {
                session_context: &ctx,
                params: &IndexMap::new(),
                time_context: TimeContext::default(),
                facet_context: None,
            },
        )
        .await
        {
            Ok(_) => panic!("top_n float should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("integer scalar"), "{err}");
    }

    #[tokio::test]
    async fn lump_null_category_is_retained_when_it_passes_predicate() {
        let ctx = SessionContext::new();
        let dataframe = lump_dataframe(
            &ctx,
            vec![None, None, Some("Alpha"), Some("Beta")],
            vec![100.0, 1.0, 50.0, 40.0],
        );
        let (compiled_transform, _) = compile_transform(
            Lump::top_n(col("category"), 1)
                .order_by(sum(col("value")))
                .name("category_lump"),
        );

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let mut rows = lump_rows_from_batches(&batches);
        rows.sort_by(|a, b| a.1.total_cmp(&b.1));
        assert_eq!(
            rows,
            vec![
                (None, 1.0, None, Some(1.0), false),
                (
                    Some("Beta".to_string()),
                    40.0,
                    Some("Other".to_string()),
                    None,
                    true
                ),
                (
                    Some("Alpha".to_string()),
                    50.0,
                    Some("Other".to_string()),
                    None,
                    true
                ),
                (None, 100.0, None, Some(1.0), false),
            ]
        );
    }

    #[tokio::test]
    async fn lump_null_category_is_lumped_when_it_fails_predicate() {
        let ctx = SessionContext::new();
        let dataframe = lump_dataframe(
            &ctx,
            vec![None, None, Some("Alpha"), Some("Beta")],
            vec![1.0, 1.0, 100.0, 40.0],
        );
        let (compiled_transform, _) = compile_transform(
            Lump::top_n(col("category"), 1)
                .order_by(sum(col("value")))
                .name("category_lump"),
        );

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let mut rows = lump_rows_from_batches(&batches);
        rows.sort_by(|a, b| a.1.total_cmp(&b.1));
        assert_eq!(
            rows,
            vec![
                (None, 1.0, Some("Other".to_string()), None, true),
                (None, 1.0, Some("Other".to_string()), None, true),
                (
                    Some("Beta".to_string()),
                    40.0,
                    Some("Other".to_string()),
                    None,
                    true
                ),
                (
                    Some("Alpha".to_string()),
                    100.0,
                    Some("Alpha".to_string()),
                    Some(1.0),
                    false
                ),
            ]
        );
    }

    #[tokio::test]
    async fn lump_drop_other_filters_rows() {
        let ctx = SessionContext::new();
        let dataframe = lump_dataframe(
            &ctx,
            vec![
                Some("Alpha"),
                Some("Alpha"),
                Some("Beta"),
                Some("Gamma"),
                Some("Delta"),
            ],
            vec![50.0, 40.0, 30.0, 20.0, 10.0],
        );
        let (compiled_transform, _) = compile_transform(
            Lump::top_n(col("category"), 2)
                .order_by(sum(col("value")))
                .drop_other()
                .name("category_lump"),
        );

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let mut rows = lump_rows_from_batches(&batches);
        rows.sort_by(|a, b| a.1.total_cmp(&b.1));
        assert_eq!(
            rows,
            vec![
                (
                    Some("Beta".to_string()),
                    30.0,
                    Some("Beta".to_string()),
                    Some(2.0),
                    false
                ),
                (
                    Some("Alpha".to_string()),
                    40.0,
                    Some("Alpha".to_string()),
                    Some(1.0),
                    false
                ),
                (
                    Some("Alpha".to_string()),
                    50.0,
                    Some("Alpha".to_string()),
                    Some(1.0),
                    false
                ),
            ]
        );
    }

    #[tokio::test]
    async fn lump_rank_keeps_ties() {
        let ctx = SessionContext::new();
        let dataframe = lump_dataframe(
            &ctx,
            vec![
                Some("Alpha"),
                Some("Beta"),
                Some("Gamma"),
                Some("Delta"),
                Some("Epsilon"),
            ],
            vec![90.0, 80.0, 60.0, 60.0, 10.0],
        );
        let (compiled_transform, _) = compile_transform(
            Lump::top_n(col("category"), 3)
                .order_by(sum(col("value")))
                .window(rank())
                .keep(lump::window_value().lt_eq(lit(3)))
                .name("category_lump"),
        );

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let rows = lump_rows_from_batches(&batches);
        let retained = rows
            .iter()
            .filter(|row| !row.4)
            .map(|row| row.0.clone().unwrap())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            retained,
            ["Alpha", "Beta", "Delta", "Gamma"]
                .into_iter()
                .map(String::from)
                .collect()
        );
    }

    #[tokio::test]
    async fn lump_percent_rank_predicate() {
        let ctx = SessionContext::new();
        let dataframe = lump_dataframe(
            &ctx,
            vec![
                Some("Alpha"),
                Some("Beta"),
                Some("Gamma"),
                Some("Delta"),
                Some("Epsilon"),
            ],
            vec![90.0, 80.0, 70.0, 60.0, 10.0],
        );
        let (compiled_transform, _) = compile_transform(
            Lump::top_n(col("category"), 5)
                .order_by(sum(col("value")))
                .window(percent_rank())
                .keep(lump::window_value().lt_eq(lit(0.5)))
                .name("category_lump"),
        );

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let rows = lump_rows_from_batches(&batches);
        let retained = rows
            .iter()
            .filter(|row| !row.4)
            .map(|row| row.0.clone().unwrap())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            retained,
            ["Alpha", "Beta", "Gamma"]
                .into_iter()
                .map(String::from)
                .collect()
        );
    }

    #[tokio::test]
    async fn lump_ntile_predicate() {
        let ctx = SessionContext::new();
        let dataframe = lump_dataframe(
            &ctx,
            vec![
                Some("Alpha"),
                Some("Beta"),
                Some("Gamma"),
                Some("Delta"),
                Some("Epsilon"),
                Some("Zeta"),
                Some("Eta"),
                Some("Theta"),
            ],
            vec![80.0, 70.0, 60.0, 50.0, 40.0, 30.0, 20.0, 10.0],
        );
        let (compiled_transform, _) = compile_transform(
            Lump::top_n(col("category"), 8)
                .order_by(sum(col("value")))
                .window(ntile(lit(4)))
                .keep(lump::window_value().lt_eq(lit(2)))
                .name("category_lump"),
        );

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let rows = lump_rows_from_batches(&batches);
        let retained = rows
            .iter()
            .filter(|row| !row.4)
            .map(|row| row.0.clone().unwrap())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            retained,
            ["Alpha", "Beta", "Delta", "Gamma"]
                .into_iter()
                .map(String::from)
                .collect()
        );
    }

    #[tokio::test]
    async fn lump_non_string_without_other_value_errors() {
        let ctx = SessionContext::new();
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("category_id", DataType::Int64, false),
                Field::new("value", DataType::Float64, false),
            ])),
            vec![
                Arc::new(Int64Array::from(vec![1, 2, 3])) as _,
                Arc::new(Float64Array::from(vec![10.0, 5.0, 1.0])) as _,
            ],
        )
        .unwrap();
        let dataframe = ctx.read_batch(batch).unwrap();
        let (compiled_transform, _) = compile_transform(
            Lump::top_n(col("category_id"), 1)
                .order_by(sum(col("value")))
                .name("category_lump"),
        );

        let err = match avenger_chart_core::apply_compiled_data_transforms(
            dataframe,
            &[compiled_transform],
            &DataTransformExecutionContext {
                session_context: &ctx,
                params: &IndexMap::new(),
                time_context: TimeContext::default(),
                facet_context: None,
            },
        )
        .await
        {
            Ok(_) => panic!("non-string default Other should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("other_value"), "{err}");
    }

    #[tokio::test]
    async fn bin_maxbins_zero_errors() {
        let err = match Bin::new(col("value"))
            .maxbins(0)
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free))
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
                params: &IndexMap::new(),
                time_context: TimeContext::default(),
                facet_context: None,
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
    async fn channel_expr_transform_output_feeds_later_transform_expression() {
        let ctx = SessionContext::new();
        let dataframe = bin_dataframe(&ctx);
        let (bin_transform, bin) = compile_transform(Bin::new(col("value")).maxbins(2));
        let (calculate_transform, _) =
            compile_transform(Calculate::new().expr("start_copy", bin.start()));

        let batches =
            transformed_batches(&ctx, dataframe, vec![bin_transform, calculate_transform]).await;
        assert_eq!(batches.len(), 1);
        let batch = &batches[0];
        let starts = batch
            .column_by_name("value_bin_start")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        let copies = batch
            .column_by_name("start_copy")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        for index in 0..batch.num_rows() {
            assert_eq!(starts.is_null(index), copies.is_null(index));
            if !starts.is_null(index) {
                assert_eq!(starts.value(index), copies.value(index));
            }
        }
    }

    #[tokio::test]
    async fn channel_expr_transform_output_can_be_later_transform_input() {
        let ctx = SessionContext::new();
        let dataframe = bin_dataframe(&ctx);
        let (bin_transform, bin) = compile_transform(Bin::new(col("value")).maxbins(2));
        let (kde_transform, _) = compile_transform(Kde::new(bin.start()).bandwidth(1.0).steps(4));

        let batches =
            transformed_batches(&ctx, dataframe, vec![bin_transform, kde_transform]).await;
        let row_count: usize = batches.iter().map(RecordBatch::num_rows).sum();
        assert_eq!(row_count, 5);
    }

    #[tokio::test]
    async fn kde_default_and_custom_output_names() {
        let ctx = SessionContext::new();
        let dataframe = bin_dataframe_from_values(&ctx, vec![Some(0.0), Some(1.0), Some(2.0)]);
        let (compiled_transform, output) =
            compile_transform(Kde::new(col("value")).bandwidth(1.0).steps(3));
        assert_eq!(output.value().to_string(), "value");
        assert_eq!(output.density().to_string(), "density");

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        assert_eq!(batches[0].num_rows(), 4);
        assert!(batches[0].column_by_name("value").is_some());
        assert!(batches[0].column_by_name("density").is_some());

        let dataframe = bin_dataframe_from_values(&ctx, vec![Some(0.0), Some(1.0), Some(2.0)]);
        let (compiled_transform, output) = compile_transform(
            Kde::new(col("value"))
                .bandwidth(1.0)
                .steps(2)
                .as_fields("sample", "estimate"),
        );
        assert_eq!(output.value().to_string(), "sample");
        assert_eq!(output.density().to_string(), "estimate");
        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        assert_eq!(batches[0].num_rows(), 3);
        assert!(batches[0].column_by_name("sample").is_some());
        assert!(batches[0].column_by_name("estimate").is_some());
    }

    #[tokio::test]
    async fn kde_fixed_bandwidth_steps_and_grouped_grids() {
        let ctx = SessionContext::new();
        let dataframe = kde_dataframe(&ctx);
        let (compiled_transform, _) = compile_transform(
            Kde::new(col("value"))
                .group_by([col("series")])
                .bandwidth(1.0)
                .steps(2),
        );

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let rows = kde_group_rows_from_batches(&batches, "value");
        assert_eq!(rows.len(), 6);
        assert_eq!(
            rows.iter().map(|row| (&row.0, row.1)).collect::<Vec<_>>(),
            vec![
                (&"A".to_string(), 0.0),
                (&"A".to_string(), 1.0),
                (&"A".to_string(), 2.0),
                (&"B".to_string(), 10.0),
                (&"B".to_string(), 11.0),
                (&"B".to_string(), 12.0),
            ]
        );

        let dataframe = kde_dataframe(&ctx);
        let (compiled_transform, _) = compile_transform(
            Kde::new(col("value"))
                .group_by([col("series")])
                .bandwidth(1.0)
                .steps(2)
                .resolve(KdeResolve::Shared),
        );
        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let rows = kde_group_rows_from_batches(&batches, "value");
        assert_eq!(
            rows.iter().map(|row| (&row.0, row.1)).collect::<Vec<_>>(),
            vec![
                (&"A".to_string(), 0.0),
                (&"A".to_string(), 6.0),
                (&"A".to_string(), 12.0),
                (&"B".to_string(), 0.0),
                (&"B".to_string(), 6.0),
                (&"B".to_string(), 12.0),
            ]
        );
    }

    #[tokio::test]
    async fn kde_adds_facet_partition_context_to_grouping() {
        let ctx = SessionContext::new();
        let dataframe = sample_dataframe(&ctx);
        let (compiled_transform, _) = compile_transform(
            Kde::new(col("value"))
                .group_by([col("series")])
                .bandwidth(1.0)
                .steps(1),
        );
        let result = avenger_chart_core::apply_compiled_data_transforms(
            dataframe,
            &[compiled_transform],
            &DataTransformExecutionContext {
                session_context: &ctx,
                params: &IndexMap::new(),
                time_context: TimeContext::default(),
                facet_context: Some(DataTransformFacetContext {
                    transform_level: SharingLevel::GLOBAL,
                    final_mark_level: SharingLevel::FREE,
                    partition_exprs: vec![col("category")],
                }),
            },
        )
        .await
        .expect("apply kde");
        let batches = result.dataframe.collect().await.expect("collect kde");
        assert_eq!(batches.len(), 1);
        let batch = &batches[0];
        assert_eq!(batch.schema().field(0).name(), "series");
        assert_eq!(batch.schema().field(1).name(), "category");

        let series = batch
            .column_by_name("series")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let category = batch
            .column_by_name("category")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let keys = (0..batch.num_rows())
            .map(|row| {
                (
                    category.value(row).to_string(),
                    series.value(row).to_string(),
                )
            })
            .collect::<HashSet<_>>();
        assert_eq!(
            keys,
            HashSet::from([
                ("A".to_string(), "s1".to_string()),
                ("A".to_string(), "s2".to_string()),
                ("A".to_string(), "s3".to_string()),
                ("B".to_string(), "s1".to_string()),
            ])
        );
    }

    #[tokio::test]
    async fn kde_counts_scales_density_by_group_count() {
        let ctx = SessionContext::new();
        let dataframe = bin_dataframe_from_values(&ctx, vec![Some(0.0), Some(1.0), Some(2.0)]);
        let (compiled_transform, _) = compile_transform(
            Kde::new(col("value"))
                .bandwidth(1.0)
                .steps(2)
                .extent(0.0, 2.0),
        );
        let density = kde_rows_from_batches(
            &transformed_batches(&ctx, dataframe, vec![compiled_transform]).await,
            "value",
            "density",
        );

        let dataframe = bin_dataframe_from_values(&ctx, vec![Some(0.0), Some(1.0), Some(2.0)]);
        let (compiled_transform, _) = compile_transform(
            Kde::new(col("value"))
                .bandwidth(1.0)
                .steps(2)
                .extent(0.0, 2.0)
                .counts(true),
        );
        let counts = kde_rows_from_batches(
            &transformed_batches(&ctx, dataframe, vec![compiled_transform]).await,
            "value",
            "density",
        );
        for (density, counts) in density.iter().zip(counts.iter()) {
            assert!((counts.1 - density.1 * 3.0).abs() < 1e-12);
        }
    }

    #[tokio::test]
    async fn kde_cumulative_is_monotonic_and_ends_near_one() {
        let ctx = SessionContext::new();
        let dataframe = bin_dataframe_from_values(&ctx, vec![Some(0.0), Some(1.0), Some(2.0)]);
        let (compiled_transform, _) = compile_transform(
            Kde::new(col("value"))
                .bandwidth(0.35)
                .steps(40)
                .extent(-5.0, 7.0)
                .cumulative(true),
        );

        let rows = kde_rows_from_batches(
            &transformed_batches(&ctx, dataframe, vec![compiled_transform]).await,
            "value",
            "density",
        );
        for window in rows.windows(2) {
            assert!(window[1].1 >= window[0].1, "{rows:?}");
        }
        assert!(rows.last().unwrap().1 > 0.999, "{rows:?}");
    }

    #[tokio::test]
    async fn kde_auto_bandwidth_is_positive_for_edge_cases() {
        assert!(super::kde::auto_bandwidth(&[0.0, 1.0, 2.0]).is_sign_positive());
        assert!(super::kde::auto_bandwidth(&[7.0, 7.0, 7.0]).is_sign_positive());
        assert!(super::kde::auto_bandwidth(&[7.0]).is_sign_positive());
        assert!(super::kde::auto_bandwidth(&[]).is_sign_positive());
    }

    #[tokio::test]
    async fn kde_params_work_for_config_expressions() {
        let ctx = SessionContext::new();
        let dataframe = bin_dataframe_from_values(&ctx, vec![Some(0.0), Some(1.0), Some(2.0)]);
        let steps = Param::new("kde_steps", ScalarValue::Int64(Some(3)));
        let bandwidth = Param::new("kde_bandwidth", ScalarValue::Float64(Some(0.5)));
        let params = IndexMap::from([
            (steps.name.clone(), steps.default.clone()),
            (bandwidth.name.clone(), bandwidth.default.clone()),
        ]);
        let (compiled_transform, _) = compile_transform(
            Kde::new(col("value"))
                .steps(steps.expr())
                .bandwidth(bandwidth.expr()),
        );

        let batches =
            transformed_batches_with_params(&ctx, dataframe, vec![compiled_transform], &params)
                .await;
        assert_eq!(batches[0].num_rows(), 4);
    }

    #[tokio::test]
    async fn kde_config_rejects_column_refs() {
        let ctx = SessionContext::new();
        let dataframe = bin_dataframe_from_values(&ctx, vec![Some(0.0), Some(1.0), Some(2.0)]);
        let (compiled_transform, _) =
            compile_transform(Kde::new(col("value")).steps(col("value")).bandwidth(1.0));
        let err = match avenger_chart_core::apply_compiled_data_transforms(
            dataframe,
            &[compiled_transform],
            &DataTransformExecutionContext {
                session_context: &ctx,
                params: &IndexMap::new(),
                time_context: TimeContext::default(),
                facet_context: None,
            },
        )
        .await
        {
            Ok(_) => panic!("column ref config should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("column references"), "{err}");
    }

    #[tokio::test]
    async fn kde_skips_null_and_non_finite_values() {
        let ctx = SessionContext::new();
        let dataframe = kde_dataframe_from_values(
            &ctx,
            vec![Some("A"), Some("A"), Some("A"), Some("A")],
            vec![Some(0.0), None, Some(f64::NAN), Some(f64::INFINITY)],
        );
        let (compiled_transform, _) = compile_transform(
            Kde::new(col("value"))
                .group_by([col("series")])
                .bandwidth(1.0)
                .steps(2),
        );
        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        assert_eq!(batches[0].num_rows(), 3);

        let dataframe = kde_dataframe_from_values(
            &ctx,
            vec![Some("A"), Some("A"), Some("A")],
            vec![None, Some(f64::NAN), Some(f64::INFINITY)],
        );
        let (compiled_transform, _) =
            compile_transform(Kde::new(col("value")).bandwidth(1.0).steps(2));
        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        assert_eq!(batches[0].num_rows(), 0);
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
        assert!(matches!(
            output.start().into_channel_value(),
            ChannelValue::Scaled { .. }
        ));
        let result = avenger_chart_core::apply_compiled_data_transforms(
            dataframe,
            &[compiled_transform],
            &DataTransformExecutionContext {
                session_context: &ctx,
                params: &IndexMap::new(),
                time_context: TimeContext::default(),
                facet_context: None,
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

    #[tokio::test]
    async fn rasterize_2d_count_bins_rows_and_closes_final_bin() {
        let ctx = SessionContext::new();
        let dataframe = rasterize_dataframe(
            &ctx,
            vec![None; 10],
            vec![
                Some(0.0),
                Some(0.49),
                Some(1.0),
                Some(2.0),
                Some(2.0),
                Some(-1.0),
                Some(f64::INFINITY),
                Some(f64::NAN),
                None,
                Some(0.0),
            ],
            vec![
                Some(0.0),
                Some(0.49),
                Some(1.0),
                Some(2.0),
                Some(0.0),
                Some(0.0),
                Some(1.0),
                Some(1.0),
                Some(0.0),
                None,
            ],
            vec![Some(1.0); 10],
        );
        let (transform, _) = compile_transform(
            Rasterize2D::new(col("x"), col("y"))
                .x(|x| x.extent(0.0, 2.0).bins(2))
                .y(|y| y.extent(0.0, 2.0).bins(2))
                .agg("count"),
        );

        let batches = transformed_batches(&ctx, dataframe, vec![transform]).await;
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].num_rows(), 1);
        assert_eq!(raster_count_values(&batches[0], 0), vec![2, 1, 0, 2]);
        assert_eq!(
            raster_uniform_dimension(&batches[0], 0, 0),
            ("x".to_string(), "linear".to_string(), 0.0, 2.0, 2)
        );
        assert_eq!(
            raster_uniform_dimension(&batches[0], 0, 1),
            ("y".to_string(), "linear".to_string(), 0.0, 2.0, 2)
        );
    }

    #[tokio::test]
    async fn rasterize_2d_count_value_skips_null_and_non_finite_values() {
        let ctx = SessionContext::new();
        let dataframe = rasterize_dataframe(
            &ctx,
            vec![None; 5],
            vec![Some(0.0), Some(0.49), Some(1.0), Some(2.0), Some(2.0)],
            vec![Some(0.0), Some(0.49), Some(1.0), Some(2.0), Some(0.0)],
            vec![Some(1.0), None, Some(f64::NAN), Some(4.0), Some(5.0)],
        );
        let (transform, _) = compile_transform(
            Rasterize2D::new(col("x"), col("y"))
                .x(|x| x.extent(0.0, 2.0).bins(2))
                .y(|y| y.extent(0.0, 2.0).bins(2))
                .value(col("value"))
                .agg("count"),
        );

        let batches = transformed_batches(&ctx, dataframe, vec![transform]).await;
        assert_eq!(raster_count_values(&batches[0], 0), vec![1, 1, 0, 1]);
    }

    #[tokio::test]
    async fn rasterize_2d_executor_count_matches_sync_transform() {
        let ctx = SessionContext::new();
        let dataframe = rasterize_dataframe(
            &ctx,
            vec![None; 10],
            vec![
                Some(0.0),
                Some(0.49),
                Some(1.0),
                Some(2.0),
                Some(2.0),
                Some(-1.0),
                Some(f64::INFINITY),
                Some(f64::NAN),
                None,
                Some(0.0),
            ],
            vec![
                Some(0.0),
                Some(0.49),
                Some(1.0),
                Some(2.0),
                Some(0.0),
                Some(0.0),
                Some(1.0),
                Some(1.0),
                Some(0.0),
                None,
            ],
            vec![Some(1.0); 10],
        );
        let batch = rasterize_executor_batch(
            &ctx,
            dataframe,
            Rasterize2D::new(col("x"), col("y"))
                .x(|x| x.extent(0.0, 2.0).bins(2))
                .y(|y| y.extent(0.0, 2.0).bins(2))
                .agg("count"),
            IndexMap::new(),
        )
        .await;

        assert_eq!(batch.num_rows(), 1);
        assert_eq!(raster_count_values(&batch, 0), vec![2, 1, 0, 2]);
    }

    #[tokio::test]
    async fn rasterize_2d_executor_uses_embedded_view_params() {
        let ctx = SessionContext::new();
        let dataframe = rasterize_dataframe(
            &ctx,
            vec![None; 5],
            vec![Some(0.0), Some(0.49), Some(1.0), Some(2.0), Some(2.0)],
            vec![Some(0.0), Some(0.49), Some(1.0), Some(2.0), Some(0.0)],
            vec![Some(1.0); 5],
        );
        let params = IndexMap::from([
            ("x0".to_string(), ScalarValue::Float64(Some(0.0))),
            ("x1".to_string(), ScalarValue::Float64(Some(2.0))),
            ("xbins".to_string(), ScalarValue::UInt32(Some(2))),
            ("y0".to_string(), ScalarValue::Float64(Some(0.0))),
            ("y1".to_string(), ScalarValue::Float64(Some(2.0))),
            ("ybins".to_string(), ScalarValue::UInt32(Some(2))),
        ]);
        let batch = rasterize_executor_batch(
            &ctx,
            dataframe,
            Rasterize2D::new(col("x"), col("y"))
                .x(|x| {
                    x.extent(
                        Param::new("x0", ScalarValue::Float64(Some(-1.0))).expr(),
                        Param::new("x1", ScalarValue::Float64(Some(1.0))).expr(),
                    )
                    .bins(Param::new("xbins", ScalarValue::UInt32(Some(1))).expr())
                })
                .y(|y| {
                    y.extent(
                        Param::new("y0", ScalarValue::Float64(Some(-1.0))).expr(),
                        Param::new("y1", ScalarValue::Float64(Some(1.0))).expr(),
                    )
                    .bins(Param::new("ybins", ScalarValue::UInt32(Some(1))).expr())
                })
                .agg("count"),
            params,
        )
        .await;

        assert_eq!(raster_count_values(&batch, 0), vec![2, 1, 0, 2]);
    }

    #[tokio::test]
    async fn rasterize_2d_infers_extents_with_datafusion_prepass() {
        let ctx = SessionContext::new();
        let dataframe = rasterize_dataframe(
            &ctx,
            vec![None; 5],
            vec![
                Some(0.0),
                Some(1.0),
                Some(2.0),
                Some(f64::NAN),
                Some(f64::INFINITY),
            ],
            vec![Some(10.0), Some(15.0), Some(20.0), Some(12.0), Some(18.0)],
            vec![Some(1.0); 5],
        );
        let (transform, _) = compile_transform(
            Rasterize2D::new(col("x"), col("y"))
                .x(|x| x.bins(2))
                .y(|y| y.bins(2))
                .agg("count"),
        );

        let batches = transformed_batches(&ctx, dataframe, vec![transform]).await;
        assert_eq!(
            raster_uniform_dimension(&batches[0], 0, 0),
            ("x".to_string(), "linear".to_string(), 0.0, 2.0, 2)
        );
        assert_eq!(
            raster_uniform_dimension(&batches[0], 0, 1),
            ("y".to_string(), "linear".to_string(), 10.0, 20.0, 2)
        );
        assert_eq!(raster_count_values(&batches[0], 0), vec![1, 0, 0, 2]);
    }

    /// Read a categorical dimension (name + values) from a raster row.
    fn raster_categorical_dimension(
        batch: &RecordBatch,
        row: usize,
        dimension: usize,
    ) -> (String, Vec<String>) {
        let geometry = raster_struct(batch)
            .column_by_name("geometry")
            .unwrap()
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();
        let dimensions = geometry
            .column_by_name("dimensions")
            .unwrap()
            .as_any()
            .downcast_ref::<ListArray>()
            .unwrap();
        let row_dims = dimensions.value(row);
        let row_dims = row_dims.as_any().downcast_ref::<StructArray>().unwrap();
        let names = row_dims
            .column_by_name("name")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let coords = row_dims
            .column_by_name("coords")
            .unwrap()
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();
        let kinds = coords
            .column_by_name("kind")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(kinds.value(dimension), "categorical");
        let values = coords
            .column_by_name("values")
            .unwrap()
            .as_any()
            .downcast_ref::<ListArray>()
            .unwrap();
        let dim_values = values.value(dimension);
        let dim_values = dim_values.as_any().downcast_ref::<StringArray>().unwrap();
        (
            names.value(dimension).to_string(),
            (0..dim_values.len())
                .map(|index| dim_values.value(index).to_string())
                .collect(),
        )
    }

    fn raster_values_dims(batch: &RecordBatch, row: usize) -> Vec<String> {
        let values = raster_struct(batch)
            .column_by_name("values")
            .unwrap()
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();
        let dims = values
            .column_by_name("dims")
            .unwrap()
            .as_any()
            .downcast_ref::<ListArray>()
            .unwrap();
        let row_dims = dims.value(row);
        let row_dims = row_dims.as_any().downcast_ref::<StringArray>().unwrap();
        (0..row_dims.len())
            .map(|index| row_dims.value(index).to_string())
            .collect()
    }

    #[tokio::test]
    async fn rasterize_2d_by_produces_sorted_planes() {
        let ctx = SessionContext::new();
        // Categories arrive in shuffled order; planes must emit sorted.
        let dataframe = rasterize_dataframe(
            &ctx,
            vec![Some("b"), Some("a"), Some("b"), Some("a"), Some("c")],
            vec![Some(0.0), Some(2.0), Some(0.0), Some(0.0), Some(2.0)],
            vec![Some(0.0), Some(2.0), Some(0.0), Some(0.0), Some(0.0)],
            vec![Some(1.0); 5],
        );
        let (transform, _) = compile_transform(
            Rasterize2D::new(col("x"), col("y"))
                .x(|x| x.extent(0.0, 2.0).bins(2))
                .y(|y| y.extent(0.0, 2.0).bins(2))
                .by(col("group"))
                .agg("count"),
        );

        let batches = transformed_batches(&ctx, dataframe, vec![transform]).await;
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].num_rows(), 1);
        let (by_name, categories) = raster_categorical_dimension(&batches[0], 0, 2);
        assert_eq!(by_name, "group");
        assert_eq!(categories, vec!["a", "b", "c"]);
        assert_eq!(raster_values_dims(&batches[0], 0), vec!["group", "y", "x"]);
        // Plane-major data: a hits cells 0 and 3 ((0,0) and (2,2));
        // b = 2 hits in cell 0; c = 1 hit at (x=2, y=0) -> cell 1.
        assert_eq!(
            raster_count_values(&batches[0], 0),
            vec![
                1, 0, 0, 1, // a
                2, 0, 0, 0, // b
                0, 1, 0, 0, // c
            ]
        );
        // The uniform dims are unchanged in slots 0/1.
        assert_eq!(
            raster_uniform_dimension(&batches[0], 0, 0),
            ("x".to_string(), "linear".to_string(), 0.0, 2.0, 2)
        );
    }

    #[tokio::test]
    async fn rasterize_2d_by_merges_disjoint_categories_across_partitions() {
        use datafusion::datasource::MemTable;
        let ctx = SessionContext::new();
        let schema = Arc::new(Schema::new(vec![
            Field::new("cat", DataType::Utf8, true),
            Field::new("x", DataType::Float64, true),
            Field::new("y", DataType::Float64, true),
        ]));
        let batch_for = |cats: Vec<&str>, xs: Vec<f64>, ys: Vec<f64>| {
            RecordBatch::try_new(
                schema.clone(),
                vec![
                    Arc::new(StringArray::from(cats)) as _,
                    Arc::new(Float64Array::from(xs)) as _,
                    Arc::new(Float64Array::from(ys)) as _,
                ],
            )
            .unwrap()
        };
        // Partition 0 sees only "b" then "a"; partition 1 sees "c" and "a"
        // — cross-partition merge must unify planes by VALUE.
        let partitions = vec![
            vec![batch_for(
                vec!["b", "a", "b"],
                vec![0.0, 0.0, 0.0],
                vec![0.0, 0.0, 0.0],
            )],
            vec![batch_for(vec!["c", "a"], vec![2.0, 0.0], vec![0.0, 0.0])],
        ];
        let table = Arc::new(MemTable::try_new(schema.clone(), partitions).unwrap());
        ctx.register_table("multi_part", table).unwrap();
        let dataframe = ctx.table("multi_part").await.unwrap();

        let (transform, _) = compile_transform(
            Rasterize2D::new(col("x"), col("y"))
                .x(|x| x.extent(0.0, 2.0).bins(2))
                .y(|y| y.extent(0.0, 2.0).bins(2))
                .by(col("cat"))
                .agg("count"),
        );
        let batches = transformed_batches(&ctx, dataframe, vec![transform]).await;
        assert_eq!(batches.len(), 1);
        let (_, categories) = raster_categorical_dimension(&batches[0], 0, 2);
        assert_eq!(categories, vec!["a", "b", "c"]);
        assert_eq!(
            raster_count_values(&batches[0], 0),
            vec![
                2, 0, 0, 0, // a: one per partition
                2, 0, 0, 0, // b
                0, 1, 0, 0, // c
            ]
        );
    }

    #[tokio::test]
    async fn rasterize_2d_by_stringifies_integer_categories() {
        let ctx = SessionContext::new();
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("passengers", DataType::Int64, true),
                Field::new("x", DataType::Float64, true),
                Field::new("y", DataType::Float64, true),
            ])),
            vec![
                Arc::new(datafusion::arrow::array::Int64Array::from(vec![2, 1, 2])) as _,
                Arc::new(Float64Array::from(vec![0.0, 0.0, 2.0])) as _,
                Arc::new(Float64Array::from(vec![0.0, 0.0, 0.0])) as _,
            ],
        )
        .unwrap();
        let dataframe = ctx.read_batch(batch).unwrap();
        let (transform, _) = compile_transform(
            Rasterize2D::new(col("x"), col("y"))
                .x(|x| x.extent(0.0, 2.0).bins(2))
                .y(|y| y.extent(0.0, 2.0).bins(2))
                .by(col("passengers"))
                .agg("count"),
        );
        let batches = transformed_batches(&ctx, dataframe, vec![transform]).await;
        let (by_name, categories) = raster_categorical_dimension(&batches[0], 0, 2);
        assert_eq!(by_name, "passengers");
        assert_eq!(categories, vec!["1", "2"]);
        assert_eq!(
            raster_count_values(&batches[0], 0),
            vec![
                1, 0, 0, 0, // "1"
                1, 1, 0, 0, // "2"
            ]
        );
    }

    #[tokio::test]
    async fn rasterize_2d_by_composes_with_partition_by() {
        let ctx = SessionContext::new();
        // group is the partition; value column doubles as the by category
        // via a string cast of value parity? Keep it simple: partition by
        // group, by-categorize on a second string derived from x position.
        let dataframe = rasterize_dataframe(
            &ctx,
            vec![Some("A"), Some("A"), Some("B"), Some("B")],
            vec![Some(0.0), Some(2.0), Some(0.0), Some(0.0)],
            vec![Some(0.0), Some(0.0), Some(0.0), Some(0.0)],
            vec![Some(1.0), Some(2.0), Some(1.0), Some(1.0)],
        );
        let (transform, _) = compile_transform(
            Rasterize2D::new(col("x"), col("y"))
                .x(|x| x.extent(0.0, 2.0).bins(2))
                .y(|y| y.extent(0.0, 2.0).bins(2))
                .partition_by([col("group")])
                .by(col("value"))
                .agg("count"),
        );
        let batches = transformed_batches(&ctx, dataframe, vec![transform]).await;
        let mut by_group: IndexMap<String, (Vec<String>, Vec<u64>)> = IndexMap::new();
        for batch in &batches {
            let groups = batch
                .column_by_name("group")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            for row in 0..batch.num_rows() {
                let (_, categories) = raster_categorical_dimension(batch, row, 2);
                by_group.insert(
                    groups.value(row).to_string(),
                    (categories, raster_count_values(batch, row)),
                );
            }
        }
        // A observed value-categories {1, 2}; B only {1} — per-row planes.
        assert_eq!(by_group["A"].0, vec!["1.0", "2.0"]);
        assert_eq!(by_group["A"].1, vec![1, 0, 0, 0, 0, 1, 0, 0]);
        assert_eq!(by_group["B"].0, vec!["1.0"]);
        assert_eq!(by_group["B"].1, vec![2, 0, 0, 0]);
    }

    #[tokio::test]
    async fn rasterize_2d_by_sum_planes() {
        let ctx = SessionContext::new();
        let dataframe = rasterize_dataframe(
            &ctx,
            vec![Some("a"), Some("a"), Some("b")],
            vec![Some(0.0), Some(0.0), Some(2.0)],
            vec![Some(0.0), Some(0.0), Some(0.0)],
            vec![Some(1.5), Some(2.5), Some(4.0)],
        );
        let (transform, _) = compile_transform(
            Rasterize2D::new(col("x"), col("y"))
                .x(|x| x.extent(0.0, 2.0).bins(2))
                .y(|y| y.extent(0.0, 2.0).bins(2))
                .by(col("group"))
                .value(col("value"))
                .agg("sum"),
        );
        let batches = transformed_batches(&ctx, dataframe, vec![transform]).await;
        assert_eq!(
            raster_f64_values(&batches[0], 0),
            vec![
                Some(4.0),
                None,
                None,
                None, // a
                None,
                Some(4.0),
                None,
                None, // b
            ]
        );
    }

    #[test]
    fn rasterize_2d_by_serialization_round_trip_and_untagged_bytes() {
        let (with_by, _) = compile_transform(
            Rasterize2D::new(col("x"), col("y"))
                .x(|x| x.extent(0.0, 2.0).bins(2))
                .y(|y| y.extent(0.0, 2.0).bins(2))
                .by(col("cat"))
                .agg("count"),
        );
        let json = serde_json::to_string(&with_by.transform).unwrap();
        assert!(json.contains("by_dim_name"));
        let decoded: Box<dyn avenger_chart_core::CompiledDataTransform> =
            serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::to_string(&decoded).unwrap(), json);

        // Without by(...), the serialized form must not mention the new
        // fields at all — existing materialization keys/identities and
        // specs stay byte-identical.
        let (without_by, _) = compile_transform(
            Rasterize2D::new(col("x"), col("y"))
                .x(|x| x.extent(0.0, 2.0).bins(2))
                .y(|y| y.extent(0.0, 2.0).bins(2))
                .agg("count"),
        );
        let json = serde_json::to_string(&without_by.transform).unwrap();
        assert!(!json.contains("\"by\""));
        assert!(!json.contains("by_dim_name"));
    }

    #[tokio::test]
    async fn rasterize_2d_partitioned_output_returns_one_raster_per_group() {
        let ctx = SessionContext::new();
        let dataframe = rasterize_dataframe(
            &ctx,
            vec![Some("A"), Some("A"), Some("B"), Some("B"), Some("B")],
            vec![Some(0.0), Some(2.0), Some(0.0), Some(1.0), Some(2.0)],
            vec![Some(0.0), Some(2.0), Some(0.0), Some(1.0), Some(0.0)],
            vec![Some(1.0); 5],
        );
        let (transform, _) = compile_transform(
            Rasterize2D::new(col("x"), col("y"))
                .x(|x| x.extent(0.0, 2.0).bins(2))
                .y(|y| y.extent(0.0, 2.0).bins(2))
                .partition_by([col("group")])
                .agg("count"),
        );

        let batches = transformed_batches(&ctx, dataframe, vec![transform]).await;
        let mut counts_by_group = IndexMap::new();
        for batch in &batches {
            let groups = batch
                .column_by_name("group")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            for row in 0..batch.num_rows() {
                counts_by_group.insert(
                    groups.value(row).to_string(),
                    raster_count_values(batch, row),
                );
            }
        }
        assert_eq!(counts_by_group["A"], vec![1, 0, 0, 1]);
        assert_eq!(counts_by_group["B"], vec![1, 1, 0, 1]);
    }

    #[tokio::test]
    async fn rasterize_2d_executor_partitioned_output_returns_one_raster_per_group() {
        let ctx = SessionContext::new();
        let dataframe = rasterize_dataframe(
            &ctx,
            vec![Some("A"), Some("A"), Some("B"), Some("B"), Some("B")],
            vec![Some(0.0), Some(2.0), Some(0.0), Some(1.0), Some(2.0)],
            vec![Some(0.0), Some(2.0), Some(0.0), Some(1.0), Some(0.0)],
            vec![Some(1.0); 5],
        );
        let batch = rasterize_executor_batch(
            &ctx,
            dataframe,
            Rasterize2D::new(col("x"), col("y"))
                .x(|x| x.extent(0.0, 2.0).bins(2))
                .y(|y| y.extent(0.0, 2.0).bins(2))
                .partition_by([col("group")])
                .agg("count"),
            IndexMap::new(),
        )
        .await;
        let groups = batch
            .column_by_name("group")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let mut counts_by_group = IndexMap::new();
        for row in 0..batch.num_rows() {
            counts_by_group.insert(
                groups.value(row).to_string(),
                raster_count_values(&batch, row),
            );
        }
        assert_eq!(counts_by_group["A"], vec![1, 0, 0, 1]);
        assert_eq!(counts_by_group["B"], vec![1, 1, 0, 1]);
    }

    fn rasterize_value_reducer_dataframe(ctx: &SessionContext) -> DataFrame {
        rasterize_dataframe(
            ctx,
            vec![None; 11],
            vec![
                Some(0.0),
                Some(0.2),
                Some(2.0),
                Some(2.0),
                Some(2.0),
                Some(2.0),
                Some(2.0),
                Some(1.5),
                Some(2.0),
                Some(-1.0),
                Some(0.0),
            ],
            vec![
                Some(0.0),
                Some(0.2),
                Some(0.0),
                Some(2.0),
                Some(2.0),
                Some(2.0),
                Some(0.0),
                Some(1.5),
                Some(2.0),
                Some(0.0),
                Some(0.0),
            ],
            vec![
                Some(1.0),
                Some(3.0),
                Some(-2.0),
                Some(5.0),
                Some(7.0),
                Some(9.0),
                Some(f64::NAN),
                None,
                Some(f64::INFINITY),
                Some(100.0),
                None,
            ],
        )
    }

    async fn rasterize_value_reducer_values(ctx: &SessionContext, agg: &str) -> Vec<Option<f64>> {
        let dataframe = rasterize_value_reducer_dataframe(ctx);
        let (transform, _) = compile_transform(
            Rasterize2D::new(col("x"), col("y"))
                .x(|x| x.extent(0.0, 2.0).bins(2))
                .y(|y| y.extent(0.0, 2.0).bins(2))
                .value(col("value"))
                .agg(agg),
        );
        let batches = transformed_batches(ctx, dataframe, vec![transform]).await;
        raster_f64_values(&batches[0], 0)
    }

    #[tokio::test]
    async fn rasterize_2d_sum_min_max_skip_invalid_values_and_null_empty_cells() {
        let ctx = SessionContext::new();
        assert_option_f64_close(
            &rasterize_value_reducer_values(&ctx, "sum").await,
            &[Some(4.0), Some(-2.0), None, Some(21.0)],
        );
        assert_option_f64_close(
            &rasterize_value_reducer_values(&ctx, "min").await,
            &[Some(1.0), Some(-2.0), None, Some(5.0)],
        );
        assert_option_f64_close(
            &rasterize_value_reducer_values(&ctx, "max").await,
            &[Some(3.0), Some(-2.0), None, Some(9.0)],
        );
    }

    #[tokio::test]
    async fn rasterize_2d_mean_variance_and_stddev_match_hand_computed_values() {
        let ctx = SessionContext::new();
        assert_option_f64_close(
            &rasterize_value_reducer_values(&ctx, "mean").await,
            &[Some(2.0), Some(-2.0), None, Some(7.0)],
        );
        assert_option_f64_close(
            &rasterize_value_reducer_values(&ctx, "var_pop").await,
            &[Some(1.0), Some(0.0), None, Some(8.0 / 3.0)],
        );
        assert_option_f64_close(
            &rasterize_value_reducer_values(&ctx, "var_samp").await,
            &[Some(2.0), None, None, Some(4.0)],
        );
        assert_option_f64_close(
            &rasterize_value_reducer_values(&ctx, "stddev_pop").await,
            &[Some(1.0), Some(0.0), None, Some((8.0_f64 / 3.0).sqrt())],
        );
        assert_option_f64_close(
            &rasterize_value_reducer_values(&ctx, "stddev_samp").await,
            &[Some(2.0_f64.sqrt()), None, None, Some(2.0)],
        );
    }

    #[tokio::test]
    async fn rasterize_2d_executor_value_reducers_match_sync_transform() {
        let ctx = SessionContext::new();
        for (agg, expected) in [
            ("sum", vec![Some(4.0), Some(-2.0), None, Some(21.0)]),
            ("min", vec![Some(1.0), Some(-2.0), None, Some(5.0)]),
            ("max", vec![Some(3.0), Some(-2.0), None, Some(9.0)]),
            ("mean", vec![Some(2.0), Some(-2.0), None, Some(7.0)]),
            ("var_pop", vec![Some(1.0), Some(0.0), None, Some(8.0 / 3.0)]),
            ("var_samp", vec![Some(2.0), None, None, Some(4.0)]),
            (
                "stddev_pop",
                vec![Some(1.0), Some(0.0), None, Some((8.0_f64 / 3.0).sqrt())],
            ),
            (
                "stddev_samp",
                vec![Some(2.0_f64.sqrt()), None, None, Some(2.0)],
            ),
        ] {
            let batch = rasterize_executor_batch(
                &ctx,
                rasterize_value_reducer_dataframe(&ctx),
                Rasterize2D::new(col("x"), col("y"))
                    .x(|x| x.extent(0.0, 2.0).bins(2))
                    .y(|y| y.extent(0.0, 2.0).bins(2))
                    .value(col("value"))
                    .agg(agg),
                IndexMap::new(),
            )
            .await;
            assert_option_f64_close(&raster_f64_values(&batch, 0), &expected);
        }
    }

    #[tokio::test]
    async fn rasterize_2d_partitioned_value_reducer_output_returns_float_rasters() {
        let ctx = SessionContext::new();
        let dataframe = rasterize_dataframe(
            &ctx,
            vec![Some("A"), Some("A"), Some("B"), Some("B"), Some("B")],
            vec![Some(0.0), Some(0.2), Some(0.0), Some(2.0), Some(2.0)],
            vec![Some(0.0), Some(0.2), Some(0.0), Some(2.0), Some(2.0)],
            vec![Some(1.0), Some(3.0), Some(2.0), Some(6.0), Some(8.0)],
        );
        let (transform, _) = compile_transform(
            Rasterize2D::new(col("x"), col("y"))
                .x(|x| x.extent(0.0, 2.0).bins(2))
                .y(|y| y.extent(0.0, 2.0).bins(2))
                .value(col("value"))
                .partition_by([col("group")])
                .agg("mean"),
        );

        let batches = transformed_batches(&ctx, dataframe, vec![transform]).await;
        let mut values_by_group = IndexMap::new();
        for batch in &batches {
            let groups = batch
                .column_by_name("group")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            for row in 0..batch.num_rows() {
                values_by_group
                    .insert(groups.value(row).to_string(), raster_f64_values(batch, row));
            }
        }
        assert_option_f64_close(&values_by_group["A"], &[Some(2.0), None, None, None]);
        assert_option_f64_close(&values_by_group["B"], &[Some(2.0), None, None, Some(7.0)]);
    }

    #[test]
    fn rasterize_2d_output_handle_uses_alias_dimension_names() {
        let (_compiled_transform, output) = compile_transform(
            Rasterize2D::new(
                (col("value") / lit(10.0)).alias("value_tens"),
                col("category").alias("category_axis"),
            )
            .x(|x| x.extent(0.0, 10.0).bins(32))
            .y(|y| y.extent(0.0, 4.0).bins(16))
            .agg("count"),
        );

        assert_eq!(output.x_dim().name(), "value_tens");
        assert_eq!(output.y_dim().name(), "category_axis");
        match output.raster() {
            Expr::Column(column) => assert_eq!(column.name, "raster"),
            other => panic!("expected raster column expression, got {other:?}"),
        }
    }

    #[test]
    fn rasterize_2d_output_handle_uses_datafusion_names_for_unaliased_expressions() {
        let x = col("x") + lit(1.0);
        let y = col("y") * lit(2.0);
        let expected_x = x.name_for_alias().unwrap();
        let expected_y = y.name_for_alias().unwrap();

        let (_compiled_transform, output) = compile_transform(
            Rasterize2D::new(x, y)
                .x(|x| x.extent(0.0, 10.0).bins(32))
                .y(|y| y.extent(0.0, 4.0).bins(16))
                .agg("count"),
        );

        assert_eq!(output.x_dim().name(), expected_x);
        assert_eq!(output.y_dim().name(), expected_y);
    }

    #[test]
    fn rasterize_2d_rejects_duplicate_dimension_names() {
        let result = Rasterize2D::new(col("value"), col("value"))
            .x(|x| x.extent(0.0, 10.0).bins(32))
            .y(|y| y.extent(0.0, 4.0).bins(16))
            .agg("count")
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free));
        let err = result.err().expect("duplicate dimension names should fail");
        assert!(
            err.to_string().contains("dimension names must be distinct"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rasterize_2d_rejects_invalid_reducer_name() {
        let result = Rasterize2D::new(col("x"), col("y"))
            .agg("median")
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free));
        let err = result.err().expect("invalid reducer should fail");
        assert!(
            err.to_string()
                .contains("reducer \"median\" is not supported"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rasterize_2d_value_reducer_requires_value_expression() {
        let result = Rasterize2D::new(col("x"), col("y"))
            .agg("mean")
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free));
        let err = result.err().expect("mean without value should fail");
        assert!(
            err.to_string().contains("requires value"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rasterize_2d_partition_by_requires_simple_columns() {
        let result = Rasterize2D::new(col("x"), col("y"))
            .partition_by([col("category") + lit("_suffix")])
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free));
        let err = result.err().expect("computed partition key should fail");
        assert!(
            err.to_string().contains("simple column"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn rasterize_2d_rejects_excessive_grid_size() {
        let ctx = SessionContext::new();
        let dataframe = rasterize_dataframe(
            &ctx,
            vec![None],
            vec![Some(0.0)],
            vec![Some(0.0)],
            vec![Some(1.0)],
        );
        let (compiled_transform, _) = compile_transform(
            Rasterize2D::new(col("x"), col("y"))
                .x(|x| x.extent(0.0, 1.0).bins(4097))
                .y(|y| y.extent(0.0, 1.0).bins(4097))
                .agg("count"),
        );
        let err = avenger_chart_core::apply_compiled_data_transforms(
            dataframe,
            &[compiled_transform],
            &DataTransformExecutionContext {
                session_context: &ctx,
                params: &IndexMap::new(),
                time_context: TimeContext::default(),
                facet_context: None,
            },
        )
        .await
        .err()
        .expect("excessive grid should fail");
        assert!(
            err.to_string().contains("above the current limit"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rasterize_2d_rejects_unsupported_sampling() {
        let result = Rasterize2D::new(col("x"), col("y"))
            .x(|x| x.sampling("log10"))
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free));
        let err = result.err().expect("unsupported sampling should fail");
        assert!(
            err.to_string().contains("sampling(\"linear\")"),
            "unexpected error: {err}"
        );
    }

    // ----- ScalarAggregate -----

    async fn scalar_aggregate_materialization(
        ctx: &SessionContext,
        dataframe: &DataFrame,
        transform: ScalarAggregate,
        params: IndexMap<String, ScalarValue>,
    ) -> avenger_chart_core::ViewScalarMaterialization {
        let (stage, _) = compile_transform(transform);
        let materialization_ctx = ViewMaterializationContext {
            session_context: ctx,
            params: &params,
            time_context: TimeContext::default(),
            facet_context: None,
            policy: MaterializationPolicy::default(),
            priority: 0.0,
        };
        stage
            .transform
            .view_scalar_materialization_request(dataframe, &materialization_ctx)
            .unwrap()
            .expect("eager ScalarAggregate should offer scalar materialization")
    }

    #[tokio::test]
    async fn scalar_aggregate_materialization_key_moves_with_params_identity_stable() {
        let ctx = SessionContext::new();
        let df = ctx
            .sql("SELECT * FROM (VALUES (1.0), (2.0), (3.0)) AS t(x)")
            .await
            .unwrap();
        let mut params_a = IndexMap::new();
        params_a.insert("domain_start".to_string(), ScalarValue::Float64(Some(0.0)));
        let mut params_b = IndexMap::new();
        params_b.insert("domain_start".to_string(), ScalarValue::Float64(Some(2.5)));

        let first = scalar_aggregate_materialization(
            &ctx,
            &df,
            ScalarAggregate::new().count("n"),
            params_a,
        )
        .await;
        let second = scalar_aggregate_materialization(
            &ctx,
            &df,
            ScalarAggregate::new().count("n"),
            params_b,
        )
        .await;

        assert_eq!(first.measure_names, vec!["n".to_string()]);
        assert_ne!(
            first.request.key, second.request.key,
            "resolved param values must move the materialization key"
        );
        assert_eq!(
            first.request.identity, second.request.identity,
            "param values must NOT churn the materialization identity"
        );
        assert_eq!(
            first.request.kind.as_ref(),
            SCALAR_AGGREGATE_MATERIALIZATION_KIND
        );
    }

    #[tokio::test]
    async fn scalar_aggregate_lazy_declines_materialization() {
        let ctx = SessionContext::new();
        let df = ctx
            .sql("SELECT * FROM (VALUES (1.0)) AS t(x)")
            .await
            .unwrap();
        let (stage, _) = compile_transform(ScalarAggregate::new().count("n").lazy());
        let materialization_ctx = ViewMaterializationContext {
            session_context: &ctx,
            params: &IndexMap::new(),
            time_context: TimeContext::default(),
            facet_context: None,
            policy: MaterializationPolicy::default(),
            priority: 0.0,
        };
        assert!(
            stage
                .transform
                .view_scalar_materialization_request(&df, &materialization_ctx)
                .unwrap()
                .is_none(),
            "lazy ScalarAggregate never executes eagerly - nothing to materialize"
        );
    }

    #[tokio::test]
    async fn scalar_aggregate_executor_matches_eager_values() {
        let ctx = SessionContext::new();
        let df = ctx
            .sql("SELECT * FROM (VALUES (1.0), (2.0), (7.0)) AS t(x)")
            .await
            .unwrap();
        let transform = ScalarAggregate::new().count("n").sum("total", col("x"));
        let materialization =
            scalar_aggregate_materialization(&ctx, &df, transform.clone(), IndexMap::new()).await;
        let result = ScalarAggregateExecutor
            .run(
                materialization.request.clone(),
                MaterializationExecutionContext {
                    session_context: &ctx,
                    params: &IndexMap::new(),
                },
            )
            .await
            .unwrap();
        let MaterializationResult::RecordBatch(batch) = result else {
            panic!("expected record batch result");
        };
        let materialized =
            scalar_literals_from_batch(&materialization.measure_names, &batch).unwrap();

        let eager = apply_scalar_aggregate(&ctx, df, transform, &IndexMap::new()).await;
        for name in &materialization.measure_names {
            assert_eq!(
                materialized.get(name),
                eager.derived_scalars.get(name),
                "executor value for '{name}' must match the eager evaluation"
            );
        }

        // Warm-path round trip: literals -> batch -> literals.
        let warmed =
            scalar_batch_from_literals(&materialization.measure_names, &materialized).unwrap();
        let round_tripped =
            scalar_literals_from_batch(&materialization.measure_names, &warmed).unwrap();
        for name in &materialization.measure_names {
            assert_eq!(round_tripped.get(name), materialized.get(name));
        }
    }

    fn scalar_aggregate_all_measures() -> ScalarAggregate {
        ScalarAggregate::new()
            .count("n")
            .sum("total", col("value"))
            .mean("avg", col("value"))
            .min("lo", col("value"))
            .max("hi", col("value"))
            .median("mid", col("value"))
    }

    async fn apply_scalar_aggregate(
        ctx: &SessionContext,
        dataframe: DataFrame,
        transform: ScalarAggregate,
        params: &IndexMap<String, ScalarValue>,
    ) -> avenger_chart_core::DataTransformResult {
        let (stage, _) = compile_transform(transform);
        avenger_chart_core::apply_compiled_data_transforms(
            dataframe,
            &[stage],
            &DataTransformExecutionContext {
                session_context: ctx,
                params,
                time_context: TimeContext::default(),
                facet_context: None,
            },
        )
        .await
        .unwrap()
    }

    /// Expected values for `scalar_aggregate_all_measures` computed through
    /// the ordinary Aggregate transform on the same input.
    async fn aggregate_reference_values(
        ctx: &SessionContext,
        dataframe: DataFrame,
    ) -> IndexMap<String, ScalarValue> {
        let (stage, _) = compile_transform(
            Aggregate::new()
                .count("n")
                .sum("total", col("value"))
                .mean("avg", col("value"))
                .min("lo", col("value"))
                .max("hi", col("value"))
                .median("mid", col("value")),
        );
        let batches = transformed_batches(ctx, dataframe, vec![stage]).await;
        let batch = batches.first().expect("aggregate reference batch");
        let mut values = IndexMap::new();
        for (index, field) in batch.schema().fields().iter().enumerate() {
            values.insert(
                field.name().clone(),
                ScalarValue::try_from_array(batch.column(index), 0).unwrap(),
            );
        }
        values
    }

    #[tokio::test]
    async fn scalar_aggregate_passes_dataframe_through() {
        for lazy in [false, true] {
            let ctx = SessionContext::new();
            let input = stats_dataframe(&ctx);
            let input_batches = input.clone().collect().await.unwrap();
            let mut transform = ScalarAggregate::new().count("n");
            if lazy {
                transform = transform.lazy();
            }
            let result = apply_scalar_aggregate(&ctx, input, transform, &IndexMap::new()).await;
            let output_batches = result.dataframe.collect().await.unwrap();
            assert_eq!(input_batches, output_batches, "lazy={lazy}");
            assert!(result.derived_scalars.contains_key("n"));
        }
    }

    #[tokio::test]
    async fn scalar_aggregate_eager_values_match_aggregate() {
        let ctx = SessionContext::new();
        let expected = aggregate_reference_values(&ctx, stats_dataframe(&ctx)).await;
        let result = apply_scalar_aggregate(
            &ctx,
            stats_dataframe(&ctx),
            scalar_aggregate_all_measures(),
            &IndexMap::new(),
        )
        .await;
        for (name, expected_value) in &expected {
            let Some(Expr::Literal(actual, _)) = result.derived_scalars.get(name) else {
                panic!("expected eager literal for '{name}'");
            };
            assert_eq!(actual, expected_value, "measure '{name}'");
        }
    }

    #[tokio::test]
    async fn scalar_aggregate_lazy_values_match_aggregate() {
        let ctx = SessionContext::new();
        let expected = aggregate_reference_values(&ctx, stats_dataframe(&ctx)).await;
        let result = apply_scalar_aggregate(
            &ctx,
            stats_dataframe(&ctx),
            scalar_aggregate_all_measures().lazy(),
            &IndexMap::new(),
        )
        .await;
        for (name, expected_value) in &expected {
            let expr = result
                .derived_scalars
                .get(name)
                .expect("lazy scalar")
                .clone();
            assert!(
                matches!(expr, Expr::ScalarSubquery(_)),
                "expected subquery for '{name}'"
            );
            let values = eval_to_scalars(vec![expr], Some(&ctx), None)
                .await
                .expect("standalone lazy scalar evaluation");
            assert_eq!(&values[0], expected_value, "measure '{name}'");
        }
    }

    #[tokio::test]
    async fn scalar_aggregate_eager_binds_params() {
        let ctx = SessionContext::new();
        let threshold = Param::new("threshold", ScalarValue::Float64(Some(0.0)));
        let (filter_stage, _) =
            compile_transform(Filter::new(col("value").lt_eq(threshold.expr())));
        let (scalar_stage, _) = compile_transform(ScalarAggregate::new().count("n"));

        let mut counts = Vec::new();
        for bound in [3.5_f64, 30.0_f64] {
            let mut params = IndexMap::new();
            params.insert("threshold".to_string(), ScalarValue::Float64(Some(bound)));
            let result = avenger_chart_core::apply_compiled_data_transforms(
                stats_dataframe(&ctx),
                &[filter_stage.clone(), scalar_stage.clone()],
                &DataTransformExecutionContext {
                    session_context: &ctx,
                    params: &params,
                    time_context: TimeContext::default(),
                    facet_context: None,
                },
            )
            .await
            .unwrap();
            let Some(Expr::Literal(value, _)) = result.derived_scalars.get("n") else {
                panic!("expected eager count literal");
            };
            counts.push(value.clone());
        }
        // values 1,2,3 pass <= 3.5; 1,2,3,4,10,20,30 pass <= 30.0
        assert_eq!(counts[0], ScalarValue::Int64(Some(3)));
        assert_eq!(counts[1], ScalarValue::Int64(Some(7)));
    }

    #[tokio::test]
    async fn scalar_aggregate_empty_input_semantics() {
        for lazy in [false, true] {
            let ctx = SessionContext::new();
            let empty = bin_dataframe_from_values(&ctx, vec![]);
            let mut transform = ScalarAggregate::new()
                .count("n")
                .mean("avg", col("value"))
                .min("lo", col("value"))
                .max("hi", col("value"));
            if lazy {
                transform = transform.lazy();
            }
            let result = apply_scalar_aggregate(&ctx, empty, transform, &IndexMap::new()).await;
            async fn value_of(
                ctx: &SessionContext,
                result: &avenger_chart_core::DataTransformResult,
                name: &str,
            ) -> ScalarValue {
                let expr = result.derived_scalars.get(name).unwrap().clone();
                eval_to_scalars(vec![expr], Some(ctx), None)
                    .await
                    .expect("evaluate scalar")
                    .remove(0)
            }
            assert_eq!(
                value_of(&ctx, &result, "n").await,
                ScalarValue::Int64(Some(0)),
                "lazy={lazy}"
            );
            for name in ["avg", "lo", "hi"] {
                assert!(
                    value_of(&ctx, &result, name).await.is_null(),
                    "expected null '{name}' on empty input, lazy={lazy}"
                );
            }
        }
    }

    #[test]
    fn scalar_aggregate_duplicate_measure_name_errors() {
        let err = ScalarAggregate::new()
            .count("n")
            .sum("n", col("value"))
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free))
            .err()
            .expect("duplicate measure name should fail");
        assert!(err.to_string().contains("'n'"), "unexpected error: {err}");
    }

    #[test]
    fn scalar_aggregate_requires_a_measure() {
        let err = ScalarAggregate::new()
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free))
            .err()
            .expect("empty measure list should fail");
        assert!(
            err.to_string().contains("at least one measure"),
            "unexpected error: {err}"
        );
    }

    #[test]
    #[should_panic(expected = "Unknown scalar aggregate 'missing'")]
    fn scalar_aggregate_unknown_scalar_panics() {
        let (_, output) = compile_transform(ScalarAggregate::new().count("n"));
        let _ = output.scalar("missing");
    }

    #[test]
    fn scalar_aggregate_compiled_serialization_preserves_evaluation() {
        for (transform, expected) in [
            (
                ScalarAggregate::new().count("n"),
                ScalarAggregateEvaluation::Eager,
            ),
            (
                ScalarAggregate::new().count("n").lazy(),
                ScalarAggregateEvaluation::Lazy,
            ),
        ] {
            let (stage, _) = compile_transform(transform);
            let serialized = serde_json::to_string(&stage.transform).unwrap();
            let deserialized: Box<dyn avenger_chart_core::CompiledDataTransform> =
                serde_json::from_str(&serialized).unwrap();
            let compiled = serde_json::to_value(&deserialized).unwrap();
            assert_eq!(compiled["type"], "scalar_aggregate");
            assert_eq!(
                compiled["evaluation"],
                serde_json::to_value(expected).unwrap()
            );
        }
    }

    /// Stress determinism of grouped Rasterize2D under parallel,
    /// small-batch execution: repeated collects of the same plan must agree
    /// exactly. Guards against group-state mixups in the GroupsAccumulator
    /// (update/merge/EmitTo paths) that only fire under multi-partition
    /// execution — the mechanism behind load-dependent facet raster flakes.
    #[tokio::test]
    async fn rasterize_2d_grouped_parallel_execution_is_deterministic() {
        use datafusion::execution::config::SessionConfig;

        let config = SessionConfig::new()
            .with_target_partitions(8)
            .with_batch_size(64);
        let ctx = SessionContext::new_with_config(config);

        // 4 groups with very different densities over a 16x16 grid,
        // deterministic positions.
        let mut groups = Vec::new();
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        let mut values = Vec::new();
        for index in 0..20_000_u64 {
            let group = match index % 10 {
                0..=5 => "g1",
                6..=8 => "g2",
                9 => {
                    if index % 20 == 9 {
                        "g3"
                    } else {
                        "g4"
                    }
                }
                _ => unreachable!(),
            };
            let position = (index.wrapping_mul(2_654_435_761)) % 65_536;
            groups.push(Some(group));
            xs.push(Some((position % 256) as f64 / 256.0));
            ys.push(Some((position / 256) as f64 / 256.0));
            values.push(Some(1.0));
        }
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("group", DataType::Utf8, true),
                Field::new("x", DataType::Float64, true),
                Field::new("y", DataType::Float64, true),
                Field::new("value", DataType::Float64, true),
            ])),
            vec![
                Arc::new(StringArray::from(groups)) as _,
                Arc::new(Float64Array::from(xs)) as _,
                Arc::new(Float64Array::from(ys)) as _,
                Arc::new(Float64Array::from(values)) as _,
            ],
        )
        .unwrap();
        let df = ctx.read_batch(batch).unwrap();

        let (stage, _) = compile_transform(
            Rasterize2D::new(col("x"), col("y"))
                .x(|x| x.extent(0.0, 1.0).bins(16_usize))
                .y(|y| y.extent(0.0, 1.0).bins(16_usize))
                .partition_by([col("group")])
                .agg("count"),
        );

        let mut reference: Option<Vec<(String, Vec<u8>)>> = None;
        for round in 0..20 {
            let result = avenger_chart_core::apply_compiled_data_transforms(
                df.clone(),
                std::slice::from_ref(&stage),
                &DataTransformExecutionContext {
                    session_context: &ctx,
                    params: &IndexMap::new(),
                    time_context: TimeContext::default(),
                    facet_context: None,
                },
            )
            .await
            .unwrap();
            let batches = result
                .dataframe
                .sort(vec![col("group").sort(true, false)])
                .unwrap()
                .collect()
                .await
                .unwrap();
            let combined = {
                let schema = batches[0].schema();
                arrow::compute::concat_batches(&schema, &batches).unwrap()
            };
            assert_eq!(combined.num_rows(), 4, "round {round}: one row per group");
            // Serialize each row's raster struct for exact comparison.
            let mut rows = Vec::new();
            let group_col = combined
                .column_by_name("group")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            let raster_col = combined.column_by_name("raster").unwrap();
            for row in 0..combined.num_rows() {
                let value = ScalarValue::try_from_array(raster_col, row).unwrap();
                rows.push((
                    group_col.value(row).to_string(),
                    format!("{value:?}").into_bytes(),
                ));
            }
            match &reference {
                None => reference = Some(rows),
                Some(reference) => {
                    for (index, (expected, actual)) in reference.iter().zip(&rows).enumerate() {
                        assert_eq!(
                            expected.0, actual.0,
                            "round {round}: group order diverged at row {index}"
                        );
                        assert_eq!(
                            expected.1, actual.1,
                            "round {round}: raster payload diverged for group {}",
                            expected.0
                        );
                    }
                }
            }
        }
    }

    /// Repeated collects of the exact faceted taxi plan (CSV scan -> filter
    /// -> grouped Rasterize2D -> per-cell filter) must agree. Reproduces the
    /// flaky rasterize_taxi_pickup_count_facet_* visual tests at the data
    /// layer, with no rendering involved.
    #[tokio::test]
    async fn rasterize_2d_taxi_grouped_collects_are_deterministic() {
        let taxi_path = format!(
            "{}/../avenger-chart/tests/data/nyc_taxi_2015/nyc_taxi.csv",
            env!("CARGO_MANIFEST_DIR")
        );
        let ctx = SessionContext::new();
        let df = ctx
            .read_csv(taxi_path, datafusion::prelude::CsvReadOptions::new())
            .await
            .expect("load NYC taxi fixture")
            .filter(
                col("pickup_x")
                    .gt_eq(lit(-8_242_500.0))
                    .and(col("pickup_x").lt_eq(lit(-8_226_500.0)))
                    .and(col("pickup_y").gt_eq(lit(4_968_000.0)))
                    .and(col("pickup_y").lt_eq(lit(4_983_000.0))),
            )
            .expect("filter taxi fixture");

        let (stage, _) = compile_transform(
            Rasterize2D::new(col("pickup_x"), col("pickup_y"))
                .x(|x| x.extent(-8_242_500.0, -8_226_500.0).bins(64_usize))
                .y(|y| y.extent(4_968_000.0, 4_983_000.0).bins(64_usize))
                .partition_by([col("payment_type")])
                .agg("count"),
        );

        let mut reference: Option<Vec<String>> = None;
        for round in 0..12 {
            let result = avenger_chart_core::apply_compiled_data_transforms(
                df.clone(),
                std::slice::from_ref(&stage),
                &DataTransformExecutionContext {
                    session_context: &ctx,
                    params: &IndexMap::new(),
                    time_context: TimeContext::default(),
                    facet_context: None,
                },
            )
            .await
            .unwrap();
            let batches = result
                .dataframe
                .sort(vec![col("payment_type").sort(true, false)])
                .unwrap()
                .collect()
                .await
                .unwrap();
            let schema = batches[0].schema();
            let combined = arrow::compute::concat_batches(&schema, &batches).unwrap();
            let raster_col = combined.column_by_name("raster").unwrap();
            let mut rows = Vec::new();
            for row in 0..combined.num_rows() {
                rows.push(format!(
                    "{:?}",
                    ScalarValue::try_from_array(raster_col, row).unwrap()
                ));
            }
            match &reference {
                None => reference = Some(rows),
                Some(reference) => {
                    assert_eq!(reference.len(), rows.len(), "round {round}: row count");
                    for (index, (expected, actual)) in reference.iter().zip(&rows).enumerate() {
                        assert_eq!(
                            expected, actual,
                            "round {round}: raster diverged at sorted row {index}"
                        );
                    }
                }
            }
        }
    }

    /// Concurrent per-cell collects (the chart evaluates facet cells in
    /// parallel on one runtime) of the grouped taxi rasterization must agree
    /// with sequential reference results.
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn rasterize_2d_taxi_concurrent_cell_collects_are_deterministic() {
        let taxi_path = format!(
            "{}/../avenger-chart/tests/data/nyc_taxi_2015/nyc_taxi.csv",
            env!("CARGO_MANIFEST_DIR")
        );
        let ctx = SessionContext::new();
        let df = ctx
            .read_csv(taxi_path, datafusion::prelude::CsvReadOptions::new())
            .await
            .expect("load NYC taxi fixture")
            .filter(
                col("pickup_x")
                    .gt_eq(lit(-8_242_500.0))
                    .and(col("pickup_x").lt_eq(lit(-8_226_500.0)))
                    .and(col("pickup_y").gt_eq(lit(4_968_000.0)))
                    .and(col("pickup_y").lt_eq(lit(4_983_000.0))),
            )
            .expect("filter taxi fixture");

        let (stage, _) = compile_transform(
            Rasterize2D::new(col("pickup_x"), col("pickup_y"))
                .x(|x| x.extent(-8_242_500.0, -8_226_500.0).bins(64_usize))
                .y(|y| y.extent(4_968_000.0, 4_983_000.0).bins(64_usize))
                .partition_by([col("payment_type")])
                .agg("count"),
        );
        let result = avenger_chart_core::apply_compiled_data_transforms(
            df,
            std::slice::from_ref(&stage),
            &DataTransformExecutionContext {
                session_context: &ctx,
                params: &IndexMap::new(),
                time_context: TimeContext::default(),
                facet_context: None,
            },
        )
        .await
        .unwrap();
        let rasterized = result.dataframe;

        // Sequential reference per cell.
        let mut reference = Vec::new();
        for cell in 1..=4_i64 {
            let batches = rasterized
                .clone()
                .filter(col("payment_type").eq(lit(cell)))
                .unwrap()
                .collect()
                .await
                .unwrap();
            let schema = batches[0].schema();
            let combined = arrow::compute::concat_batches(&schema, &batches).unwrap();
            assert_eq!(combined.num_rows(), 1);
            let raster = combined.column_by_name("raster").unwrap();
            reference.push(format!(
                "{:?}",
                ScalarValue::try_from_array(raster, 0).unwrap()
            ));
        }

        for round in 0..12 {
            let mut handles = Vec::new();
            for cell in 1..=4_i64 {
                let cell_df = rasterized
                    .clone()
                    .filter(col("payment_type").eq(lit(cell)))
                    .unwrap();
                handles.push(tokio::spawn(async move {
                    let batches = cell_df.collect().await.unwrap();
                    let schema = batches[0].schema();
                    let combined = arrow::compute::concat_batches(&schema, &batches).unwrap();
                    assert_eq!(combined.num_rows(), 1);
                    let raster = combined.column_by_name("raster").unwrap();
                    format!("{:?}", ScalarValue::try_from_array(raster, 0).unwrap())
                }));
            }
            for (index, handle) in handles.into_iter().enumerate() {
                let actual = handle.await.unwrap();
                assert_eq!(
                    reference[index],
                    actual,
                    "round {round}: concurrent collect diverged for cell {}",
                    index + 1
                );
            }
        }
    }
}
