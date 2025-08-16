use avenger_scales::scales::ScaleImpl;
use datafusion::logical_expr::{Expr, lit};
use std::collections::HashMap;
use std::sync::Arc;

// Import all scale types
use avenger_scales::scales::{
    band::BandScale, linear::LinearScale, log::LogScale, ordinal::OrdinalScale, point::PointScale,
    pow::PowScale, quantile::QuantileScale, quantize::QuantizeScale, symlog::SymlogScale,
    threshold::ThresholdScale, time::TimeScale,
};

/// Create a scale implementation based on scale type name
pub fn create_scale_impl(scale_type: &str) -> Arc<dyn ScaleImpl> {
    match scale_type {
        "linear" => Arc::new(LinearScale),
        "log" | "logarithmic" => Arc::new(LogScale),
        "pow" | "power" => Arc::new(PowScale),
        "sqrt" => {
            // sqrt is pow with exponent 0.5, but PowScale will handle this via options
            Arc::new(PowScale)
        }
        "symlog" => Arc::new(SymlogScale),
        "time" | "temporal" => Arc::new(TimeScale),
        "band" => Arc::new(BandScale),
        "point" => Arc::new(PointScale),
        "ordinal" => Arc::new(OrdinalScale),
        "threshold" => Arc::new(ThresholdScale),
        "quantile" => Arc::new(QuantileScale),
        "quantize" => Arc::new(QuantizeScale),
        _ => {
            eprintln!("Unknown scale type '{}', defaulting to linear", scale_type);
            Arc::new(LinearScale)
        }
    }
}

/// Apply default options for each scale type
pub fn apply_scale_defaults(scale_type: &str, options: &mut HashMap<String, Expr>) {
    match scale_type {
        "linear" => {
            // Don't set round by default - it should only be set for positional scales
            // Color scales need continuous values, not rounded integers
        }
        "band" => {
            options
                .entry("padding_inner".to_string())
                .or_insert(lit(0.1));
            options.entry("padding".to_string()).or_insert(lit(0.1));
            options.entry("align".to_string()).or_insert(lit(0.5));
            options.entry("round".to_string()).or_insert(lit(true));
        }
        "point" => {
            options.entry("padding".to_string()).or_insert(lit(0.5));
            options.entry("align".to_string()).or_insert(lit(0.5));
            options.entry("round".to_string()).or_insert(lit(true));
        }
        "log" | "logarithmic" => {
            options.entry("base".to_string()).or_insert(lit(10.0));
        }
        "pow" | "power" => {
            options.entry("exponent".to_string()).or_insert(lit(1.0));
        }
        "sqrt" => {
            // sqrt is pow with exponent 0.5
            options.insert("exponent".to_string(), lit(0.5));
        }
        "symlog" => {
            options.entry("constant".to_string()).or_insert(lit(1.0));
        }
        "time" | "temporal" => {
            // Time scales might benefit from rounding for positional use
            // but not for color mapping
        }
        _ => {
            // No specific defaults for other scale types
        }
    }
}