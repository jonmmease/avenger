//! Extension traits for ConfiguredScale to add chart-specific functionality
//!
//! These traits extend avenger_scales::ConfiguredScale with DataFusion integration
//! and legend-specific convenience methods without adding dependencies to avenger-scales.

use std::sync::Arc;

use avenger_scales::scales::ConfiguredScale;
use datafusion::{
    arrow::{
        array::ArrayRef,
        datatypes::{DataType, Field, FieldRef},
    },
    logical_expr::{Expr, ExprSchemable, cast},
    prelude::SessionContext,
};
use datafusion_common::ScalarValue;

use avenger_chart_core::{
    AvengerChartError, ConfiguredScaleLegendExt, DefaultLogicalExprNodeExt, DomainValues,
    PositionBoundary,
};

use crate::{ConfiguredScaleWithSpec, udf::create_scale_udf};

/// Extension trait for DataFusion integration
pub trait ConfiguredScaleDataFusionExt {
    /// Create a DataFusion expression that applies this scale to input values
    fn to_expr(&self, input: Expr) -> Result<Expr, AvengerChartError>;

    /// Create a DataFusion expression with custom band parameter for band/point scales
    fn to_expr_with_band(&self, input: Expr, band: f64) -> Result<Expr, AvengerChartError>;

    /// Create a DataFusion expression with custom positional boundary metadata.
    fn to_expr_with_position_boundary(
        &self,
        input: Expr,
        boundary: &PositionBoundary,
        ctx: &SessionContext,
    ) -> Result<Expr, AvengerChartError>;
}

impl ConfiguredScaleDataFusionExt for ConfiguredScaleWithSpec {
    fn to_expr(&self, input: Expr) -> Result<Expr, AvengerChartError> {
        use datafusion::logical_expr::lit;
        use datafusion::prelude::named_struct;
        use datafusion_common::DFSchema;

        // Get data types from the configured scale
        let domain_type = self.configured.config.domain.data_type();
        let input_type = scale_input_type(self.configured.scale_impl.scale_type(), &domain_type);
        let range_type = self.configured.config.range.data_type();
        let empty_schema = DFSchema::empty();

        // Build options struct - convert avenger_scales::Scalar to expressions
        let options_expr = if self.configured.config.options.is_empty() {
            // Create empty struct
            lit(ScalarValue::Struct(
                datafusion::arrow::array::StructArray::new_empty_fields(1, None).into(),
            ))
        } else {
            // Convert HashMap<String, Scalar> to named_struct expression.
            // Sort by key: the options map is a HashMap, and letting its
            // per-instance iteration order pick the struct field order makes
            // the emitted plan nondeterministic across evaluations (options
            // are read back BY NAME, so field order is semantically inert,
            // but plan-identity consumers — display, proto serialization,
            // the physical result cache's fingerprints — all see it).
            let mut sorted_options: Vec<_> = self.configured.config.options.iter().collect();
            sorted_options.sort_by(|(a, _), (b, _)| a.cmp(b));
            let struct_args: Vec<Expr> = sorted_options
                .into_iter()
                .flat_map(|(key, value)| {
                    // Convert avenger_scales::Scalar to ScalarValue
                    let scalar_value =
                        ScalarValue::try_from_array(&value.0, 0).unwrap_or(ScalarValue::Null);
                    vec![lit(key.clone()), lit(scalar_value)]
                })
                .collect();
            named_struct(struct_args)
        };

        let options_type = options_expr.get_type(&empty_schema)?;

        // Use the stored Scale<Auto> directly - no need to recreate from scale type!
        // This preserves full extensibility for external scale types
        let udf = create_scale_udf(
            self.spec().clone(),
            domain_type.clone(),
            input_type.clone(),
            range_type.clone(),
            options_type,
        )?;

        // Convert arrays to ScalarValue::List for the UDF call
        let domain_scalar = array_to_list_scalar(self.configured.config.domain.clone())?;
        let range_scalar = array_to_list_scalar(self.configured.config.range.clone())?;

        // Call the UDF with domain, range, options, and input
        Ok(udf.call(vec![
            lit(domain_scalar),
            lit(range_scalar),
            options_expr,
            input,
        ]))
    }

    fn to_expr_with_band(&self, input: Expr, band: f64) -> Result<Expr, AvengerChartError> {
        // Band parameter controls position within a band for band/point scales:
        // - 0.0 = start of band
        // - 0.5 = center of band (default)
        // - 1.0 = end of band
        // For non-band scales, this parameter is ignored
        if scale_supports_option(self, "band") {
            to_expr_with_options(
                self,
                input,
                vec![(
                    "band".to_string(),
                    avenger_scales::scalar::Scalar::from_f32(band as f32),
                )],
            )
        } else {
            // For non-band scales, ignore the band parameter
            self.to_expr(input)
        }
    }

    fn to_expr_with_position_boundary(
        &self,
        input: Expr,
        boundary: &PositionBoundary,
        ctx: &SessionContext,
    ) -> Result<Expr, AvengerChartError> {
        match boundary {
            PositionBoundary::Band { band } => self.to_expr_with_band(input, *band),
            PositionBoundary::LevelBand { level, band } => {
                if !scale_supports_option(self, "level") || !scale_supports_option(self, "band") {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "level_band({level}, {band}) requires a nested band scale"
                    )));
                }

                to_expr_with_options(
                    self,
                    input,
                    vec![
                        (
                            "level".to_string(),
                            avenger_scales::scalar::Scalar::from_i32(*level as i32),
                        ),
                        (
                            "band".to_string(),
                            avenger_scales::scalar::Scalar::from_f32(*band as f32),
                        ),
                    ],
                )
            }
            PositionBoundary::BandExpr { band } => {
                let band = band.to_default_expr(ctx)?;
                to_expr_with_interpolated_band(self, input, None, band)
            }
            PositionBoundary::LevelBandExpr { level, band } => {
                let band = band.to_default_expr(ctx)?;
                to_expr_with_interpolated_band(self, input, Some(*level), band)
            }
        }
    }
}

fn scale_supports_option(scale: &ConfiguredScaleWithSpec, name: &str) -> bool {
    scale
        .configured
        .scale_impl
        .option_definitions()
        .iter()
        .any(|def| def.name == name)
}

fn to_expr_with_options(
    scale: &ConfiguredScaleWithSpec,
    input: Expr,
    options: Vec<(String, avenger_scales::scalar::Scalar)>,
) -> Result<Expr, AvengerChartError> {
    let mut config = scale.configured.config.clone();
    for (key, value) in options {
        config.options.insert(key, value);
    }

    let temp_configured = ConfiguredScale {
        scale_impl: scale.configured.scale_impl.clone(),
        config,
    };
    let temp_wrapper = ConfiguredScaleWithSpec::with_range_binding(
        scale.spec().clone(),
        temp_configured,
        scale.range_binding(),
    )
    .with_derived_scalars(scale.derived_scalars().clone());

    temp_wrapper.to_expr(input)
}

fn to_expr_with_interpolated_band(
    scale: &ConfiguredScaleWithSpec,
    input: Expr,
    level: Option<usize>,
    band: Expr,
) -> Result<Expr, AvengerChartError> {
    if !scale_supports_option(scale, "band") {
        return scale.to_expr(input);
    }
    if let Some(level) = level
        && !scale_supports_option(scale, "level")
    {
        return Err(AvengerChartError::InvalidArgument(format!(
            "level_band({level}, ...) requires a nested band scale"
        )));
    }

    let static_options = |band: f32| {
        let mut options = Vec::new();
        if let Some(level) = level {
            options.push((
                "level".to_string(),
                avenger_scales::scalar::Scalar::from_i32(level as i32),
            ));
        }
        options.push((
            "band".to_string(),
            avenger_scales::scalar::Scalar::from_f32(band),
        ));
        options
    };

    let start = to_expr_with_options(scale, input.clone(), static_options(0.0))?;
    let end = to_expr_with_options(scale, input, static_options(1.0))?;
    let start = cast(start, DataType::Float64);
    let end = cast(end, DataType::Float64);
    let band = cast(band, DataType::Float64);
    Ok(start.clone() + (end - start) * band)
}

fn scale_input_type(scale_type: &str, domain_type: &DataType) -> DataType {
    if scale_type != "nested_band" {
        return domain_type.clone();
    }

    let DataType::Struct(fields) = domain_type else {
        return domain_type.clone();
    };

    DataType::Struct(
        fields
            .iter()
            .map(|field| {
                Arc::new(Field::new(
                    field.name(),
                    nested_band_component_input_type(field.data_type()),
                    field.is_nullable(),
                )) as FieldRef
            })
            .collect::<Vec<_>>()
            .into(),
    )
}

fn nested_band_component_input_type(data_type: &DataType) -> DataType {
    let DataType::Struct(fields) = data_type else {
        return data_type.clone();
    };
    let is_labeled_component = fields.len() == 2
        && fields.iter().any(|field| field.name() == "key")
        && fields.iter().any(|field| field.name() == "label");
    if !is_labeled_component {
        return data_type.clone();
    }
    fields
        .iter()
        .find(|field| field.name() == "key")
        .map(|field| field.data_type().clone())
        .unwrap_or_else(|| data_type.clone())
}

// Implementation for ConfiguredScaleWithSpec - delegates to inner ConfiguredScale
impl ConfiguredScaleLegendExt for ConfiguredScaleWithSpec {
    fn domain_values(&self) -> Result<DomainValues, AvengerChartError> {
        self.configured.domain_values()
    }

    fn domain_labels(&self) -> Result<Vec<String>, AvengerChartError> {
        self.configured.domain_labels()
    }

    fn range_colors(&self) -> Result<Vec<[f32; 4]>, AvengerChartError> {
        self.configured.range_colors()
    }

    fn range_strings(&self) -> Result<Vec<String>, AvengerChartError> {
        self.configured.range_strings()
    }

    fn scale_scalars_to_numeric(
        &self,
        values: &[ScalarValue],
    ) -> Result<Vec<f32>, AvengerChartError> {
        self.configured.scale_scalars_to_numeric(values)
    }

    fn scale_scalars_to_colors(
        &self,
        values: &[ScalarValue],
    ) -> Result<Vec<[f32; 4]>, AvengerChartError> {
        self.configured.scale_scalars_to_colors(values)
    }

    fn scale_scalars_to_dash_patterns(&self, values: &[ScalarValue]) -> Vec<Option<Vec<f32>>> {
        self.configured.scale_scalars_to_dash_patterns(values)
    }
}

/// Convert an Arrow array to a ScalarValue::List
fn array_to_list_scalar(array: ArrayRef) -> Result<ScalarValue, AvengerChartError> {
    use datafusion::arrow::array::ListArray;
    use datafusion::arrow::buffer::OffsetBuffer;

    // Create a ListArray that wraps our array as a single list element
    let offsets = OffsetBuffer::from_lengths([array.len()]);
    let list_array = ListArray::try_new(
        Field::new("item", array.data_type().clone(), true).into(),
        offsets,
        array,
        None,
    )
    .map_err(|e| AvengerChartError::InternalError(format!("Failed to create list array: {}", e)))?;

    // Convert the first (and only) element to ScalarValue
    ScalarValue::try_from_array(&list_array, 0).map_err(|e| {
        AvengerChartError::InternalError(format!("Failed to convert to scalar: {}", e))
    })
}
