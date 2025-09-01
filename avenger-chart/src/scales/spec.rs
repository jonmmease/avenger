//! Scale type specifications for compile-time type safety

use avenger_scales::scales::ScaleImpl;
use std::collections::HashMap;
use std::sync::Arc;

/// Marker trait for scale types
pub trait ScaleSpec: 'static {
    /// Create the scale implementation for this type
    fn create_impl() -> Arc<dyn ScaleImpl>;

    /// Get the name of this scale type
    fn name() -> &'static str;

    /// Get default options for this scale type
    /// Returns a map of option name to scalar value
    fn default_options() -> HashMap<String, avenger_scales::scalar::Scalar> {
        HashMap::new()
    }
}

// ===== Marker types for each scale =====

/// Linear scale marker type
#[derive(Debug, Clone, Copy)]
pub struct Linear;

/// Logarithmic scale marker type
#[derive(Debug, Clone, Copy)]
pub struct Log;

/// Power scale marker type
#[derive(Debug, Clone, Copy)]
pub struct Pow;

/// Square root scale marker type
#[derive(Debug, Clone, Copy)]
pub struct Sqrt;

/// Symmetric log scale marker type
#[derive(Debug, Clone, Copy)]
pub struct Symlog;

/// Time scale marker type
#[derive(Debug, Clone, Copy)]
pub struct Time;

/// Band scale marker type
#[derive(Debug, Clone, Copy)]
pub struct Band;

/// Point scale marker type
#[derive(Debug, Clone, Copy)]
pub struct Point;

/// Ordinal scale marker type
#[derive(Debug, Clone, Copy)]
pub struct Ordinal;

/// Threshold scale marker type
#[derive(Debug, Clone, Copy)]
pub struct Threshold;

/// Quantile scale marker type
#[derive(Debug, Clone, Copy)]
pub struct Quantile;

/// Quantize scale marker type
#[derive(Debug, Clone, Copy)]
pub struct Quantize;

/// Auto scale marker type (for automatic type inference)
#[derive(Debug, Clone, Copy)]
pub struct Auto;

// ===== ScaleSpec implementations =====

impl ScaleSpec for Linear {
    fn create_impl() -> Arc<dyn ScaleImpl> {
        use avenger_scales::scales::linear::LinearScale;
        Arc::new(LinearScale)
    }

    fn name() -> &'static str {
        "linear"
    }
}

impl ScaleSpec for Log {
    fn create_impl() -> Arc<dyn ScaleImpl> {
        use avenger_scales::scales::log::LogScale;
        Arc::new(LogScale)
    }

    fn name() -> &'static str {
        "log"
    }
}

impl ScaleSpec for Pow {
    fn create_impl() -> Arc<dyn ScaleImpl> {
        use avenger_scales::scales::pow::PowScale;
        Arc::new(PowScale)
    }

    fn name() -> &'static str {
        "pow"
    }
}

impl ScaleSpec for Sqrt {
    fn create_impl() -> Arc<dyn ScaleImpl> {
        // Sqrt is not a separate scale in avenger_scales, use Pow with exponent 0.5
        use avenger_scales::scales::pow::PowScale;
        Arc::new(PowScale)
    }

    fn name() -> &'static str {
        "sqrt"
    }
    
    fn default_options() -> HashMap<String, avenger_scales::scalar::Scalar> {
        let mut options = HashMap::new();
        options.insert("exponent".to_string(), avenger_scales::scalar::Scalar::from(0.5_f32));
        options
    }
}

impl ScaleSpec for Symlog {
    fn create_impl() -> Arc<dyn ScaleImpl> {
        use avenger_scales::scales::symlog::SymlogScale;
        Arc::new(SymlogScale)
    }

    fn name() -> &'static str {
        "symlog"
    }
}

impl ScaleSpec for Time {
    fn create_impl() -> Arc<dyn ScaleImpl> {
        use avenger_scales::scales::time::TimeScale;
        Arc::new(TimeScale)
    }

    fn name() -> &'static str {
        "time"
    }
}

impl ScaleSpec for Band {
    fn create_impl() -> Arc<dyn ScaleImpl> {
        use avenger_scales::scales::band::BandScale;
        Arc::new(BandScale)
    }

    fn name() -> &'static str {
        "band"
    }
}

impl ScaleSpec for Point {
    fn create_impl() -> Arc<dyn ScaleImpl> {
        use avenger_scales::scales::point::PointScale;
        Arc::new(PointScale)
    }

    fn name() -> &'static str {
        "point"
    }
}

impl ScaleSpec for Ordinal {
    fn create_impl() -> Arc<dyn ScaleImpl> {
        use avenger_scales::scales::ordinal::OrdinalScale;
        Arc::new(OrdinalScale)
    }

    fn name() -> &'static str {
        "ordinal"
    }
}

impl ScaleSpec for Threshold {
    fn create_impl() -> Arc<dyn ScaleImpl> {
        use avenger_scales::scales::threshold::ThresholdScale;
        Arc::new(ThresholdScale)
    }

    fn name() -> &'static str {
        "threshold"
    }
}

impl ScaleSpec for Quantile {
    fn create_impl() -> Arc<dyn ScaleImpl> {
        use avenger_scales::scales::quantile::QuantileScale;
        Arc::new(QuantileScale)
    }

    fn name() -> &'static str {
        "quantile"
    }
}

impl ScaleSpec for Quantize {
    fn create_impl() -> Arc<dyn ScaleImpl> {
        use avenger_scales::scales::quantize::QuantizeScale;
        Arc::new(QuantizeScale)
    }

    fn name() -> &'static str {
        "quantize"
    }
}

impl ScaleSpec for Auto {
    fn create_impl() -> Arc<dyn ScaleImpl> {
        unimplemented!("Auto scale type does not have a direct implementation");
    }

    fn name() -> &'static str {
        "auto"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sqrt_scale_default_options() {
        // Test that Sqrt scale has exponent = 0.5 as default
        let options = Sqrt::default_options();
        
        assert!(options.contains_key("exponent"), "Sqrt scale should have exponent option");
        
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
        let options = Linear::default_options();
        assert!(options.is_empty(), "Linear scale should have no default options");
    }
}