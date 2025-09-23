//! Scale type specifications for compile-time type safety

use avenger_scales::scales::{DomainKind, RangeKind, ScaleImpl};
use std::collections::HashMap;
use std::sync::Arc;

/// Marker trait for scale types
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
#[derive(Debug, Clone, Copy, Default)]
pub struct Linear;

/// Logarithmic scale marker type
#[derive(Debug, Clone, Copy, Default)]
pub struct Log;

/// Power scale marker type
#[derive(Debug, Clone, Copy, Default)]
pub struct Pow;

/// Square root scale marker type
#[derive(Debug, Clone, Copy, Default)]
pub struct Sqrt;

/// Symmetric log scale marker type
#[derive(Debug, Clone, Copy, Default)]
pub struct Symlog;

/// Time scale marker type
#[derive(Debug, Clone, Copy, Default)]
pub struct Time;

/// Band scale marker type
#[derive(Debug, Clone, Copy, Default)]
pub struct Band;

/// Point scale marker type
#[derive(Debug, Clone, Copy, Default)]
pub struct Point;

/// Ordinal scale marker type
#[derive(Debug, Clone, Copy, Default)]
pub struct Ordinal;

/// Threshold scale marker type
#[derive(Debug, Clone, Copy, Default)]
pub struct Threshold;

/// Quantile scale marker type
#[derive(Debug, Clone, Copy, Default)]
pub struct Quantile;

/// Quantize scale marker type
#[derive(Debug, Clone, Copy, Default)]
pub struct Quantize;

/// Auto scale marker type (for automatic type inference)
#[derive(Debug, Clone, Copy, Default)]
pub struct Auto;

// ===== ScaleSpec implementations =====

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
