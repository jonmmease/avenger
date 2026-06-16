//! Built-in chart scale type descriptors.

use std::{collections::HashMap, sync::Arc};

use avenger_chart_core::{ScaleSpec, ScaleTypePreference};
use avenger_scales::scales::ScaleImpl;
use serde::{Deserialize, Serialize};

pub fn scale_spec_for_preference(preference: ScaleTypePreference) -> Box<dyn ScaleSpec> {
    match preference {
        ScaleTypePreference::Linear => Box::new(Linear),
        ScaleTypePreference::Log => Box::new(Log),
        ScaleTypePreference::Pow => Box::new(Pow),
        ScaleTypePreference::Sqrt => Box::new(Sqrt),
        ScaleTypePreference::Symlog => Box::new(Symlog),
        ScaleTypePreference::Time => Box::new(Time),
        ScaleTypePreference::Band => Box::new(Band),
        ScaleTypePreference::NestedBand => Box::new(NestedBand),
        ScaleTypePreference::Point => Box::new(Point),
        ScaleTypePreference::Ordinal => Box::new(Ordinal),
        ScaleTypePreference::Threshold => Box::new(Threshold),
        ScaleTypePreference::Quantile => Box::new(Quantile),
        ScaleTypePreference::Quantize => Box::new(Quantize),
    }
}

// ===== Marker types for each scale =====

/// Linear scale marker type
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Linear;

/// Logarithmic scale marker type
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Log;

/// Power scale marker type
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Pow;

/// Square root scale marker type
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Sqrt;

/// Symmetric log scale marker type
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Symlog;

/// Time scale marker type
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Time;

/// Band scale marker type
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Band;

/// Nested categorical band scale marker type
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct NestedBand;

/// Point scale marker type
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Point;

/// Ordinal scale marker type
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Ordinal;

/// Threshold scale marker type
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Threshold;

/// Quantile scale marker type
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Quantile;

/// Quantize scale marker type
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Quantize;

// ===== ScaleSpec implementations =====

#[typetag::serde]
impl ScaleSpec for Linear {
    fn clone_box(&self) -> Box<dyn ScaleSpec> {
        Box::new(*self)
    }

    fn create_impl(&self) -> Arc<dyn ScaleImpl> {
        use avenger_scales::scales::linear::LinearScale;
        Arc::new(LinearScale)
    }

    fn name(&self) -> &'static str {
        "linear"
    }
}

#[typetag::serde]
impl ScaleSpec for Log {
    fn clone_box(&self) -> Box<dyn ScaleSpec> {
        Box::new(*self)
    }

    fn create_impl(&self) -> Arc<dyn ScaleImpl> {
        use avenger_scales::scales::log::LogScale;
        Arc::new(LogScale)
    }

    fn name(&self) -> &'static str {
        "log"
    }
}

#[typetag::serde]
impl ScaleSpec for Pow {
    fn clone_box(&self) -> Box<dyn ScaleSpec> {
        Box::new(*self)
    }

    fn create_impl(&self) -> Arc<dyn ScaleImpl> {
        use avenger_scales::scales::pow::PowScale;
        Arc::new(PowScale)
    }

    fn name(&self) -> &'static str {
        "pow"
    }
}

#[typetag::serde]
impl ScaleSpec for Sqrt {
    fn clone_box(&self) -> Box<dyn ScaleSpec> {
        Box::new(*self)
    }

    fn create_impl(&self) -> Arc<dyn ScaleImpl> {
        // Sqrt is not a separate scale in avenger_scales, use Pow with exponent 0.5
        use avenger_scales::scales::pow::PowScale;
        Arc::new(PowScale)
    }

    fn name(&self) -> &'static str {
        "sqrt"
    }

    fn default_options(&self) -> HashMap<String, avenger_scales::scalar::Scalar> {
        let mut options = HashMap::new();
        options.insert(
            "exponent".to_string(),
            avenger_scales::scalar::Scalar::from(0.5_f32),
        );
        options
    }
}

#[typetag::serde]
impl ScaleSpec for Symlog {
    fn clone_box(&self) -> Box<dyn ScaleSpec> {
        Box::new(*self)
    }

    fn create_impl(&self) -> Arc<dyn ScaleImpl> {
        use avenger_scales::scales::symlog::SymlogScale;
        Arc::new(SymlogScale)
    }

    fn name(&self) -> &'static str {
        "symlog"
    }
}

#[typetag::serde]
impl ScaleSpec for Time {
    fn clone_box(&self) -> Box<dyn ScaleSpec> {
        Box::new(*self)
    }

    fn create_impl(&self) -> Arc<dyn ScaleImpl> {
        use avenger_scales::scales::time::TimeScale;
        Arc::new(TimeScale)
    }

    fn name(&self) -> &'static str {
        "time"
    }
}

#[typetag::serde]
impl ScaleSpec for Band {
    fn clone_box(&self) -> Box<dyn ScaleSpec> {
        Box::new(*self)
    }

    fn create_impl(&self) -> Arc<dyn ScaleImpl> {
        use avenger_scales::scales::band::BandScale;
        Arc::new(BandScale)
    }

    fn name(&self) -> &'static str {
        "band"
    }
}

#[typetag::serde]
impl ScaleSpec for NestedBand {
    fn clone_box(&self) -> Box<dyn ScaleSpec> {
        Box::new(*self)
    }

    fn create_impl(&self) -> Arc<dyn ScaleImpl> {
        use avenger_scales::scales::nested_band::NestedBandScale;
        Arc::new(NestedBandScale)
    }

    fn name(&self) -> &'static str {
        "nested_band"
    }
}

#[typetag::serde]
impl ScaleSpec for Point {
    fn clone_box(&self) -> Box<dyn ScaleSpec> {
        Box::new(*self)
    }

    fn create_impl(&self) -> Arc<dyn ScaleImpl> {
        use avenger_scales::scales::point::PointScale;
        Arc::new(PointScale)
    }

    fn name(&self) -> &'static str {
        "point"
    }
}

#[typetag::serde]
impl ScaleSpec for Ordinal {
    fn clone_box(&self) -> Box<dyn ScaleSpec> {
        Box::new(*self)
    }

    fn create_impl(&self) -> Arc<dyn ScaleImpl> {
        use avenger_scales::scales::ordinal::OrdinalScale;
        Arc::new(OrdinalScale)
    }

    fn name(&self) -> &'static str {
        "ordinal"
    }
}

#[typetag::serde]
impl ScaleSpec for Threshold {
    fn clone_box(&self) -> Box<dyn ScaleSpec> {
        Box::new(*self)
    }

    fn create_impl(&self) -> Arc<dyn ScaleImpl> {
        use avenger_scales::scales::threshold::ThresholdScale;
        Arc::new(ThresholdScale)
    }

    fn name(&self) -> &'static str {
        "threshold"
    }
}

#[typetag::serde]
impl ScaleSpec for Quantile {
    fn clone_box(&self) -> Box<dyn ScaleSpec> {
        Box::new(*self)
    }

    fn create_impl(&self) -> Arc<dyn ScaleImpl> {
        use avenger_scales::scales::quantile::QuantileScale;
        Arc::new(QuantileScale)
    }

    fn name(&self) -> &'static str {
        "quantile"
    }
}

#[typetag::serde]
impl ScaleSpec for Quantize {
    fn clone_box(&self) -> Box<dyn ScaleSpec> {
        Box::new(*self)
    }

    fn create_impl(&self) -> Arc<dyn ScaleImpl> {
        use avenger_scales::scales::quantize::QuantizeScale;
        Arc::new(QuantizeScale)
    }

    fn name(&self) -> &'static str {
        "quantize"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sqrt_scale_default_options() {
        // Test that Sqrt scale has exponent = 0.5 as default
        let sqrt = Sqrt;
        let options = sqrt.default_options();

        assert!(
            options.contains_key("exponent"),
            "Sqrt scale should have exponent option"
        );

        let exponent = options.get("exponent").unwrap();
        assert_eq!(
            exponent.as_f32().unwrap(),
            0.5,
            "Sqrt scale should have exponent = 0.5"
        );
    }

    #[test]
    fn test_linear_scale_default_options() {
        // Test that Linear scale has no default options
        let linear = Linear;
        let options = linear.default_options();
        assert!(
            options.is_empty(),
            "Linear scale should have no default options"
        );
    }

    #[test]
    fn test_scale_spec_clone_box() {
        // Test that we can clone a Box<dyn ScaleSpec>
        let spec: Box<dyn ScaleSpec> = Box::new(Band);
        let cloned = spec.clone();

        // Both should have the same name
        assert_eq!(spec.name(), cloned.name());
        assert_eq!(spec.name(), "band");
    }

    #[test]
    fn test_scale_spec_clone_different_types() {
        // Test cloning different scale types
        let specs: Vec<Box<dyn ScaleSpec>> = vec![
            Box::new(Linear),
            Box::new(Log),
            Box::new(Sqrt),
            Box::new(Band),
            Box::new(NestedBand),
            Box::new(Ordinal),
        ];

        for spec in &specs {
            let cloned = spec.clone();
            assert_eq!(spec.name(), cloned.name());
        }
    }

    #[test]
    fn nested_band_scale_spec_creates_nested_band_impl() {
        use avenger_scales::scales::DomainKind;

        let spec = NestedBand;
        let scale_impl = spec.create_impl();
        assert_eq!(spec.name(), "nested_band");
        assert_eq!(scale_impl.scale_type(), "nested_band");
        assert_eq!(scale_impl.domain_kind(), DomainKind::NestedCategorical);
    }
}
