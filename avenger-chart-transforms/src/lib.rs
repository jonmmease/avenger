mod aggregate;
mod bin;
mod calculate;
mod common;
mod filter;
mod fold;
mod impute;
mod join_aggregate;
mod kde;
pub mod lump;
mod select;
mod stack;
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
pub use select::{CompiledSelectTransform, Select, SelectExprSpec};
pub use stack::{CompiledStackTransform, Stack, StackOffset, StackOutput, TransformSortSpec};
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
            Array, BooleanArray, Float64Array, Int32Array, Int64Array, StringArray, StructArray,
            TimestampMillisecondArray, UInt64Array,
        },
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    };
    use avenger_chart_core::{
        ChannelValue, CoordinationScope, DataTransform, DataTransformCompileContext,
        DataTransformExecutionContext, DataTransformStage, DefaultLogicalExprNodeExt, Param,
        TimeContext, WeekStart, collect_derived_scalar_ids, eval_to_scalars,
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

    fn lump_rows(
        batch: &RecordBatch,
    ) -> Vec<(Option<String>, f64, Option<String>, Option<f64>, bool)> {
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

    fn lump_rows_from_batches(
        batches: &[RecordBatch],
    ) -> Vec<(Option<String>, f64, Option<String>, Option<f64>, bool)> {
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
            },
        )
        .await
        .unwrap();
        let batches = result.dataframe.collect().await.unwrap();
        let rows: usize = batches.iter().map(|batch| batch.num_rows()).sum();
        assert_eq!(rows, 4);
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
