//! Extension traits for ConfiguredScale to add chart-specific functionality
//!
//! These traits extend avenger_scales::ConfiguredScale with DataFusion integration
//! and legend-specific convenience methods without adding dependencies to avenger-scales.

use avenger_scales::scales::ConfiguredScale;
use datafusion::{
    arrow::{array::ArrayRef, datatypes::Field},
    logical_expr::{Expr, ExprSchemable},
};
use datafusion_common::ScalarValue;

use avenger_chart_core::{AvengerChartError, ConfiguredScaleLegendExt, DomainValues};

use crate::{ConfiguredScaleWithSpec, udf::create_scale_udf};

/// Extension trait for DataFusion integration
pub trait ConfiguredScaleDataFusionExt {
    /// Create a DataFusion expression that applies this scale to input values
    fn to_expr(&self, input: Expr) -> Result<Expr, AvengerChartError>;

    /// Create a DataFusion expression with custom band parameter for band/point scales
    fn to_expr_with_band(&self, input: Expr, band: f64) -> Result<Expr, AvengerChartError>;
}

impl ConfiguredScaleDataFusionExt for ConfiguredScaleWithSpec {
    fn to_expr(&self, input: Expr) -> Result<Expr, AvengerChartError> {
        use datafusion::logical_expr::lit;
        use datafusion::prelude::named_struct;
        use datafusion_common::DFSchema;

        // Get data types from the configured scale
        let domain_type = self.configured.config.domain.data_type();
        let range_type = self.configured.config.range.data_type();
        let empty_schema = DFSchema::empty();

        // Build options struct - convert avenger_scales::Scalar to expressions
        let options_expr = if self.configured.config.options.is_empty() {
            // Create empty struct
            lit(ScalarValue::Struct(
                datafusion::arrow::array::StructArray::new_empty_fields(1, None).into(),
            ))
        } else {
            // Convert HashMap<String, Scalar> to named_struct expression
            let struct_args: Vec<Expr> = self
                .configured
                .config
                .options
                .iter()
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
            range_type.clone(),
            options_type,
        )?;

        // Convert arrays to ScalarValue::List for the UDF call
        let domain_scalar = array_to_list_scalar(self.configured.config.domain.clone())?;
        let range_scalar = array_to_list_scalar(self.configured.config.range.clone())?;

        // Cast input to match domain type if needed
        let casted_input = datafusion::logical_expr::cast(input, domain_type.clone());

        // Call the UDF with domain, range, options, and input
        Ok(udf.call(vec![
            lit(domain_scalar),
            lit(range_scalar),
            options_expr,
            casted_input,
        ]))
    }

    fn to_expr_with_band(&self, input: Expr, band: f64) -> Result<Expr, AvengerChartError> {
        // Band parameter controls position within a band for band/point scales:
        // - 0.0 = start of band
        // - 0.5 = center of band (default)
        // - 1.0 = end of band
        // For non-band scales, this parameter is ignored
        if self
            .configured
            .scale_impl
            .option_definitions()
            .iter()
            .any(|def| def.name == "band")
        {
            // Clone config and add band option
            let mut config = self.configured.config.clone();
            config.options.insert(
                "band".to_string(),
                avenger_scales::scalar::Scalar::from_f32(band as f32),
            );

            // Create a temporary ConfiguredScale with the band option
            let temp_configured = ConfiguredScale {
                scale_impl: self.configured.scale_impl.clone(),
                config,
            };

            // Create a new wrapper with the modified configured scale
            let temp_wrapper = ConfiguredScaleWithSpec::with_range_binding(
                self.spec().clone(),
                temp_configured,
                self.range_binding(),
            );

            temp_wrapper.to_expr(input)
        } else {
            // For non-band scales, ignore the band parameter
            self.to_expr(input)
        }
    }
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
