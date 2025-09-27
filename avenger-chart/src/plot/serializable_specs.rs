//! Serializable versions of scale and axis specifications
//!
//! These types store the essential configuration needed to reconstruct
//! scales and axes after deserialization.

use crate::axis::Axis;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Serializable scale specification that can be reconstructed after deserialization
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SerializableScaleSpec {
    /// The type of scale (e.g., "linear", "ordinal", "band", etc.)
    pub scale_type: String,

    /// Domain configuration
    pub domain: Option<SerializableScaleDomain>,

    /// Range configuration
    pub range: Option<SerializableScaleRange>,

    /// Scale options as JSON values
    pub options: HashMap<String, serde_json::Value>,
}

/// Serializable domain configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SerializableScaleDomain {
    /// Interval domain with min and max expressions
    Interval {
        min: String,  // Expression as string
        max: String,  // Expression as string
    },

    /// Discrete domain values
    Discrete {
        values: Vec<String>,  // Expressions as strings
    },

    /// Domain inferred from data
    Data {
        // We'll store references to data, but actual data is in DataContext
        infer_from_channels: bool,
    },
}

/// Serializable range configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SerializableScaleRange {
    /// Numeric range [start, end]
    Numeric(f64, f64),

    /// Color range
    Color(Vec<String>),  // Color strings

    /// Discrete range values
    Discrete(Vec<String>),

    /// Default range for the scale type
    Default,
}

/// Serializable axis specification
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SerializableAxisSpec {
    /// Axis title
    pub title: Option<String>,

    /// Whether to show grid lines
    pub grid: bool,

    /// Number of ticks
    pub tick_count: Option<usize>,

    /// Tick format string
    pub format: Option<String>,

    /// Additional axis properties
    pub properties: HashMap<String, serde_json::Value>,
}

impl SerializableScaleSpec {
    /// Create from a Scale<Auto> that may contain non-serializable closures
    pub fn from_scale(scale: &crate::scales::Scale<crate::scales::Auto>) -> Self {
        use crate::scales::ScaleDomain;
        use datafusion::logical_expr::Expr;

        // Extract scale type name
        let scale_type = scale.scale_spec
            .as_option()
            .map(|spec| spec.name().to_string())
            .unwrap_or_else(|| "auto".to_string());

        // Extract domain configuration
        let domain = scale.domain.as_option().map(|d| {
            match d {
                ScaleDomain { explicit_domain: Some(exprs), .. } if exprs.len() == 2 => {
                    // Interval domain
                    SerializableScaleDomain::Interval {
                        min: format!("{:?}", exprs[0]),
                        max: format!("{:?}", exprs[1]),
                    }
                }
                ScaleDomain { explicit_domain: Some(exprs), .. } => {
                    // Discrete domain
                    SerializableScaleDomain::Discrete {
                        values: exprs.iter().map(|e| format!("{:?}", e)).collect(),
                    }
                }
                _ => {
                    // Domain from data
                    SerializableScaleDomain::Data {
                        infer_from_channels: true,
                    }
                }
            }
        });

        // Extract range configuration
        let range = scale.range.as_option().map(|r| {
            use crate::scales::ScaleRange;
            match r {
                ScaleRange::Numeric(start, end) => SerializableScaleRange::Numeric(*start, *end),
                ScaleRange::Color(colors) => {
                    // Convert color expressions to strings
                    SerializableScaleRange::Color(colors.iter().map(|c| format!("{:?}", c)).collect())
                }
                ScaleRange::Discrete(values) => {
                    SerializableScaleRange::Discrete(values.iter().map(|v| format!("{:?}", v)).collect())
                }
                _ => SerializableScaleRange::Default,
            }
        });

        // Extract options as JSON values
        let mut options = HashMap::new();
        for (key, expr) in &scale.options {
            // Convert expression to JSON-serializable format
            // For now, just store as string representation
            options.insert(key.clone(), serde_json::Value::String(format!("{:?}", expr)));
        }

        SerializableScaleSpec {
            scale_type,
            domain,
            range,
            options,
        }
    }

    /// Reconstruct a Scale from this specification
    /// Note: This creates a basic scale without closures or complex expressions
    pub fn to_scale(&self) -> Result<crate::scales::Scale<crate::scales::Auto>, crate::error::AvengerChartError> {
        use crate::scales::{Scale, ScaleDomain};
        use datafusion::logical_expr::lit;

        // Start with a default scale
        let mut scale = Scale::new();

        // Set scale type if specified
        // Note: We'll need to handle this differently since we can't create ScaleSpec instances dynamically
        // For now, we'll just store the type name and handle it during scale building

        // Set domain if specified
        if let Some(domain) = &self.domain {
            match domain {
                SerializableScaleDomain::Interval { min, max } => {
                    // For now, use placeholder values
                    // In practice, we'd parse the expression strings
                    scale = scale.domain_interval(lit(0.0), lit(1.0));
                }
                SerializableScaleDomain::Discrete { values } => {
                    // Convert string expressions back to Expr
                    // For now, use literals
                    let exprs: Vec<_> = values.iter().map(|v| lit(v.clone())).collect();
                    scale = scale.domain_discrete(exprs);
                }
                SerializableScaleDomain::Data { .. } => {
                    // Domain will be inferred from data
                }
            }
        }

        // Set range if specified
        if let Some(range) = &self.range {
            use crate::scales::ScaleRange;
            match range {
                SerializableScaleRange::Numeric(start, end) => {
                    scale = scale.range(ScaleRange::Numeric(*start, *end));
                }
                SerializableScaleRange::Color(colors) => {
                    // Convert color strings back to expressions
                    let color_exprs: Vec<_> = colors.iter().map(|c| lit(c.clone())).collect();
                    scale = scale.range(ScaleRange::Color(color_exprs));
                }
                SerializableScaleRange::Discrete(values) => {
                    let value_exprs: Vec<_> = values.iter().map(|v| lit(v.clone())).collect();
                    scale = scale.range(ScaleRange::Discrete(value_exprs));
                }
                SerializableScaleRange::Default => {
                    // Use default range
                }
            }
        }

        // Set options
        for (key, value) in &self.options {
            if let Some(str_val) = value.as_str() {
                // For now, treat all options as string literals
                scale = scale.option(key, lit(str_val));
            }
        }

        Ok(scale)
    }
}

impl SerializableAxisSpec {
    /// Create from an Axis trait object
    pub fn from_axis(axis: &dyn Axis) -> Self {
        // Extract basic properties that all axes should have
        // This is a simplified version - in practice we'd need more comprehensive extraction
        SerializableAxisSpec {
            title: None,  // Would need to extract from axis
            grid: false,  // Would need to extract from axis
            tick_count: None,
            format: None,
            properties: HashMap::new(),
        }
    }

    /// Create a basic axis from this specification
    pub fn to_axis(&self) -> Box<dyn Axis> {
        // Create a default axis and apply properties
        // This would need to be implemented based on the actual Axis types
        Box::new(crate::axis::LinearAxis::new())
    }
}