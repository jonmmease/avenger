//! Default scale options based on usage context

use datafusion::logical_expr::{Expr, lit};
use std::collections::HashMap;

/// Context in which a scale is being used
#[derive(Debug, Clone, PartialEq)]
pub enum ScaleUsageContext {
    /// X-axis position encoding
    XPosition,
    /// Y-axis position encoding  
    YPosition,
    /// Color encoding
    Color,
    /// Size encoding
    Size,
    /// General/unknown usage
    General,
}

/// Get default options for a scale based on its type and usage context
pub fn get_default_scale_options(
    scale_type: &str,
    context: ScaleUsageContext,
) -> HashMap<String, Expr> {
    let mut options = HashMap::new();

    match (scale_type, context) {
        // Linear scale on Y-axis typically wants to include zero
        ("linear", ScaleUsageContext::YPosition) => {
            options.insert("zero".to_string(), lit(true));
            options.insert("nice".to_string(), lit(true));
        }

        // Linear scale on X-axis might not need zero
        ("linear", ScaleUsageContext::XPosition) => {
            options.insert("nice".to_string(), lit(true));
        }

        // Log and pow scales also benefit from nice domains
        ("log" | "pow" | "sqrt", ScaleUsageContext::YPosition | ScaleUsageContext::XPosition) => {
            options.insert("nice".to_string(), lit(true));
        }

        // Band scale defaults for categorical data
        ("band", _) => {
            options.insert("padding_inner".to_string(), lit(0.1));
            options.insert("padding_outer".to_string(), lit(0.1));
            options.insert("align".to_string(), lit(0.5));
        }

        // Ordinal scale defaults
        ("ordinal", _) => {
            // Ordinal scales typically don't need special defaults
        }

        // Point scale defaults (like band but for point marks)
        ("point", _) => {
            options.insert("padding".to_string(), lit(0.5));
            options.insert("align".to_string(), lit(0.5));
        }

        // Default case
        _ => {}
    }

    options
}

/// Apply default options to a scale if they haven't been explicitly set
pub fn apply_defaults_if_not_set(
    current_options: &mut HashMap<String, Expr>,
    defaults: HashMap<String, Expr>,
) {
    for (key, value) in defaults {
        current_options.entry(key).or_insert(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_linear_y_defaults() {
        let options = get_default_scale_options("linear", ScaleUsageContext::YPosition);
        assert_eq!(options.get("zero"), Some(&lit(true)));
        assert_eq!(options.get("nice"), Some(&lit(true)));
    }

    #[test]
    fn test_linear_x_defaults() {
        let options = get_default_scale_options("linear", ScaleUsageContext::XPosition);
        assert_eq!(options.get("zero"), None);
        assert_eq!(options.get("nice"), Some(&lit(true)));
    }

    #[test]
    fn test_band_defaults() {
        let options = get_default_scale_options("band", ScaleUsageContext::XPosition);
        assert_eq!(options.get("padding_inner"), Some(&lit(0.1)));
        assert_eq!(options.get("padding_outer"), Some(&lit(0.1)));
        assert_eq!(options.get("align"), Some(&lit(0.5)));
    }

    #[test]
    fn test_apply_defaults_preserves_existing() {
        let mut options = HashMap::new();
        options.insert("zero".to_string(), lit(false));

        let defaults = get_default_scale_options("linear", ScaleUsageContext::YPosition);
        apply_defaults_if_not_set(&mut options, defaults);

        // Should preserve the existing false value
        assert_eq!(options.get("zero"), Some(&lit(false)));
        // Should add the missing nice option
        assert_eq!(options.get("nice"), Some(&lit(true)));
    }
}
