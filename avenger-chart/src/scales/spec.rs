//! Scale type specifications for compile-time type safety

use std::{collections::HashMap, sync::Arc};

use avenger_scales::scales::{DomainKind, RangeKind, ScaleImpl};
use serde::{Deserialize, Serialize};

/// Marker trait for scale types
#[typetag::serde(tag = "type")]
pub trait ScaleSpec: std::fmt::Debug + Send + Sync + 'static {
    /// Clone this scale spec into a new boxed instance
    fn clone_box(&self) -> Box<dyn ScaleSpec>;

    /// Create the scale implementation for this type
    fn create_impl(&self) -> Arc<dyn ScaleImpl>;

    /// Get the name of this scale type
    fn name(&self) -> &'static str;

    /// Get default options for this scale type
    /// Returns a map of option name to scalar value
    fn default_options(&self) -> HashMap<String, avenger_scales::scalar::Scalar> {
        HashMap::new()
    }

    /// Get the domain kind for this scale type
    fn domain_kind(&self) -> DomainKind {
        self.create_impl().domain_kind()
    }

    /// Get the range kind for this scale type
    fn range_kind(&self) -> RangeKind {
        self.create_impl().range_kind()
    }
}

// Implement Clone for Box<dyn ScaleSpec>
impl Clone for Box<dyn ScaleSpec> {
    fn clone(&self) -> Self {
        self.clone_box()
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

/// Auto scale marker type (for automatic type inference)
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Auto;

// ===== ScaleSpec implementations =====

#[typetag::serde]
impl ScaleSpec for Linear {
    fn clone_box(&self) -> Box<dyn ScaleSpec> {
        Box::new(self.clone())
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
        Box::new(self.clone())
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
        Box::new(self.clone())
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
        Box::new(self.clone())
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
        Box::new(self.clone())
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
        Box::new(self.clone())
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
        Box::new(self.clone())
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
impl ScaleSpec for Point {
    fn clone_box(&self) -> Box<dyn ScaleSpec> {
        Box::new(self.clone())
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
        Box::new(self.clone())
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
        Box::new(self.clone())
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
        Box::new(self.clone())
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
        Box::new(self.clone())
    }

    fn create_impl(&self) -> Arc<dyn ScaleImpl> {
        use avenger_scales::scales::quantize::QuantizeScale;
        Arc::new(QuantizeScale)
    }

    fn name(&self) -> &'static str {
        "quantize"
    }
}

#[typetag::serde]
impl ScaleSpec for Auto {
    fn clone_box(&self) -> Box<dyn ScaleSpec> {
        Box::new(self.clone())
    }

    fn create_impl(&self) -> Arc<dyn ScaleImpl> {
        unimplemented!("Auto scale type does not have a direct implementation");
    }

    fn name(&self) -> &'static str {
        "auto"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sqrt_scale_default_options() {
        // Test that Sqrt scale has exponent = 0.5 as default
        let sqrt = Sqrt::default();
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
        let linear = Linear::default();
        let options = linear.default_options();
        assert!(
            options.is_empty(),
            "Linear scale should have no default options"
        );
    }

    #[test]
    fn test_scale_spec_clone_box() {
        // Test that we can clone a Box<dyn ScaleSpec>
        let spec: Box<dyn ScaleSpec> = Box::new(Band::default());
        let cloned = spec.clone();

        // Both should have the same name
        assert_eq!(spec.name(), cloned.name());
        assert_eq!(spec.name(), "band");
    }

    #[test]
    fn test_scale_spec_clone_different_types() {
        // Test cloning different scale types
        let specs: Vec<Box<dyn ScaleSpec>> = vec![
            Box::new(Linear::default()),
            Box::new(Log::default()),
            Box::new(Sqrt::default()),
            Box::new(Band::default()),
            Box::new(Ordinal::default()),
        ];

        for spec in &specs {
            let cloned = spec.clone();
            assert_eq!(spec.name(), cloned.name());
        }
    }
}
