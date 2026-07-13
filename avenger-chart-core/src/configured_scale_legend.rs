//! Legend-facing helpers for configured scales.

use avenger_scales::scales::{ConfiguredScale, DomainKind, RangeKind};
use datafusion::{
    arrow::{
        array::{Array, Float32Array, ListArray},
        datatypes::Float32Type,
    },
    common::ScalarValue,
};

use crate::{AvengerChartError, ScalarValueHelpers};

/// Extension trait for legend generation
pub trait ConfiguredScaleLegendExt {
    /// Extract domain values as ScalarValues for legend generation
    fn domain_values(&self) -> Result<DomainValues, AvengerChartError>;

    /// Get formatted domain labels for legends
    fn domain_labels(&self) -> Result<Vec<String>, AvengerChartError>;

    /// Extract color values from range if this is a color scale
    fn range_colors(&self) -> Result<Vec<[f32; 4]>, AvengerChartError>;

    /// Extract shape names from the scale's range
    fn range_strings(&self) -> Result<Vec<String>, AvengerChartError>;

    /// Map scalar values through the scale to get numeric values
    fn scale_scalars_to_numeric(
        &self,
        values: &[ScalarValue],
    ) -> Result<Vec<f32>, AvengerChartError>;

    /// Map scalar values through the scale to get color values
    fn scale_scalars_to_colors(
        &self,
        values: &[ScalarValue],
    ) -> Result<Vec<[f32; 4]>, AvengerChartError>;

    /// Map domain values to dash patterns
    fn scale_scalars_to_dash_patterns(&self, values: &[ScalarValue]) -> Vec<Option<Vec<f32>>>;
}

/// Domain values extracted for legend generation
#[derive(Debug, Clone)]
pub enum DomainValues {
    /// Discrete domain values (for ordinal/band/point scales)
    Discrete(Vec<ScalarValue>),
    /// Interval domain with min and max (for continuous scales)
    Interval(ScalarValue, ScalarValue),
}

impl ConfiguredScaleLegendExt for ConfiguredScale {
    fn domain_values(&self) -> Result<DomainValues, AvengerChartError> {
        // First check if scale provides custom legend entries
        if let Some(entries) = self.scale_impl.legend_entries(&self.config) {
            // Convert Scalar to ScalarValue for each entry
            let values: Vec<ScalarValue> = entries
                .iter()
                .map(|e| ScalarValue::try_from_array(&e.representative_value.0, 0))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(DomainValues::Discrete(values));
        }

        // Use DomainKind and RangeKind to determine extraction method
        let domain_array = &self.config.domain;
        let domain_kind = self.scale_impl.domain_kind();
        let range_kind = self.scale_impl.range_kind();

        match (domain_kind, range_kind) {
            // Categorical domains always have discrete values
            (DomainKind::Categorical, _) => {
                let scalars: Vec<ScalarValue> = (0..domain_array.len())
                    .map(|i| ScalarValue::try_from_array(domain_array, i))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(DomainValues::Discrete(scalars))
            }
            // Nested categorical domains are positional geometry, not legend domains.
            (DomainKind::NestedCategorical, _) => Ok(DomainValues::Discrete(Vec::new())),
            // Numeric/Temporal domains with continuous ranges use intervals
            (DomainKind::Numeric | DomainKind::Temporal, RangeKind::Continuous) => {
                if domain_array.len() >= 2 {
                    let min = ScalarValue::try_from_array(domain_array, 0)?;
                    let max = ScalarValue::try_from_array(domain_array, domain_array.len() - 1)?;
                    Ok(DomainValues::Interval(min, max))
                } else {
                    Err(AvengerChartError::InternalError(format!(
                        "Invalid domain for scale: expected at least 2 elements, got {}",
                        domain_array.len()
                    )))
                }
            }
            // Numeric domains with discrete ranges extract all values
            // (though typically these will have custom legend entries)
            (DomainKind::Numeric, RangeKind::Discrete) => {
                let scalars: Vec<ScalarValue> = (0..domain_array.len())
                    .map(|i| ScalarValue::try_from_array(domain_array, i))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(DomainValues::Discrete(scalars))
            }
            // Temporal domains with discrete ranges (shouldn't happen in practice)
            (DomainKind::Temporal, RangeKind::Discrete) => {
                let scalars: Vec<ScalarValue> = (0..domain_array.len())
                    .map(|i| ScalarValue::try_from_array(domain_array, i))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(DomainValues::Discrete(scalars))
            }
        }
    }

    fn domain_labels(&self) -> Result<Vec<String>, AvengerChartError> {
        // First check if scale provides custom legend entries with labels
        if let Some(entries) = self.scale_impl.legend_entries(&self.config) {
            return Ok(entries.into_iter().map(|e| e.label).collect());
        }

        // Otherwise format domain values using the scale's formatter
        let domain_values = self.domain_values()?;
        Ok(match domain_values {
            DomainValues::Discrete(values) => {
                // Convert ScalarValues to arrow array and format
                let array = ScalarValue::iter_to_array(values.iter().cloned())?;
                let formatted = self.format(&array)?;
                formatted.as_vec(array.len(), None)
            }
            DomainValues::Interval(min, max) => {
                // Format interval bounds
                let array = ScalarValue::iter_to_array(vec![min, max])?;
                let formatted = self.format(&array)?;
                formatted.as_vec(2, None)
            }
        })
    }

    fn range_colors(&self) -> Result<Vec<[f32; 4]>, AvengerChartError> {
        use avenger_scales::scales::DomainKind;
        use datafusion::arrow::datatypes::DataType;

        // For ordinal scales (categorical domain), return colors matching domain length with wrapping
        if self.scale_impl.domain_kind() == DomainKind::Categorical {
            let domain_len = self.config.domain.len();
            let range_len = self.config.range.len();

            if domain_len == 0 || range_len == 0 {
                return Ok(vec![]);
            }

            // Extract all range colors first
            let all_colors = match self.config.range.data_type() {
                DataType::Utf8 => {
                    use datafusion::arrow::array::StringArray;
                    let string_array = self
                        .config
                        .range
                        .as_any()
                        .downcast_ref::<StringArray>()
                        .ok_or_else(|| {
                            AvengerChartError::InternalError(format!(
                                "Expected StringArray for ordinal color scale range, got {:?}",
                                self.config.range.data_type()
                            ))
                        })?;

                    let mut colors = Vec::new();
                    for i in 0..string_array.len() {
                        if !string_array.is_null(i) {
                            let color_str = string_array.value(i);
                            if let Some(color) = parse_color_to_rgba(color_str) {
                                colors.push(color);
                            }
                        }
                    }
                    colors
                }
                _ => {
                    return Err(AvengerChartError::InternalError(
                        "Ordinal scale expected string color range".to_string(),
                    ));
                }
            };

            // Return exactly domain_len colors, wrapping if necessary
            let mut result = Vec::with_capacity(domain_len);
            for i in 0..domain_len {
                result.push(all_colors[i % all_colors.len()]);
            }
            Ok(result)
        } else {
            // For non-ordinal scales, return all range colors
            match self.config.range.data_type() {
                // List of color arrays (continuous color scales)
                DataType::List(_) => {
                    // Extract colors from list array
                    let list_array = self
                        .config
                        .range
                        .as_any()
                        .downcast_ref::<ListArray>()
                        .ok_or_else(|| {
                            AvengerChartError::InternalError(format!(
                                "Expected ListArray for color range, got {:?}",
                                self.config.range.data_type()
                            ))
                        })?;

                    let mut colors = Vec::new();
                    for i in 0..list_array.len() {
                        if let Some(color_array) =
                            list_array.value(i).as_any().downcast_ref::<Float32Array>()
                            && color_array.len() >= 4
                        {
                            colors.push([
                                color_array.value(0),
                                color_array.value(1),
                                color_array.value(2),
                                color_array.value(3),
                            ]);
                        }
                    }
                    Ok(colors)
                }
                // String array (for discrete scales like threshold/quantize/quantile with hex colors)
                DataType::Utf8 => {
                    use datafusion::arrow::array::StringArray;

                    let string_array = self
                        .config
                        .range
                        .as_any()
                        .downcast_ref::<StringArray>()
                        .ok_or_else(|| {
                            AvengerChartError::InternalError(format!(
                                "Expected StringArray for discrete color scale range, got {:?}",
                                self.config.range.data_type()
                            ))
                        })?;

                    let mut colors = Vec::new();
                    for i in 0..string_array.len() {
                        if !string_array.is_null(i) {
                            let color_str = string_array.value(i);
                            // Parse hex color to RGBA
                            if let Some(color) = parse_color_to_rgba(color_str) {
                                colors.push(color);
                            }
                        }
                    }
                    Ok(colors)
                }
                _ => Err(AvengerChartError::InternalError(format!(
                    "Cannot extract colors from range with data type: {:?}",
                    self.config.range.data_type()
                ))),
            }
        }
    }

    fn range_strings(&self) -> Result<Vec<String>, AvengerChartError> {
        use avenger_scales::scales::DomainKind;

        // For ordinal scales, return strings matching domain length with wrapping
        if self.scale_impl.domain_kind() == DomainKind::Categorical {
            let domain_len = self.config.domain.len();
            let range_len = self.config.range.len();

            if domain_len == 0 || range_len == 0 {
                return Ok(vec![]);
            }

            // Extract all range strings first
            let mut all_strings = Vec::new();
            for i in 0..range_len {
                all_strings
                    .push(ScalarValue::try_from_array(&self.config.range, i)?.as_scalar_string()?);
            }

            // Return exactly domain_len strings, wrapping if necessary
            let mut result = Vec::with_capacity(domain_len);
            for i in 0..domain_len {
                result.push(all_strings[i % all_strings.len()].clone());
            }
            Ok(result)
        } else {
            // For non-ordinal scales, return all range strings
            let mut shapes = Vec::new();
            for i in 0..self.config.range.len() {
                shapes
                    .push(ScalarValue::try_from_array(&self.config.range, i)?.as_scalar_string()?);
            }
            Ok(shapes)
        }
    }

    fn scale_scalars_to_numeric(
        &self,
        values: &[ScalarValue],
    ) -> Result<Vec<f32>, AvengerChartError> {
        use datafusion::arrow::array::AsArray;
        use datafusion::arrow::compute::cast;
        use datafusion::arrow::datatypes::DataType;

        // Convert scalar values to an arrow array
        let domain_array = ScalarValue::iter_to_array(values.iter().cloned())?;

        // Apply the scale transformation using the configured scale
        let scaled_array = self.scale(&domain_array)?;

        // Cast to Float32Array
        let float_array = cast(&scaled_array, &DataType::Float32).map_err(|e| {
            AvengerChartError::InternalError(format!(
                "Failed to cast scale result to Float32: {}",
                e
            ))
        })?;

        // Extract the values
        if let Some(float_array) = float_array.as_primitive_opt::<Float32Type>() {
            Ok(float_array.values().to_vec())
        } else {
            Err(AvengerChartError::InternalError(
                "Failed to extract float values from scale result".to_string(),
            ))
        }
    }

    fn scale_scalars_to_colors(
        &self,
        values: &[ScalarValue],
    ) -> Result<Vec<[f32; 4]>, AvengerChartError> {
        use datafusion::arrow::datatypes::DataType;

        // Always use scale evaluation - no special cases
        // This ensures all scale types work consistently
        let domain_array = ScalarValue::iter_to_array(values.iter().cloned())?;
        let scaled_array = self.scale(&domain_array)?;

        // Extract colors based on array type
        let data_type = scaled_array.data_type();
        match data_type {
            DataType::FixedSizeList(_, 4) => {
                // Color array - extract RGBA values
                use datafusion::arrow::array::FixedSizeListArray;

                let list_array = scaled_array
                    .as_any()
                    .downcast_ref::<FixedSizeListArray>()
                    .ok_or_else(|| {
                        AvengerChartError::InternalError("Expected FixedSizeListArray".to_string())
                    })?;

                let mut colors = Vec::new();
                for i in 0..list_array.len() {
                    if let Ok(ScalarValue::FixedSizeList(list)) =
                        ScalarValue::try_from_array(&scaled_array, i)
                    {
                        // Extract the array from the Arc
                        if list.len() == 4 {
                            let mut rgba = [0.0f32; 4];
                            // Try to get float values from the array
                            for (j, item) in rgba.iter_mut().enumerate() {
                                if let Ok(ScalarValue::Float32(Some(v))) =
                                    ScalarValue::try_from_array(list.as_ref(), j)
                                {
                                    *item = v;
                                }
                            }
                            colors.push(rgba);
                        }
                    }
                }
                Ok(colors)
            }
            DataType::Utf8 => {
                // String output (hex colors) - parse to RGBA
                use datafusion::arrow::array::StringArray;

                let string_array = scaled_array
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .ok_or_else(|| {
                        AvengerChartError::InternalError(
                            "Expected StringArray for color output".to_string(),
                        )
                    })?;

                let mut colors = Vec::new();
                for i in 0..string_array.len() {
                    if !string_array.is_null(i) {
                        let color_str = string_array.value(i);
                        if let Some(color) = parse_color_to_rgba(color_str) {
                            colors.push(color);
                        } else {
                            // Default color if parsing fails
                            colors.push([0.0, 0.0, 0.0, 1.0]);
                        }
                    } else {
                        // Default color for null values
                        colors.push([0.0, 0.0, 0.0, 1.0]);
                    }
                }
                Ok(colors)
            }
            DataType::Dictionary(_, value_type) => {
                // Dictionary arrays (e.g., from ordinal scales)
                // Process based on the value type
                match value_type.as_ref() {
                    DataType::Utf8 => {
                        // String values (hex colors)
                        // Process each entry in the dictionary array
                        let mut colors = Vec::new();
                        for i in 0..scaled_array.len() {
                            let color_str = match ScalarValue::try_from_array(&scaled_array, i) {
                                Ok(ScalarValue::Dictionary(_, value)) => {
                                    // Extract string from dictionary scalar
                                    if let ScalarValue::Utf8(Some(s)) = value.as_ref() {
                                        Some(s.clone())
                                    } else {
                                        None
                                    }
                                }
                                Ok(ScalarValue::Utf8(Some(s))) => Some(s),
                                _ => None,
                            };

                            if let Some(color_str) = color_str {
                                if let Some(color) = parse_color_to_rgba(&color_str) {
                                    colors.push(color);
                                } else {
                                    colors.push([0.0, 0.0, 0.0, 1.0]);
                                }
                            } else {
                                colors.push([0.0, 0.0, 0.0, 1.0]);
                            }
                        }
                        Ok(colors)
                    }
                    _ => Err(AvengerChartError::InternalError(format!(
                        "Unsupported dictionary value type for colors: {:?}",
                        value_type
                    ))),
                }
            }
            _ => Err(AvengerChartError::InternalError(format!(
                "Scale returned unexpected data type for colors: {:?}",
                data_type
            ))),
        }
    }

    fn scale_scalars_to_dash_patterns(&self, values: &[ScalarValue]) -> Vec<Option<Vec<f32>>> {
        // For ordinal scales, map each value through the scale to get its dash pattern
        // We need to evaluate the scale for each value to get the correct range value

        use datafusion::arrow::array::Array;
        use datafusion::arrow::compute::cast;
        use datafusion::arrow::datatypes::DataType;

        // Convert ScalarValues to an Arrow array
        let array = ScalarValue::iter_to_array(values.to_vec()).ok();
        if let Some(array) = array {
            // Cast to string if needed (ordinal scales expect string inputs)
            let string_array = if array.data_type() != &DataType::Utf8 {
                cast(&array, &DataType::Utf8).unwrap_or(array)
            } else {
                array
            };

            // Use the scale method to map domain values to range values
            if let Ok(result) = self.scale(&string_array) {
                // The result should be a string array with dash pattern names
                let mut patterns = Vec::new();
                for i in 0..result.len() {
                    if let Ok(value) = ScalarValue::try_from_array(&result, i) {
                        let pattern_name = match value {
                            ScalarValue::Utf8(Some(name)) => Some(name),
                            ScalarValue::Dictionary(_, v) => {
                                // Extract string from dictionary scalar
                                if let ScalarValue::Utf8(Some(name)) = v.as_ref() {
                                    Some(name.clone())
                                } else {
                                    None
                                }
                            }
                            _ => {
                                // If the result is not a string, return None
                                None
                            }
                        };

                        if let Some(name) = pattern_name {
                            patterns.push(parse_dash_pattern(&name));
                        } else {
                            patterns.push(None);
                        }
                    } else {
                        patterns.push(None);
                    }
                }
                return patterns;
            }
        }

        // Fallback: return None for all values
        vec![None; values.len()]
    }
}

/// Parse a color string (hex or named) to RGBA array
fn parse_color_to_rgba(color_str: &str) -> Option<[f32; 4]> {
    use avenger_color::ColorOrGradient;
    use avenger_scales::scales::coerce::Coercer;
    use datafusion_common::ScalarValue;

    let coercer = Coercer::default();
    let array = ScalarValue::iter_to_array(
        [ScalarValue::Utf8(Some(color_str.to_string()))]
            .iter()
            .cloned(),
    )
    .ok()?;

    coercer
        .to_color(&array, None)
        .ok()
        .and_then(|colors| colors.as_vec(1, None).first().cloned())
        .and_then(|color| {
            // Extract RGBA from ColorOrGradient
            if let ColorOrGradient::Color(rgba) = color {
                Some(rgba)
            } else {
                None
            }
        })
}

/// Parse a dash pattern name to actual dash array
fn parse_dash_pattern(pattern_name: &str) -> Option<Vec<f32>> {
    // Based on the patterns in avenger-scales Coercer::to_stroke_dash
    match pattern_name {
        "solid" => Some(vec![]),
        "dashed" => Some(vec![8.0, 4.0]),
        "dotted" => Some(vec![2.0, 4.0]),
        "longdash" | "long-dash" => Some(vec![14.0, 4.0]),
        "dashdot" | "dash-dot" => Some(vec![9.0, 4.0, 1.0, 4.0, 1.0, 4.0]),
        "longshort" | "long-short" => Some(vec![11.0, 4.0, 2.0, 4.0]),
        "tripledot" | "triple-dot" => Some(vec![1.0, 1.0, 1.0, 1.0, 4.0]),
        "morsedot" | "morse-dot" => Some(vec![1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 10.0]),
        "doubledash" | "double-dash" => Some(vec![5.0, 4.0, 5.0, 4.0, 5.0, 4.0, 5.0]),
        "evenshort" | "even-short" => Some(vec![6.0, 4.0, 1.0, 4.0, 2.0, 4.0, 1.0, 4.0]),
        "densedash" | "dense-dash" => Some(vec![4.0, 4.0]),
        _ => {
            // Try to parse as comma-separated numbers
            let cleaned = pattern_name.replace(',', " ");
            let parts: Vec<f32> = cleaned
                .split_whitespace()
                .filter_map(|s| s.parse().ok())
                .collect();
            if !parts.is_empty() { Some(parts) } else { None }
        }
    }
}
