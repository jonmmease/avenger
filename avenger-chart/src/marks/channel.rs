use datafusion::logical_expr::{ident, Expr};

/// Specifies which scale to use for a channel
#[derive(Debug, Clone)]
pub enum ScaleSpec {
    /// Use default scale (channel name with trailing numbers removed)
    Default,
    /// Use a specific named scale
    Custom(String),
    /// No scaling - use raw values
    None,
}

/// Represents a channel encoding with an expression and scale specification
#[derive(Debug, Clone)]
pub struct ChannelValue {
    /// The expression that provides values for this channel
    pub expr: Expr,
    /// Which scale to use for this channel
    pub scale: ScaleSpec,
}

impl ChannelValue {
    pub fn new(expr: Expr) -> Self {
        Self { 
            expr, 
            scale: ScaleSpec::Default,
        }
    }
    
    pub fn with_scale(expr: Expr, scale: impl Into<String>) -> Self {
        Self {
            expr,
            scale: ScaleSpec::Custom(scale.into()),
        }
    }
    
    /// Disable scaling for this channel
    pub fn no_scale(expr: Expr) -> Self {
        Self {
            expr,
            scale: ScaleSpec::None,
        }
    }
    
    /// Get the scale name for this channel
    pub fn scale_name(&self, channel_name: &str) -> Option<String> {
        match &self.scale {
            ScaleSpec::Default => {
                // Use channel name with trailing numbers removed
                Some(strip_trailing_numbers(channel_name).to_string())
            }
            ScaleSpec::Custom(name) => Some(name.clone()),
            ScaleSpec::None => None,
        }
    }
    
    /// Extract column name if this is a simple column reference
    pub fn as_column_name(&self) -> Option<String> {
        // Check if expr is a simple column identifier
        if let Expr::Column(col) = &self.expr {
            Some(col.name.clone())
        } else {
            None
        }
    }
}

/// Remove trailing numbers from a channel name to get the base scale name
/// e.g., "x1" -> "x", "color2" -> "color", "x" -> "x"
fn strip_trailing_numbers(name: &str) -> &str {
    name.trim_end_matches(char::is_numeric)
}

// Conversions for ergonomic API

impl From<&str> for ChannelValue {
    fn from(column: &str) -> Self {
        Self::new(ident(column))
    }
}

impl From<String> for ChannelValue {
    fn from(column: String) -> Self {
        Self::new(ident(column))
    }
}

impl From<Expr> for ChannelValue {
    fn from(expr: Expr) -> Self {
        Self::new(expr)
    }
}

impl From<(Expr, &str)> for ChannelValue {
    fn from((expr, scale): (Expr, &str)) -> Self {
        Self::with_scale(expr, scale)
    }
}

impl From<(Expr, Option<&str>)> for ChannelValue {
    fn from((expr, scale): (Expr, Option<&str>)) -> Self {
        match scale {
            Some(s) => Self::with_scale(expr, s),
            None => Self::no_scale(expr),
        }
    }
}

impl From<(Expr, String)> for ChannelValue {
    fn from((expr, scale): (Expr, String)) -> Self {
        Self::with_scale(expr, scale)
    }
}

impl From<(Expr, Option<String>)> for ChannelValue {
    fn from((expr, scale): (Expr, Option<String>)) -> Self {
        match scale {
            Some(s) => Self::with_scale(expr, s),
            None => Self::no_scale(expr),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_strip_trailing_numbers() {
        assert_eq!(strip_trailing_numbers("x"), "x");
        assert_eq!(strip_trailing_numbers("x1"), "x");
        assert_eq!(strip_trailing_numbers("x123"), "x");
        assert_eq!(strip_trailing_numbers("color2"), "color");
        assert_eq!(strip_trailing_numbers("foo"), "foo");
    }
}