use crate::error::AvengerChartError;
use crate::facet::coordination::SerializableDataExtents;
use crate::facet::scalar_cmp::scalar_total_cmp;
use avenger_scales::scales::DomainKind;
use datafusion::common::{DFSchema, ScalarValue};
use datafusion::dataframe::DataFrame;
use datafusion::logical_expr::{Expr, ExprSchemable};
use datafusion::prelude::SessionContext;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomainSort {
    Ascending,
    Descending,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelDataKind {
    Categorical,
    Temporal,
    Numeric,
}

pub fn classify_channel(
    expr: &Expr,
    domain_kind: Option<DomainKind>,
    schema: &DFSchema,
) -> ChannelDataKind {
    if domain_kind == Some(DomainKind::Categorical) {
        return ChannelDataKind::Categorical;
    }

    if domain_kind == Some(DomainKind::Temporal) {
        return ChannelDataKind::Temporal;
    }

    let Ok(dt) = expr.get_type(schema) else {
        return ChannelDataKind::Numeric;
    };

    if is_categorical_data_type(&dt) {
        ChannelDataKind::Categorical
    } else if is_temporal_data_type(&dt) {
        ChannelDataKind::Temporal
    } else {
        ChannelDataKind::Numeric
    }
}

pub async fn compute_extents(
    kind: ChannelDataKind,
    df: &DataFrame,
    expr: &Expr,
    ctx: &SessionContext,
    sort_order: DomainSort,
) -> Result<SerializableDataExtents, AvengerChartError> {
    match kind {
        ChannelDataKind::Categorical => {
            compute_categorical_extents(df, expr, ctx, sort_order).await
        }
        ChannelDataKind::Temporal => compute_temporal_extents(df, expr, ctx).await,
        ChannelDataKind::Numeric => compute_numeric_extents(df, expr, ctx).await,
    }
}

/// Compute numeric extents (min, max) for an expression from a DataFrame
///
/// This is a lightweight operation that runs a simple SQL aggregate query
/// without building full scales (which would cause recursion in nested facets).
async fn compute_numeric_extents(
    df: &DataFrame,
    expr: &Expr,
    _ctx: &SessionContext,
) -> Result<SerializableDataExtents, AvengerChartError> {
    use datafusion::functions_aggregate::min_max::{max, min};

    let agg_df = df.clone().aggregate(
        vec![],
        vec![
            min(expr.clone()).alias("min_val"),
            max(expr.clone()).alias("max_val"),
        ],
    )?;

    let batches = agg_df.collect().await?;

    if batches.is_empty() || batches[0].num_rows() == 0 {
        return Err(AvengerChartError::InternalError(
            "Empty result from extent computation".to_string(),
        ));
    }

    let batch = &batches[0];
    let min_col = batch.column(0);
    let max_col = batch.column(1);

    // Check for NULL min/max values - this happens when all values are NULL
    // (e.g., all rows hit literal branches in a conditional)
    if min_col.is_null(0) || max_col.is_null(0) {
        return Err(AvengerChartError::InternalError(
            "No numeric data for extent computation (all values may be NULL from conditional literals)"
                .to_string(),
        ));
    }

    // Try to extract numeric values
    let min_val = extract_f64_from_array(min_col, 0)?;
    let max_val = extract_f64_from_array(max_col, 0)?;

    Ok(SerializableDataExtents::interval(min_val, max_val))
}

/// Compute categorical extents (unique values) for an expression from a DataFrame
///
/// This is a lightweight operation that runs a DISTINCT query to get unique values.
/// Used for categorical scale sharing in nested facets.
async fn compute_categorical_extents(
    df: &DataFrame,
    expr: &Expr,
    _ctx: &SessionContext,
    sort_order: DomainSort,
) -> Result<SerializableDataExtents, AvengerChartError> {
    // Use DISTINCT to get unique values
    let distinct_df = df.clone().select(vec![expr.clone()])?.distinct()?;
    let batches = distinct_df.collect().await?;

    // Extract values into Vec<ScalarValue>
    let mut values = Vec::new();
    for batch in &batches {
        let col = batch.column(0);
        for i in 0..col.len() {
            let scalar = ScalarValue::try_from_array(col, i)?;
            // Skip NULL values - these come from conditional literal branches
            // that use NULL placeholders and shouldn't affect the domain
            if scalar.is_null() {
                continue;
            }
            values.push(normalize_domain_scalar(scalar));
        }
    }

    if sort_order != DomainSort::None {
        // Sort for consistent ordering across facet cells
        values.sort_by(scalar_total_cmp);
        if sort_order == DomainSort::Descending {
            values.reverse();
        }
    }

    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
        eprintln!(
            "  compute_categorical_extents: extracted {} unique values",
            values.len()
        );
    }

    Ok(SerializableDataExtents::discrete(values))
}

async fn compute_temporal_extents(
    df: &DataFrame,
    expr: &Expr,
    _ctx: &SessionContext,
) -> Result<SerializableDataExtents, AvengerChartError> {
    use datafusion::functions_aggregate::min_max::{max, min};

    let agg_df = df.clone().aggregate(
        vec![],
        vec![
            min(expr.clone()).alias("min_val"),
            max(expr.clone()).alias("max_val"),
        ],
    )?;

    let batches = agg_df.collect().await?;

    if batches.is_empty() || batches[0].num_rows() == 0 {
        return Err(AvengerChartError::InternalError(
            "Empty result from extent computation".to_string(),
        ));
    }

    let batch = &batches[0];

    // Check for NULL min/max values - this happens when all values are NULL
    // (e.g., all rows hit literal branches in a conditional)
    if batch.column(0).is_null(0) || batch.column(1).is_null(0) {
        return Err(AvengerChartError::InternalError(
            "No temporal data for extent computation (all values may be NULL from conditional literals)"
                .to_string(),
        ));
    }

    let min_val = ScalarValue::try_from_array(batch.column(0), 0)?;
    let max_val = ScalarValue::try_from_array(batch.column(1), 0)?;

    let min_ms = scalar_to_timestamp_ms(&min_val).ok_or_else(|| {
        AvengerChartError::InternalError("Failed to interpret temporal min value".to_string())
    })?;
    let max_ms = scalar_to_timestamp_ms(&max_val).ok_or_else(|| {
        AvengerChartError::InternalError("Failed to interpret temporal max value".to_string())
    })?;

    Ok(SerializableDataExtents::temporal(min_ms, max_ms))
}

fn is_categorical_data_type(data_type: &datafusion::arrow::datatypes::DataType) -> bool {
    use datafusion::arrow::datatypes::DataType;
    matches!(
        data_type,
        DataType::Utf8
            | DataType::LargeUtf8
            | DataType::Utf8View
            | DataType::Boolean
            | DataType::Dictionary(_, _)
    )
}

fn is_temporal_data_type(data_type: &datafusion::arrow::datatypes::DataType) -> bool {
    use datafusion::arrow::datatypes::DataType;
    matches!(
        data_type,
        DataType::Date32
            | DataType::Date64
            | DataType::Timestamp(_, _)
            | DataType::Time32(_)
            | DataType::Time64(_)
            | DataType::Duration(_)
    )
}

fn extract_f64_from_array(
    array: &std::sync::Arc<dyn datafusion::arrow::array::Array>,
    index: usize,
) -> Result<f64, AvengerChartError> {
    use datafusion::arrow::array::*;
    use datafusion::arrow::datatypes::DataType;

    if array.is_null(index) {
        return Err(AvengerChartError::InternalError(
            "Null value in extent computation".to_string(),
        ));
    }

    match array.data_type() {
        DataType::Float64 => {
            let arr = array.as_any().downcast_ref::<Float64Array>().unwrap();
            Ok(arr.value(index))
        }
        DataType::Float32 => {
            let arr = array.as_any().downcast_ref::<Float32Array>().unwrap();
            Ok(arr.value(index) as f64)
        }
        DataType::Int64 => {
            let arr = array.as_any().downcast_ref::<Int64Array>().unwrap();
            Ok(arr.value(index) as f64)
        }
        DataType::Int32 => {
            let arr = array.as_any().downcast_ref::<Int32Array>().unwrap();
            Ok(arr.value(index) as f64)
        }
        DataType::Int16 => {
            let arr = array.as_any().downcast_ref::<Int16Array>().unwrap();
            Ok(arr.value(index) as f64)
        }
        DataType::Int8 => {
            let arr = array.as_any().downcast_ref::<Int8Array>().unwrap();
            Ok(arr.value(index) as f64)
        }
        DataType::UInt64 => {
            let arr = array.as_any().downcast_ref::<UInt64Array>().unwrap();
            Ok(arr.value(index) as f64)
        }
        DataType::UInt32 => {
            let arr = array.as_any().downcast_ref::<UInt32Array>().unwrap();
            Ok(arr.value(index) as f64)
        }
        DataType::UInt16 => {
            let arr = array.as_any().downcast_ref::<UInt16Array>().unwrap();
            Ok(arr.value(index) as f64)
        }
        DataType::UInt8 => {
            let arr = array.as_any().downcast_ref::<UInt8Array>().unwrap();
            Ok(arr.value(index) as f64)
        }
        _ => Err(AvengerChartError::InternalError(format!(
            "Unsupported data type for numeric extraction: {:?}",
            array.data_type()
        ))),
    }
}

pub(crate) fn normalize_domain_scalar(value: ScalarValue) -> ScalarValue {
    match value {
        ScalarValue::Dictionary(_, inner) => normalize_domain_scalar(*inner),
        other => other,
    }
}

pub(crate) fn scalar_to_timestamp_ms(value: &ScalarValue) -> Option<i64> {
    match value {
        ScalarValue::Date32(Some(days)) => Some(*days as i64 * 86_400_000),
        ScalarValue::Date64(Some(ms)) => Some(*ms),
        ScalarValue::TimestampSecond(Some(ts), _) => Some(*ts * 1000),
        ScalarValue::TimestampMillisecond(Some(ts), _) => Some(*ts),
        ScalarValue::TimestampMicrosecond(Some(ts), _) => Some(*ts / 1000),
        ScalarValue::TimestampNanosecond(Some(ts), _) => Some(*ts / 1_000_000),
        _ => None,
    }
}
