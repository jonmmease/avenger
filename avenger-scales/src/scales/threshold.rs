use std::{collections::HashMap, sync::Arc};

use arrow::{
    array::{ArrayRef, AsArray, DictionaryArray, Float32Array, Int16Array},
    compute::kernels::cast,
    datatypes::{DataType, Float32Type},
};
use lazy_static::lazy_static;

use crate::{error::AvengerScaleError, scalar::Scalar};

use super::{
    ConfiguredScale, DomainKind, InferDomainFromDataMethod, LegendEntry, OptionDefinition,
    RangeKind, ScaleConfig, ScaleContext, ScaleImpl,
};

/// Threshold scale that maps continuous numeric input values to discrete range values
/// based on a set of threshold boundaries.
///
/// The domain must contain threshold values in ascending order. The range must contain
/// n+1 values where n is the number of thresholds. Input values are mapped as follows:
/// - Values < first threshold → first range value
/// - Values >= threshold[i] and < threshold[i+1] → range[i+1]
/// - Values >= last threshold → last range value
///
/// # Config Options
///
/// This scale does not currently support any configuration options.
#[derive(Debug, Clone)]
pub struct ThresholdScale;

impl ThresholdScale {
    pub fn configured(domain: Vec<f32>, range: ArrayRef) -> ConfiguredScale {
        ConfiguredScale {
            scale_impl: Arc::new(Self),
            config: ScaleConfig {
                domain: Arc::new(Float32Array::from(domain)),
                range,
                options: HashMap::new(),
                context: ScaleContext::default(),
            },
        }
    }
}

impl ScaleImpl for ThresholdScale {
    fn scale_type(&self) -> &'static str {
        "threshold"
    }

    fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod {
        InferDomainFromDataMethod::Explicit
    }

    fn domain_kind(&self) -> DomainKind {
        DomainKind::Numeric
    }

    fn range_kind(&self) -> RangeKind {
        RangeKind::Discrete
    }

    fn option_definitions(&self) -> &[OptionDefinition] {
        lazy_static! {
            static ref DEFINITIONS: Vec<OptionDefinition> = vec![
                // Threshold scale supports no custom options currently
                // But default option is allowed for consistency
                OptionDefinition::optional("default", super::OptionConstraint::String),
            ];
        }

        &DEFINITIONS
    }

    fn scale(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        let thresholds = validate_extract_thresholds(&config.domain)?;

        // Validate the range has the correct number of elements
        if config.range.len() != thresholds.len() + 1 {
            return Err(AvengerScaleError::ThresholdDomainMismatch {
                domain_len: thresholds.len(),
                range_len: config.range.len(),
            });
        }

        // Cast input values to Float32
        let values = cast(&values, &DataType::Float32)?;
        let values_array = values.as_primitive::<Float32Type>();

        // Create indices into the range based on thresholds
        let indices = Int16Array::from(
            values_array
                .iter()
                .map(|x| match x {
                    Some(x) => {
                        if x.is_finite() {
                            let idx =
                                match thresholds.binary_search_by(|t| t.partial_cmp(&x).unwrap()) {
                                    Ok(i) => (i + 1) as i16,
                                    Err(i) => i as i16,
                                };
                            Some(idx)
                        } else {
                            None
                        }
                    }
                    None => None,
                })
                .collect::<Vec<_>>(),
        );

        // Create dictionary array with indices pointing to range values
        let dict_array = DictionaryArray::try_new(indices, config.range.clone())?;
        Ok(Arc::new(dict_array) as ArrayRef)
    }

    fn ticks(
        &self,
        config: &ScaleConfig,
        _count: Option<f32>,
    ) -> Result<ArrayRef, AvengerScaleError> {
        // Ticks are the same as the domain values
        Ok(config.domain.clone())
    }

    fn legend_entries(&self, config: &ScaleConfig) -> Option<Vec<LegendEntry>> {
        // Extract threshold values from domain
        let thresholds = match validate_extract_thresholds(&config.domain) {
            Ok(t) => t,
            Err(_) => return None,
        };

        // Use the formatter to format threshold values
        let formatter = &config.context.formatters.number;

        let mut entries = Vec::new();

        // Create n+1 intervals for n thresholds
        for i in 0..=thresholds.len() {
            let (label, repr_value) = if i == 0 {
                // First interval: < first_threshold
                if let Some(&first) = thresholds.first() {
                    let formatted = formatter.format(&[Some(first)], None);
                    (format!("< {}", formatted[0]), first - 1.0)
                } else {
                    continue;
                }
            } else if i == thresholds.len() {
                // Last interval: >= last_threshold
                if let Some(&last) = thresholds.last() {
                    let formatted = formatter.format(&[Some(last)], None);
                    (format!("≥ {}", formatted[0]), last + 1.0)
                } else {
                    continue;
                }
            } else {
                // Middle intervals: between consecutive thresholds
                let prev = thresholds[i - 1];
                let next = thresholds[i];
                let formatted_prev = formatter.format(&[Some(prev)], None);
                let formatted_next = formatter.format(&[Some(next)], None);
                (
                    format!("{} - {}", formatted_prev[0], formatted_next[0]),
                    (prev + next) / 2.0,
                )
            };

            entries.push(LegendEntry {
                label,
                representative_value: Scalar::from(repr_value),
            });
        }

        Some(entries)
    }
}

fn validate_extract_thresholds(domain: &ArrayRef) -> Result<Vec<f32>, AvengerScaleError> {
    // Try to convert range to f32
    let thresholds = cast(&domain, &DataType::Float32).map_err(|_| {
        AvengerScaleError::ScaleOperationNotSupported(
            "threshold scale domain is not numeric".to_string(),
        )
    })?;

    let thresholds = thresholds
        .as_primitive::<Float32Type>()
        .values()
        .iter()
        .cloned()
        .collect::<Vec<_>>();

    // Validate the thresholds are in ascending order
    if !thresholds.windows(2).all(|w| w[0] <= w[1]) {
        return Err(AvengerScaleError::ThresholdsNotAscending(
            thresholds.clone(),
        ));
    }

    Ok(thresholds)
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use super::*;
    use arrow::array::{Float32Array, StringArray};
    use avenger_common::types::ImageAlign;

    #[test]
    fn test_threshold_scale_basic() -> Result<(), AvengerScaleError> {
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![30.0, 70.0])),
            range: Arc::new(Float32Array::from(vec![0.0, 1.0, 2.0])),
            options: HashMap::new(),
            context: ScaleContext::default(),
        };
        let scale = ThresholdScale;

        let values = Arc::new(Float32Array::from(vec![20.0, 50.0, 80.0])) as ArrayRef;
        let result = scale
            .scale_to_numeric(&config, &values)?
            .as_vec(values.len(), None);

        assert_eq!(result, vec![0.0, 1.0, 2.0]);

        Ok(())
    }

    #[test]
    fn test_threshold_scale_enum() -> Result<(), AvengerScaleError> {
        let config = ScaleConfig {
            domain: Arc::new(Float32Array::from(vec![30.0, 70.0])),
            range: Arc::new(StringArray::from(vec!["left", "center", "right"])),
            options: HashMap::new(),
            context: ScaleContext::default(),
        };
        let scale = ThresholdScale;

        let values = Arc::new(Float32Array::from(vec![50.0, 20.0, 80.0])) as ArrayRef;
        let result = scale
            .scale_to_image_align(&config, &values)?
            .as_vec(values.len(), None);

        assert_eq!(
            result,
            vec![ImageAlign::Center, ImageAlign::Left, ImageAlign::Right]
        );

        Ok(())
    }
}
