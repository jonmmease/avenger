pub mod arc;
pub mod area;
pub mod encoding;
pub mod image;
pub mod line;
pub mod path;
pub mod rect;
pub mod rule;
pub mod symbol;
pub mod text;
pub mod trail;

use crate::runtime::context::CompilationContext;
use crate::utils::ExprHelpers;
use crate::{
    error::AvengerChartError,
    types::mark::{Encoding, Mark},
};
use arrow::array::AsArray;
use arrow::datatypes::Float32Type;
use arrow::{
    array::{ArrayRef, Float32Array, RecordBatch},
    compute::{cast, concat_batches},
    datatypes::DataType,
};
use async_trait::async_trait;
use avenger_common::types::{
    AreaOrientation, ColorOrGradient, ImageAlign, ImageBaseline, StrokeCap, StrokeJoin,
};
use avenger_common::value::ScalarOrArray;
use avenger_scales::scales::coerce::Coercer;
use avenger_scales::utils::ScalarValueUtils;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_text::types::{FontStyle, FontWeight, TextAlign, TextBaseline};
use datafusion::common::ScalarValue;
use datafusion::prelude::{DataFrame, Expr};
use indexmap::IndexMap;
use paste::paste;
use std::collections::HashMap;

macro_rules! define_enum_extract_scalar {
    ($enum_type:ty) => {
        paste! {
            pub fn [<$enum_type:snake _scalar>](&self, field: &str) -> Result<Option<$enum_type>, AvengerChartError> {
                // Error if column is found but not a scalar
                if self.column_batch.column_by_name(field).is_some() {
                    return Err(AvengerChartError::InternalError(format!(
                        "Column {field} is not a scalar",
                    )));
                }

                // Return scalar value if found
                let Some(array) = self.scalar_batch.column_by_name(field) else {
                    return Ok(None);
                };

                let coercer = Coercer::default();
                let value =  coercer.[<to_ $enum_type:snake>](array)?;
                let Some(c) = value.first() else {
                    return Ok(None);
                };
                Ok(Some(c.clone()))
            }
        }
    };
}

pub struct CompiledMark {
    pub scene_marks: Vec<SceneMark>,
    pub details: HashMap<Vec<usize>, RecordBatch>,
}

#[async_trait]
pub trait MarkCompiler: Send + Sync + 'static {
    async fn compile(
        &self,
        mark: &Mark,
        context: &CompilationContext,
    ) -> Result<CompiledMark, AvengerChartError>;
}

#[derive(Clone, Debug, PartialEq)]
struct EncodingBatches {
    scalar_batch: RecordBatch,
    column_batch: RecordBatch,
    details_batch: Option<RecordBatch>,
}

impl EncodingBatches {
    pub fn new(
        scalar_batch: RecordBatch,
        column_batch: RecordBatch,
        details_batch: Option<RecordBatch>,
    ) -> Self {
        Self {
            scalar_batch,
            column_batch,
            details_batch,
        }
    }

    pub fn get_scalar_batch(&self) -> &RecordBatch {
        &self.scalar_batch
    }

    pub fn get_column_batch(&self) -> &RecordBatch {
        &self.column_batch
    }

    pub fn len(&self) -> usize {
        self.column_batch.num_rows()
    }

    pub fn array_for_field(&self, field: &str) -> Option<ArrayRef> {
        let array = if let Some(array) = self.column_batch.column_by_name(field) {
            array
        } else if let Some(array) = self.scalar_batch.column_by_name(field) {
            array
        } else {
            return None;
        };

        Some(array.clone())
    }

    pub fn numeric_scalar(&self, field: &str) -> Result<Option<f32>, AvengerChartError> {
        // Error if column is found but not a scalar
        if self.column_batch.column_by_name(field).is_some() {
            return Err(AvengerChartError::InternalError(format!(
                "Column {field} is not a scalar",
            )));
        }

        // Return scalar value if found
        let Some(array) = self.scalar_batch.column_by_name(field) else {
            return Ok(None);
        };
        let array = cast(array, &DataType::Float32)?;
        let array = array.as_primitive::<Float32Type>();
        Ok(Some(array.value(0)))
    }

    pub fn color_scalar(&self, field: &str) -> Result<Option<ColorOrGradient>, AvengerChartError> {
        // Error if column is found but not a scalar
        if self.column_batch.column_by_name(field).is_some() {
            return Err(AvengerChartError::InternalError(format!(
                "Column {field} is not a scalar",
            )));
        }

        // Return scalar value if found
        let Some(array) = self.scalar_batch.column_by_name(field) else {
            return Ok(None);
        };

        let coercer = Coercer::default();
        let colors = coercer.to_color(array, None)?;
        let Some(c) = colors.first() else {
            return Ok(None);
        };
        Ok(Some(c.clone()))
    }

    pub fn stroke_dash_scalar(&self, field: &str) -> Result<Option<Vec<f32>>, AvengerChartError> {
        // Error if column is found but not a scalar
        if self.column_batch.column_by_name(field).is_some() {
            return Err(AvengerChartError::InternalError(format!(
                "Column {field} is not a scalar",
            )));
        }

        // Return scalar value if found
        let Some(array) = self.scalar_batch.column_by_name(field) else {
            return Ok(None);
        };

        let coercer = Coercer::default();
        let colors = coercer.to_stroke_dash(array)?;
        let Some(c) = colors.first() else {
            return Ok(None);
        };
        Ok(Some(c.clone()))
    }

    define_enum_extract_scalar!(StrokeCap);
    define_enum_extract_scalar!(StrokeJoin);
    define_enum_extract_scalar!(ImageAlign);
    define_enum_extract_scalar!(ImageBaseline);
    define_enum_extract_scalar!(AreaOrientation);
    define_enum_extract_scalar!(TextAlign);
    define_enum_extract_scalar!(TextBaseline);
    define_enum_extract_scalar!(FontWeight);
    define_enum_extract_scalar!(FontStyle);
}

async fn eval_encoding_exprs(
    from: &Option<DataFrame>,
    encodings: &IndexMap<String, Expr>,
    details: &Option<Vec<String>>,
    context: &CompilationContext,
) -> Result<EncodingBatches, AvengerChartError> {
    // Get the dataset to use for this mark
    let from_df = if let Some(from) = from {
        from.clone()
    } else {
        // Single row DataFrame with no columns
        context.ctx.read_empty()?
    };

    // Exprs that don't reference any columns, that we will evaluate against the empty DataFrame
    let mut scalar_exprs: Vec<Expr> = Vec::new();

    // Exprs that reference columns, that we will evaluate against the from_df DataFrame
    let mut column_exprs: Vec<Expr> = Vec::new();

    for (name, encoding) in encodings.iter() {
        if encoding.column_refs().is_empty() {
            // scalar_exprs.push(encoding.clone().apply_params(&context.params)?.alias(name));
            scalar_exprs.push(encoding.clone().alias(name));
        } else {
            // column_exprs.push(encoding.clone().apply_params(&context.params)?.alias(name));
            column_exprs.push(encoding.clone().alias(name));
        }
    }

    // Get record batch for result of column exprs
    let column_exprs_df = from_df
        .clone()
        .select(column_exprs)?
        .with_param_values(context.param_values.clone())?;
    let column_exprs_schema = column_exprs_df.schema().inner().clone();
    let column_exprs_batch =
        concat_batches(&column_exprs_schema, &column_exprs_df.collect().await?)?;

    // Get/build DataFrame to evaluate scalar expressions against
    let scalar_exprs_df = context
        .ctx
        .read_empty()?
        .select(scalar_exprs)?
        .with_param_values(context.param_values.clone())?;

    let scalar_schema = scalar_exprs_df.schema().inner().clone();
    let scalar_exprs_batch = concat_batches(&scalar_schema, &scalar_exprs_df.collect().await?)?;

    // Collect details columns
    let details_exprs_batch = if let Some(details) = details {
        let detail_exprs_df = from_df
            .select_columns(&details.iter().map(|s| s.as_str()).collect::<Vec<_>>())?
            .with_param_values(context.param_values.clone())?;
        let details_exprs_schema = detail_exprs_df.schema().inner().clone();
        Some(concat_batches(
            &details_exprs_schema,
            &detail_exprs_df.collect().await?,
        )?)
    } else {
        None
    };

    Ok(EncodingBatches::new(
        scalar_exprs_batch,
        column_exprs_batch,
        details_exprs_batch,
    ))
}
