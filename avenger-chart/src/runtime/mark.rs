use std::collections::HashMap;

use arrow::{
    array::{ArrayRef, Float32Array, RecordBatch},
    compute::{cast, concat_batches},
    datatypes::DataType,
};
use async_trait::async_trait;
use avenger_common::value::ScalarOrArray;
use avenger_scales3::{
    coerce::{ColorCoercer, NumericCoercer},
    color_interpolator::ColorInterpolator,
    scales::ArrowScale,
};
use avenger_scenegraph::marks::{arc::SceneArcMark, mark::SceneMark};
use datafusion::{
    common::ParamValues,
    prelude::{DataFrame, Expr, SessionContext},
};
use indexmap::IndexMap;

use crate::{
    error::AvengerChartError,
    types::{
        mark::{Encoding, Mark},
        // scales::ScaleConfig,
    },
    utils::ExprHelpers,
};

use super::scale::{ArrowScaleImpl, EvaluatedScale};

// use super::scale::ScaleInterface;

macro_rules! apply_f32_encoding {
    ($mark:expr, $evaluted_scales:expr, $arrow_scales:expr, $encoding_batches:expr, $scene_mark:expr, $numeric_coercer:expr, $field:expr) => {
        if let Some(Encoding::Scaled(scaled)) = $mark.encodings.get($field) {
            let evaluated_scale = $evaluted_scales.get(scaled.get_scale()).ok_or(
                AvengerChartError::ScaleKindLookupError(scaled.get_scale().to_string()),
            )?;
            let arrow_scale = $arrow_scales.get(evaluated_scale.kind.as_str()).ok_or(
                AvengerChartError::ScaleKindLookupError(evaluated_scale.kind.clone()),
            )?;
            if let Some(x) = $encoding_batches.array_for_field($field) {
                $scene_mark.x = arrow_scale.scale_to_numeric(&evaluated_scale.config, &x)?;
            }
        } else {
            // No scale, try to coerce to numeric directly
            if let Some(x) = $encoding_batches.array_for_field($field) {
                $scene_mark.x = $numeric_coercer.coerce_numeric(&x)?;
            }
        }
    };
}

macro_rules! apply_color_encoding {
    ($mark:expr, $evaluted_scales:expr, $arrow_scales:expr, $encoding_batches:expr, $scene_mark:expr, $interpolator:expr, $color_coercer:expr, $field:expr) => {
        if let Some(Encoding::Scaled(scaled)) = $mark.encodings.get($field) {
            let evaluated_scale = $evaluted_scales.get(scaled.get_scale()).ok_or(
                AvengerChartError::ScaleKindLookupError(scaled.get_scale().to_string()),
            )?;
            let scale_impl = $arrow_scales.get(evaluated_scale.kind.as_str()).ok_or(
                AvengerChartError::ScaleKindLookupError(evaluated_scale.kind.clone()),
            )?;
            if let Some(value) = $encoding_batches.array_for_field($field) {
                $scene_mark.fill = scale_impl.scale_to_color(
                    &evaluated_scale.config,
                    &value,
                    $interpolator.as_ref(),
                )?;
            }
        } else {
            // No scale, try to coerce to color directly
            if let Some(value) = $encoding_batches.array_for_field($field) {
                $scene_mark.fill = $color_coercer.coerce_color(&value)?;
            }
        }
    };
}

#[async_trait]
pub trait MarkCompiler: Send + Sync + 'static {
    async fn compile(
        &self,
        mark: &Mark,
        ctx: &SessionContext,
        params: &ParamValues,
        evaluted_scales: &HashMap<String, EvaluatedScale>,
        scale_impls: &HashMap<String, Box<dyn ArrowScale>>,
        interpolator: &Box<dyn ColorInterpolator>,
        color_coercer: &Box<dyn ColorCoercer>,
        numeric_coercer: &Box<dyn NumericCoercer>,
    ) -> Result<Vec<SceneMark>, AvengerChartError>;
}

pub struct ArcMarkCompiler;

#[async_trait]
impl MarkCompiler for ArcMarkCompiler {
    async fn compile(
        &self,
        mark: &Mark,
        ctx: &SessionContext,
        params: &ParamValues,
        evaluted_scales: &HashMap<String, EvaluatedScale>,
        arrow_scales: &HashMap<String, Box<dyn ArrowScale>>,
        interpolator: &Box<dyn ColorInterpolator>,
        color_coercer: &Box<dyn ColorCoercer>,
        numeric_coercer: &Box<dyn NumericCoercer>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let encoding_batches =
            eval_encoding_exprs(&mark.from, &mark.encodings, ctx, params).await?;
        // Create a new default SceneArcMark
        let mut scene_mark = SceneArcMark::default();

        // Apply f32 encodings
        apply_f32_encoding!(
            mark,
            evaluted_scales,
            arrow_scales,
            encoding_batches,
            scene_mark,
            numeric_coercer,
            "x"
        );
        // if let Some(Encoding::Scaled(scaled)) = mark.encodings.get("x") {
        //     let evaluated_scale = evaluted_scales.get(scaled.get_scale()).ok_or(
        //         AvengerChartError::ScaleKindLookupError(scaled.get_scale().to_string()),
        //     )?;
        //     let arrow_scale = arrow_scales.get(evaluated_scale.kind.as_str()).ok_or(
        //         AvengerChartError::ScaleKindLookupError(evaluated_scale.kind.clone()),
        //     )?;
        //     if let Some(x) = encoding_batches.array_for_field("x") {
        //         scene_mark.x = arrow_scale.scale_to_numeric(&evaluated_scale.config, &x)?;
        //     }
        // } else {
        //     if let Some(x) = encoding_batches.f32_scalar_or_array_for_field("x")? {
        //         scene_mark.x = x;
        //     }
        // }
        // apply_f32_encoding!(mark, evaluted_scales, encoding_batches, scene_mark, "y");
        // apply_f32_encoding!(
        //     mark,
        //     evaluted_scales,
        //     encoding_batches,
        //     scene_mark,
        //     "start_angle"
        // );
        // apply_f32_encoding!(
        //     mark,
        //     evaluted_scales,
        //     encoding_batches,
        //     scene_mark,
        //     "end_angle"
        // );
        // apply_f32_encoding!(
        //     mark,
        //     evaluted_scales,
        //     encoding_batches,
        //     scene_mark,
        //     "outer_radius"
        // );
        // apply_f32_encoding!(
        //     mark,
        //     evaluted_scales,
        //     encoding_batches,
        //     scene_mark,
        //     "inner_radius"
        // );
        // apply_f32_encoding!(
        //     mark,
        //     evaluted_scales,
        //     encoding_batches,
        //     scene_mark,
        //     "pad_angle"
        // );
        // apply_f32_encoding!(
        //     mark,
        //     evaluted_scales,
        //     encoding_batches,
        //     scene_mark,
        //     "corner_radius"
        // );
        // apply_f32_encoding!(
        //     mark,
        //     evaluted_scales,
        //     encoding_batches,
        //     scene_mark,
        //     "stroke_width"
        // );

        // Apply color encoding
        apply_color_encoding!(
            mark,
            evaluted_scales,
            arrow_scales,
            encoding_batches,
            scene_mark,
            interpolator,
            color_coercer,
            "fill"
        );

        Ok(vec![SceneMark::Arc(scene_mark)])
    }
}

struct EncodingBatches {
    scalar_batch: RecordBatch,
    column_batch: RecordBatch,
}

impl EncodingBatches {
    pub fn new(scalar_batch: RecordBatch, column_batch: RecordBatch) -> Self {
        Self {
            scalar_batch,
            column_batch,
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

    pub fn f32_scalar_or_array_for_field(
        &self,
        field: &str,
    ) -> Result<Option<ScalarOrArray<f32>>, AvengerChartError> {
        // Loop in each batch for the named field
        let (array, is_scalar) = if let Some(array) = self.column_batch.column_by_name(field) {
            (array, false)
        } else if let Some(array) = self.scalar_batch.column_by_name(field) {
            (array, true)
        } else {
            // Not found in either batch
            return Ok(None);
        };

        // Cast to f32 then downcast to f32 array
        let array = cast(array, &DataType::Float32)?;
        let array = array.as_any().downcast_ref::<Float32Array>().ok_or(
            AvengerChartError::InternalError(format!(
                "Failed to downcast {field} to Float32Array: {array:?}",
            )),
        )?;

        if is_scalar {
            Ok(Some(ScalarOrArray::new_scalar(array.value(0))))
        } else {
            Ok(Some(ScalarOrArray::new_array(array.values().to_vec())))
        }
    }
}

async fn eval_encoding_exprs(
    from: &Option<String>,
    encodings: &IndexMap<String, Encoding>,
    ctx: &SessionContext,
    params: &ParamValues,
) -> Result<EncodingBatches, AvengerChartError> {
    // Get the dataset to use for this mark
    let from_df = if let Some(from) = from {
        // Registered DataFrame from
        ctx.table(from)
            .await
            .map_err(|_| AvengerChartError::DatasetLookupError(from.to_string()))?
    } else {
        // Single row DataFrame with no columns
        ctx.read_empty()?
    };
    let empty_df = ctx.read_empty()?;

    // Exprs that don't reference any columns, that we will evaluate against the empty DataFrame
    let mut scalar_exprs: Vec<Expr> = Vec::new();

    // Exprs that reference columns, that we will evaluate against the from_df DataFrame
    let mut column_exprs: Vec<Expr> = Vec::new();

    for (name, encoding) in encodings.iter() {
        if encoding.is_scalar() {
            // scalar_exprs.push(encoding.inner_expr().clone().apply_params(params)?.alias(name));
            scalar_exprs.push(encoding.inner_expr().clone().alias(name));
        } else {
            column_exprs.push(encoding.inner_expr().clone().alias(name));
        }
    }

    // Get record batch for result of column exprs
    let column_exprs_df = from_df.select(column_exprs)?;
    let column_exprs_schema = column_exprs_df.schema().inner().clone();
    let column_exprs_batch =
        concat_batches(&column_exprs_schema, &column_exprs_df.collect().await?)?;

    // Get/build DataFrame to evaluate scalar expressions against
    let scalar_exprs_df = empty_df
        .select(scalar_exprs)?
        .with_param_values(params.clone())?;
    let scalar_schema = scalar_exprs_df.schema().inner().clone();
    let scalar_exprs_batch = concat_batches(&scalar_schema, &scalar_exprs_df.collect().await?)?;

    Ok(EncodingBatches::new(scalar_exprs_batch, column_exprs_batch))
}

// use arrow::{
//     array::{Float32Array, RecordBatch},
//     datatypes::{DataType, Float32Type},
// };
// use avenger_common::value::ScalarOrArray;
// use avenger_scenegraph::marks::arc::SceneArcMark;

// use crate::types::mark::Mark;

// pub trait SceneMarkFromBatches {
//     fn from_batches(mark: &Mark, column_batch: &RecordBatch, scalar_batch: &RecordBatch) -> Self;
// }

// /// Extracts a float32 field from a column or scalar batch.
// macro_rules! extract_float32_field {
//     ($field:ident, $mark:expr, $column_batch:expr, $scalar_batch:expr) => {
//         if let Some(arr) = $column_batch.column_by_name(stringify!($field)) {
//             let arr = arr.as_any().downcast_ref::<Float32Array>().unwrap();
//             $mark.$field = ScalarOrArray::new_array(arr.values().to_vec());
//         } else if let Some(arr) = $scalar_batch.column_by_name(stringify!($field)) {
//             let arr = arr.as_any().downcast_ref::<Float32Array>().unwrap();
//             $mark.$field = ScalarOrArray::new_scalar(arr.value(0));
//         }
//     };
// }

// impl SceneMarkFromBatches for SceneArcMark {
//     fn from_batches(mark: &Mark, column_batch: &RecordBatch, scalar_batch: &RecordBatch) -> Self {
//         let mut mark = Self {
//             len: column_batch.num_rows() as u32,
//             ..Default::default()
//         };

//         // Handle float32 fields
//         extract_float32_field!(x, mark, column_batch, scalar_batch);
//         extract_float32_field!(y, mark, column_batch, scalar_batch);
//         extract_float32_field!(start_angle, mark, column_batch, scalar_batch);
//         extract_float32_field!(end_angle, mark, column_batch, scalar_batch);
//         extract_float32_field!(outer_radius, mark, column_batch, scalar_batch);
//         extract_float32_field!(inner_radius, mark, column_batch, scalar_batch);
//         extract_float32_field!(pad_angle, mark, column_batch, scalar_batch);
//         extract_float32_field!(corner_radius, mark, column_batch, scalar_batch);

//         // color
//         let fill_arr = if let Some(arr) = column_batch.column_by_name("fill") {
//             Some(arr.clone())
//         } else if let Some(arr) = scalar_batch.column_by_name("fill") {
//             Some(arr.clone())
//         } else {
//             None
//         };
//         // if let Some(arr) = fill_arr {
//         //     let dtype = arr.data_type();
//         //     match dtype {
//         //         DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => {
//         //             // css color strings
//         //             todo!()
//         //             // mark.fill = ScalarOrArray::new_array(arr.values().to_vec());
//         //         }
//         //         DataType::FixedSizeList(field, 4) if field.data_type().is_numeric() => {
//         //             // rgba components
//         //             todo!()
//         //         }
//         //         _ => {
//         //             mark.fill = ScalarOrArray::new_scalar(arr.value(0));
//         //         }
//         //     }
//         // }

//         // for (name, expr) in mark.encodings.iter() {
//         //     let value = expr.eval(&scalar_batch)?;
//         //     mark.encodings.insert(name.clone(), value);
//         // }

//         mark
//     }
// }
