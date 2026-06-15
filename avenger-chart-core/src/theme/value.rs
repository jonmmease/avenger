//! Theme value types and color utilities

use datafusion_common::ScalarValue;
use serde::{Deserialize, Serialize};

use avenger_color::{AbsoluteColor, ColorSpace};

/// Value types that themes can return
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ThemeValue {
    /// String value (keywords, identifiers, font families, etc.)
    String(String),

    /// Numeric value (all CSS numbers: sizes, opacities, angles, weights, etc.)
    Number(f64),

    /// Boolean value
    Boolean(bool),

    /// Length with unit
    Length(f64, LengthUnit),

    /// Angle with unit (for hue values in HSL, rotations, etc.)
    Angle(f64, AngleUnit),

    /// Percentage value
    Percentage(f64),

    /// Color value (parsed from CSS)
    Color(CssRgba),

    /// CSS function call
    Function(String, Vec<ThemeValue>),

    /// Multiple values (for padding, margin, etc.)
    List(Vec<ThemeValue>),

    /// CSS variable reference
    Variable(String),

    /// light-dark() function - theme-aware color selection
    /// First value for light mode, second for dark mode
    LightDark(Box<ThemeValue>, Box<ThemeValue>),

    /// CSS calc() expression
    /// Stores the AST for later evaluation with context (base font size, runtime params)
    /// Example: calc(var(--base-size) * 2), calc(100% - 20px)
    Calc(Box<super::calc::CalcNode>),

    /// Relative color derived from origin using channel keywords
    /// Example: oklch(from blue calc(l - 0.2) c h)
    /// Syntax: <color-function>(from <origin> <components>)
    RelativeColor {
        space: ColorSpace,
        origin: Box<ThemeValue>,
        lightness: super::color_component::ColorComponent,
        component1: super::color_component::ColorComponent,
        component2: super::color_component::ColorComponent,
        alpha: super::color_component::ColorComponent,
    },

    /// Initial value
    Initial,

    /// Inherited value
    Inherit,

    /// No value (property not set)
    None,
}

/// RGBA color representation from CSS (0-255 range)
///
/// This type represents colors as parsed from CSS, using u8 values (0-255).
/// It converts to normalized [f32; 4] arrays (0.0-1.0) for rendering.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CssRgba {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

impl CssRgba {
    pub fn from_rgba8([red, green, blue, alpha]: [u8; 4]) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }

    /// Convert to normalized RGBA array [0.0-1.0] for use with ColorOrGradient
    pub fn to_array(&self) -> [f32; 4] {
        [
            self.red as f32 / 255.0,
            self.green as f32 / 255.0,
            self.blue as f32 / 255.0,
            self.alpha as f32 / 255.0,
        ]
    }
}

/// Length units
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LengthUnit {
    Px,
    Rem,
}

/// Angle units for CSS angle values
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum AngleUnit {
    /// Degrees (360deg = full circle)
    Deg,
    /// Radians (2π rad ≈ 6.28318 rad = full circle)
    Rad,
    /// Gradians (400grad = full circle)
    Grad,
    /// Turns (1turn = full circle)
    Turn,
}

impl AngleUnit {
    /// Convert angle value to degrees
    pub fn to_degrees(&self, value: f64) -> f64 {
        match self {
            AngleUnit::Deg => value,
            AngleUnit::Rad => value.to_degrees(),
            AngleUnit::Grad => value * 360.0 / 400.0,
            AngleUnit::Turn => value * 360.0,
        }
    }
}

impl ThemeValue {
    /// Try to get as string
    pub fn as_string(&self) -> Option<&str> {
        match self {
            ThemeValue::String(s) => Some(s),
            _ => None,
        }
    }

    /// Extract font-size value with rem support and calc support
    ///
    /// Converts font-size values to pixels:
    /// - Number: Returns the raw number (interpreted as px)
    /// - Length(Px): Returns pixel value
    /// - Length(Rem): Converts rem to pixels using base_font_size (rem-based scaling)
    /// - Calc: Resolves calc expression with runtime params from ScalarValue map
    ///
    /// # Arguments
    /// * `params` - Runtime parameter values (from ThemeContext)
    /// * `base_font_size` - Base font size for rem conversion
    ///
    /// Note: Percentage values are not supported (would require parent element context)
    pub fn as_font_size(
        &self,
        params: &indexmap::IndexMap<String, datafusion::common::ScalarValue>,
        base_font_size: f32,
    ) -> Option<f32> {
        match self {
            ThemeValue::Number(n) => Some(*n as f32),
            ThemeValue::Length(n, LengthUnit::Px) => Some(*n as f32),
            ThemeValue::Length(n, LengthUnit::Rem) => Some((*n as f32) * base_font_size),
            ThemeValue::Calc(calc_node) => {
                // Convert ScalarValue params to f64
                let f64_params = scalar_value_params_to_f64(params);

                // Resolve calc with params
                let resolved = calc_node.resolve(&f64_params, base_font_size).ok()?;
                resolved.as_length_px(base_font_size)
            }
            _ => None,
        }
    }

    /// Try to get as number
    pub fn as_number(&self) -> Option<f64> {
        match self {
            ThemeValue::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// Get angle value in degrees
    /// - Angle values are converted to degrees
    /// - Plain numbers are interpreted as degrees
    pub fn as_angle_degrees(&self) -> Option<f64> {
        match self {
            ThemeValue::Number(n) => Some(*n), // Interpret as degrees
            ThemeValue::Angle(value, unit) => Some(unit.to_degrees(*value)),
            _ => None,
        }
    }

    /// Try to get as boolean
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ThemeValue::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// Try to get as list
    pub fn as_list(&self) -> Option<&[ThemeValue]> {
        match self {
            ThemeValue::List(l) => Some(l),
            _ => None,
        }
    }

    /// Try to get as CSS color
    pub fn as_color(&self) -> Option<CssRgba> {
        match self {
            ThemeValue::Color(rgba) => Some(*rgba),
            _ => None,
        }
    }

    /// Try to get as normalized color array [0.0-1.0] for use with ColorOrGradient
    pub fn as_color_array(&self) -> Option<[f32; 4]> {
        self.as_color().map(|css_rgba| css_rgba.to_array())
    }

    /// Resolve to color with runtime parameters and origin color context
    ///
    /// This handles both absolute colors and relative color syntax.
    /// For relative colors, it resolves the origin color, converts to target space,
    /// evaluates component expressions with channel keywords, and builds the derived color.
    pub fn as_color_with_params(
        &self,
        params: &indexmap::IndexMap<String, datafusion_common::ScalarValue>,
        base_font_size: f32,
    ) -> Option<CssRgba> {
        match self {
            ThemeValue::Color(rgba) => Some(*rgba),

            // Handle CSS variables - look up in params and recursively resolve
            ThemeValue::Variable(name) => {
                use datafusion_common::ScalarValue;

                if let Some(value) = params.get(name) {
                    match value {
                        // String value - could be a color name or hex color
                        ScalarValue::Utf8(Some(s)) | ScalarValue::LargeUtf8(Some(s)) => {
                            parse_color_string(s)
                        }
                        // Already a number - not a color
                        _ => None,
                    }
                } else {
                    None
                }
            }

            // Handle string values that might be color names
            ThemeValue::String(s) => parse_color_string(s),

            ThemeValue::RelativeColor {
                space,
                origin,
                lightness,
                component1,
                component2,
                alpha,
            } => {
                // 1. Resolve origin color recursively
                let origin_rgba = origin.as_color_with_params(params, base_font_size)?;
                let mut origin_abs = AbsoluteColor::from_rgba(origin_rgba.to_array());

                // 2. Convert origin to target color space
                origin_abs = origin_abs.to_color_space(*space);

                // 3. Convert params to f64
                let params_f64 = scalar_value_params_to_f64(params);

                // 4. Resolve each component with origin color context
                let c0 = lightness
                    .resolve(Some(&origin_abs), &params_f64, base_font_size)
                    .ok()? as f32;
                let c1 = component1
                    .resolve(Some(&origin_abs), &params_f64, base_font_size)
                    .ok()? as f32;
                let c2 = component2
                    .resolve(Some(&origin_abs), &params_f64, base_font_size)
                    .ok()? as f32;
                let a = alpha
                    .resolve(Some(&origin_abs), &params_f64, base_font_size)
                    .ok()? as f32;

                // 5. Build derived color in target space
                let derived = AbsoluteColor::new(*space, c0, c1, c2, a);

                // 6. Convert to CssRgba
                Some(CssRgba::from_rgba8(derived.to_rgba8()))
            }

            // Handle light-dark() function - resolve based on color-scheme param
            ThemeValue::LightDark(light, dark) => {
                use datafusion_common::ScalarValue;

                // Check color-scheme param to decide which value to use
                let use_dark = params
                    .get("color-scheme")
                    .and_then(|v| match v {
                        ScalarValue::Utf8(Some(s)) | ScalarValue::LargeUtf8(Some(s)) => {
                            Some(s.as_str() == "dark")
                        }
                        _ => None,
                    })
                    .unwrap_or(false); // Default to light mode if no param

                let selected = if use_dark { dark } else { light };

                // Recursively resolve the selected value
                selected.as_color_with_params(params, base_font_size)
            }

            // Handle function calls that return colors (e.g., contrast-color, color-mix)
            ThemeValue::Function(name, args) => match name.as_str() {
                "contrast-color" => {
                    use crate::theme::contrast_color::resolve_contrast_color_with_params;
                    resolve_contrast_color_with_params(args, params, base_font_size)
                }
                "color-mix" => {
                    use crate::theme::color_mix::resolve_color_mix_with_params;
                    resolve_color_mix_with_params(args, params, base_font_size)
                }
                _ => None,
            },

            _ => None,
        }
    }

    /// Check if value is None
    pub fn is_none(&self) -> bool {
        matches!(self, ThemeValue::None)
    }

    // ===================================================================================
    // Type-directed evaluation methods (new architecture)
    // ===================================================================================
    //
    // These methods provide type-directed evaluation of ThemeValue variants.
    // Unlike the old variant-based pattern matching in mark_default(), these methods:
    // 1. Accept any ThemeValue variant and attempt conversion to the target type
    // 2. Handle nested expressions (variables in calc, functions in variables, etc.)
    // 3. Return detailed error diagnostics when conversion fails
    // 4. Support all variants including Calc, RelativeColor, Percentage, etc.
    //
    // This is the foundation for fixing the systematic gaps in mark_default().

    /// Evaluate as CSS color with full variant support
    ///
    /// This method attempts to convert any ThemeValue to a color (CssRgba).
    /// It handles:
    /// - Direct colors: Color, String (named colors, hex)
    /// - Variables: var() that resolve to colors
    /// - Functions: color-mix(), contrast-color(), light-dark()
    /// - Relative colors: oklch(from blue ...), hsl(from red ...)
    /// - Calc expressions: calc() that evaluate to color strings
    ///
    /// # Errors
    /// Returns EvalError if:
    /// - Variable not found in params
    /// - Variable has wrong type (e.g., number instead of color)
    /// - Function evaluation fails
    /// - Calc expression fails
    /// - Variant cannot be interpreted as color (e.g., Number, Boolean)
    ///
    /// # Example
    /// ```ignore
    /// let ctx = EvalContext::new(&params, 16.0);
    /// let color_value = ThemeValue::Variable("--accent".to_string());
    /// let rgba = color_value.eval_as_color(&ctx)?;
    /// ```
    pub fn eval_as_color(
        &self,
        ctx: &super::eval::EvalContext,
    ) -> Result<CssRgba, super::eval::EvalError> {
        use super::eval::{EvalError, TargetType, variant_name};

        match self {
            // Direct color value
            ThemeValue::Color(rgba) => Ok(*rgba),

            // String that might be a color name or hex
            ThemeValue::String(s) => parse_color_string(s).ok_or_else(|| EvalError::TypeMismatch {
                variant: "String".to_string(),
                expected_type: TargetType::Color,
                value_description: format!("\"{}\" is not a valid color", s),
            }),

            // CSS variable - look up in params and recursively evaluate
            ThemeValue::Variable(name) => {
                use datafusion_common::ScalarValue;

                let scalar_value =
                    ctx.params
                        .get(name)
                        .ok_or_else(|| EvalError::VariableNotFound {
                            variable_name: name.clone(),
                        })?;

                match scalar_value {
                    ScalarValue::Utf8(Some(s)) | ScalarValue::LargeUtf8(Some(s)) => {
                        parse_color_string(s).ok_or_else(|| EvalError::VariableTypeMismatch {
                            variable_name: name.clone(),
                            expected_type: TargetType::Color,
                            actual_type: format!("string \"{}\" (not a valid color)", s),
                        })
                    }
                    _ => Err(EvalError::VariableTypeMismatch {
                        variable_name: name.clone(),
                        expected_type: TargetType::Color,
                        actual_type: format!("{:?}", scalar_value),
                    }),
                }
            }

            // Function calls (color-mix, contrast-color)
            ThemeValue::Function(name, args) => match name.as_str() {
                "color-mix" => {
                    use super::color_mix::resolve_color_mix_with_params;
                    resolve_color_mix_with_params(args, ctx.params, ctx.base_font_size).ok_or_else(
                        || EvalError::FunctionError {
                            function_name: "color-mix".to_string(),
                            error: "Failed to evaluate color-mix()".to_string(),
                        },
                    )
                }
                "contrast-color" => {
                    use super::contrast_color::resolve_contrast_color_with_params;
                    resolve_contrast_color_with_params(args, ctx.params, ctx.base_font_size)
                        .ok_or_else(|| EvalError::FunctionError {
                            function_name: "contrast-color".to_string(),
                            error: "Failed to evaluate contrast-color()".to_string(),
                        })
                }
                _ => Err(EvalError::FunctionError {
                    function_name: name.clone(),
                    error: format!("Unknown color function: {}()", name),
                }),
            },

            // light-dark() - theme-aware color selection
            ThemeValue::LightDark(light, dark) => {
                use datafusion_common::ScalarValue;

                // Check color-scheme param to decide which value to use
                let use_dark = ctx
                    .params
                    .get("color-scheme")
                    .and_then(|v| match v {
                        ScalarValue::Utf8(Some(s)) | ScalarValue::LargeUtf8(Some(s)) => {
                            Some(s.as_str() == "dark")
                        }
                        _ => None,
                    })
                    .unwrap_or(false);

                let selected = if use_dark { dark } else { light };
                selected.eval_as_color(ctx)
            }

            // Relative color syntax: oklch(from blue calc(l - 0.2) c h)
            ThemeValue::RelativeColor {
                space,
                origin,
                lightness,
                component1,
                component2,
                alpha,
            } => {
                // 1. Resolve origin color recursively
                let origin_rgba =
                    origin
                        .eval_as_color(ctx)
                        .map_err(|e| EvalError::RelativeColorError {
                            error: format!("Failed to resolve origin color: {}", e),
                        })?;

                let mut origin_abs = AbsoluteColor::from_rgba(origin_rgba.to_array());

                // 2. Convert origin to target color space
                origin_abs = origin_abs.to_color_space(*space);

                // 3. Convert params to f64
                let params_f64 = scalar_value_params_to_f64(ctx.params);

                // 4. Resolve each component with origin color context
                let c0 = lightness
                    .resolve(Some(&origin_abs), &params_f64, ctx.base_font_size)
                    .map_err(|e| EvalError::RelativeColorError {
                        error: format!("Failed to resolve lightness component: {}", e),
                    })? as f32;

                let c1 = component1
                    .resolve(Some(&origin_abs), &params_f64, ctx.base_font_size)
                    .map_err(|e| EvalError::RelativeColorError {
                        error: format!("Failed to resolve component1: {}", e),
                    })? as f32;

                let c2 = component2
                    .resolve(Some(&origin_abs), &params_f64, ctx.base_font_size)
                    .map_err(|e| EvalError::RelativeColorError {
                        error: format!("Failed to resolve component2: {}", e),
                    })? as f32;

                let a = alpha
                    .resolve(Some(&origin_abs), &params_f64, ctx.base_font_size)
                    .map_err(|e| EvalError::RelativeColorError {
                        error: format!("Failed to resolve alpha: {}", e),
                    })? as f32;

                // 5. Build derived color in target space
                let derived = AbsoluteColor::new(*space, c0, c1, c2, a);

                // 6. Convert to CssRgba
                Ok(CssRgba::from_rgba8(derived.to_rgba8()))
            }

            // Calc expressions that might evaluate to color strings
            // Example: calc(var(--color-primary)) where --color-primary is "#ff0000"
            // Note: Calc expressions for colors are rare but possible if the variable
            // resolves to a color string
            ThemeValue::Calc(_) => {
                // Calc expressions don't typically resolve to colors
                // This would require special handling if needed in the future
                Err(EvalError::TypeMismatch {
                    variant: "Calc".to_string(),
                    expected_type: TargetType::Color,
                    value_description: "calc() expressions cannot be evaluated as colors"
                        .to_string(),
                })
            }

            // Unsupported variants
            ThemeValue::Initial | ThemeValue::Inherit | ThemeValue::None => {
                Err(EvalError::Unsupported {
                    variant: variant_name(self).to_string(),
                    expected_type: TargetType::Color,
                })
            }

            // Type mismatches
            _ => Err(EvalError::TypeMismatch {
                variant: variant_name(self).to_string(),
                expected_type: TargetType::Color,
                value_description: format!("{:?}", self),
            }),
        }
    }

    /// Evaluate as numeric value
    ///
    /// Converts ThemeValue to f64. Handles:
    /// - Number: Direct numeric value
    /// - Variable: var() that resolves to number
    /// - Calc: calc() expressions that evaluate to numbers
    /// - Percentage: Converts percentage to decimal (50% → 0.5)
    /// - Angle: Converts angle to degrees
    ///
    /// # Errors
    /// Returns EvalError for non-numeric variants or evaluation failures
    pub fn eval_as_number(
        &self,
        ctx: &super::eval::EvalContext,
    ) -> Result<f64, super::eval::EvalError> {
        use super::eval::{EvalError, TargetType, variant_name};

        match self {
            ThemeValue::Number(n) => Ok(*n),

            // Percentage as decimal (50% → 0.5)
            ThemeValue::Percentage(p) => Ok(*p),

            // Angle in degrees
            ThemeValue::Angle(value, unit) => Ok(unit.to_degrees(*value)),

            // Variable lookup
            ThemeValue::Variable(name) => {
                use datafusion_common::ScalarValue;

                let scalar_value =
                    ctx.params
                        .get(name)
                        .ok_or_else(|| EvalError::VariableNotFound {
                            variable_name: name.clone(),
                        })?;

                match scalar_value {
                    ScalarValue::Float64(Some(v)) => Ok(*v),
                    ScalarValue::Float32(Some(v)) => Ok(*v as f64),
                    ScalarValue::Int64(Some(v)) => Ok(*v as f64),
                    ScalarValue::Int32(Some(v)) => Ok(*v as f64),
                    ScalarValue::UInt64(Some(v)) => Ok(*v as f64),
                    ScalarValue::UInt32(Some(v)) => Ok(*v as f64),
                    _ => Err(EvalError::VariableTypeMismatch {
                        variable_name: name.clone(),
                        expected_type: TargetType::Number,
                        actual_type: format!("{:?}", scalar_value),
                    }),
                }
            }

            // Calc expression
            ThemeValue::Calc(calc_node) => {
                let params_f64 = scalar_value_params_to_f64(ctx.params);

                let resolved = calc_node
                    .resolve(&params_f64, ctx.base_font_size)
                    .map_err(|e| EvalError::CalcError {
                        error: format!("calc() resolution failed: {}", e),
                    })?;

                // CalcLeaf has as_number() method which extracts Number variant
                resolved.as_number().ok_or_else(|| EvalError::CalcError {
                    error: format!("calc() did not resolve to a number: {:?}", resolved),
                })
            }

            ThemeValue::Initial | ThemeValue::Inherit | ThemeValue::None => {
                Err(EvalError::Unsupported {
                    variant: variant_name(self).to_string(),
                    expected_type: TargetType::Number,
                })
            }

            _ => Err(EvalError::TypeMismatch {
                variant: variant_name(self).to_string(),
                expected_type: TargetType::Number,
                value_description: format!("{:?}", self),
            }),
        }
    }

    /// Evaluate as length in pixels
    ///
    /// Converts ThemeValue to pixel length (f64). Handles:
    /// - Number: Interpreted as pixels
    /// - Length(Px): Direct pixel value
    /// - Length(Rem): Converted using base font size
    /// - Variable: var() that resolves to length
    /// - Calc: calc() expressions with length units
    ///
    /// # Errors
    /// Returns EvalError for non-length variants or evaluation failures
    pub fn eval_as_length(
        &self,
        ctx: &super::eval::EvalContext,
    ) -> Result<f64, super::eval::EvalError> {
        use super::eval::{EvalError, TargetType, variant_name};

        match self {
            ThemeValue::Number(n) => Ok(*n),

            ThemeValue::Length(n, LengthUnit::Px) => Ok(*n),
            ThemeValue::Length(n, LengthUnit::Rem) => Ok(*n * ctx.base_font_size as f64),

            // Variable lookup
            ThemeValue::Variable(name) => {
                use datafusion_common::ScalarValue;

                let scalar_value =
                    ctx.params
                        .get(name)
                        .ok_or_else(|| EvalError::VariableNotFound {
                            variable_name: name.clone(),
                        })?;

                match scalar_value {
                    ScalarValue::Float64(Some(v)) => Ok(*v),
                    ScalarValue::Float32(Some(v)) => Ok(*v as f64),
                    ScalarValue::Int64(Some(v)) => Ok(*v as f64),
                    ScalarValue::Int32(Some(v)) => Ok(*v as f64),
                    ScalarValue::UInt64(Some(v)) => Ok(*v as f64),
                    ScalarValue::UInt32(Some(v)) => Ok(*v as f64),
                    _ => Err(EvalError::VariableTypeMismatch {
                        variable_name: name.clone(),
                        expected_type: TargetType::Length,
                        actual_type: format!("{:?}", scalar_value),
                    }),
                }
            }

            // Calc expression
            ThemeValue::Calc(calc_node) => {
                let params_f64 = scalar_value_params_to_f64(ctx.params);

                let resolved = calc_node
                    .resolve(&params_f64, ctx.base_font_size)
                    .map_err(|e| EvalError::CalcError {
                        error: format!("calc() resolution failed: {}", e),
                    })?;

                // CalcLeaf has as_length_px() method which converts to pixels
                resolved
                    .as_length_px(ctx.base_font_size)
                    .ok_or_else(|| EvalError::CalcError {
                        error: format!("calc() did not resolve to a length: {:?}", resolved),
                    })
                    .map(|v| v as f64)
            }

            ThemeValue::Initial | ThemeValue::Inherit | ThemeValue::None => {
                Err(EvalError::Unsupported {
                    variant: variant_name(self).to_string(),
                    expected_type: TargetType::Length,
                })
            }

            _ => Err(EvalError::TypeMismatch {
                variant: variant_name(self).to_string(),
                expected_type: TargetType::Length,
                value_description: format!("{:?}", self),
            }),
        }
    }

    /// Evaluate as string
    ///
    /// Converts ThemeValue to String. Handles:
    /// - String: Direct string value
    /// - Variable: var() that resolves to string
    /// - Number, Color, etc.: Converted to string representation
    ///
    /// # Errors
    /// Returns EvalError for unsupported variants or evaluation failures
    pub fn eval_as_string(
        &self,
        ctx: &super::eval::EvalContext,
    ) -> Result<String, super::eval::EvalError> {
        use super::eval::{EvalError, TargetType, variant_name};

        match self {
            ThemeValue::String(s) => Ok(s.clone()),

            // Variable lookup
            ThemeValue::Variable(name) => {
                use datafusion_common::ScalarValue;

                let scalar_value =
                    ctx.params
                        .get(name)
                        .ok_or_else(|| EvalError::VariableNotFound {
                            variable_name: name.clone(),
                        })?;

                match scalar_value {
                    ScalarValue::Utf8(Some(s)) | ScalarValue::LargeUtf8(Some(s)) => Ok(s.clone()),
                    _ => Err(EvalError::VariableTypeMismatch {
                        variable_name: name.clone(),
                        expected_type: TargetType::String,
                        actual_type: format!("{:?}", scalar_value),
                    }),
                }
            }

            // Convert other types to strings
            ThemeValue::Number(n) => Ok(n.to_string()),

            ThemeValue::Color(rgba) => {
                if rgba.alpha == 255 {
                    Ok(format!(
                        "#{:02x}{:02x}{:02x}",
                        rgba.red, rgba.green, rgba.blue
                    ))
                } else {
                    let alpha = rgba.alpha as f32 / 255.0;
                    Ok(format!(
                        "rgba({}, {}, {}, {})",
                        rgba.red, rgba.green, rgba.blue, alpha
                    ))
                }
            }

            // Calc expression
            ThemeValue::Calc(calc_node) => {
                let params_f64 = scalar_value_params_to_f64(ctx.params);

                let resolved = calc_node
                    .resolve(&params_f64, ctx.base_font_size)
                    .map_err(|e| EvalError::CalcError {
                        error: format!("calc() resolution failed: {}", e),
                    })?;

                // Convert CalcLeaf to string representation
                match resolved {
                    super::calc::CalcLeaf::Number(n) => Ok(n.to_string()),
                    super::calc::CalcLeaf::Length(n, unit) => Ok(format!(
                        "{}{}",
                        n,
                        match unit {
                            LengthUnit::Px => "px",
                            LengthUnit::Rem => "rem",
                        }
                    )),
                    super::calc::CalcLeaf::Percentage(p) => Ok(format!("{}%", p)),
                    super::calc::CalcLeaf::Angle(a, unit) => Ok(format!(
                        "{}{}",
                        a,
                        match unit {
                            AngleUnit::Deg => "deg",
                            AngleUnit::Rad => "rad",
                            AngleUnit::Grad => "grad",
                            AngleUnit::Turn => "turn",
                        }
                    )),
                    _ => Err(EvalError::CalcError {
                        error: format!("Cannot convert calc result to string: {:?}", resolved),
                    }),
                }
            }

            ThemeValue::Initial | ThemeValue::Inherit | ThemeValue::None => {
                Err(EvalError::Unsupported {
                    variant: variant_name(self).to_string(),
                    expected_type: TargetType::String,
                })
            }

            _ => Err(EvalError::TypeMismatch {
                variant: variant_name(self).to_string(),
                expected_type: TargetType::String,
                value_description: format!("{:?}", self),
            }),
        }
    }
}

/// Parse a color string using avenger-scales color parser (internal use only)
/// Supports hex colors, rgb/rgba, hsl/hsla, and all CSS named colors
pub(crate) fn parse_color_string(color_str: &str) -> Option<CssRgba> {
    avenger_color::parse_color_string(color_str).map(|[r, g, b, a]| CssRgba {
        red: (r * 255.0) as u8,
        green: (g * 255.0) as u8,
        blue: (b * 255.0) as u8,
        alpha: (a * 255.0) as u8,
    })
}

/// Parse a length string like "16px", "1.5rem", "10em" into a ThemeValue::Length
pub(crate) fn parse_length_string(length_str: &str) -> Option<ThemeValue> {
    let s = length_str.trim();

    if let Some(num_str) = s.strip_suffix("px") {
        if let Ok(n) = num_str.parse::<f64>() {
            return Some(ThemeValue::Length(n, LengthUnit::Px));
        }
    } else if let Some(num_str) = s.strip_suffix("rem")
        && let Ok(n) = num_str.parse::<f64>()
    {
        return Some(ThemeValue::Length(n, LengthUnit::Rem));
    }

    None
}

/// Convert ScalarValue parameters to f64 for calc resolution
///
/// This converts DataFusion ScalarValue types to f64 for use in calc expressions.
/// Supports: Int64, UInt64, Float64, Int32, UInt32, Float32
fn scalar_value_params_to_f64(
    params: &indexmap::IndexMap<String, datafusion_common::ScalarValue>,
) -> indexmap::IndexMap<String, f64> {
    params
        .iter()
        .filter_map(|(key, value)| {
            let f64_value = match value {
                ScalarValue::Int64(Some(v)) => Some(*v as f64),
                ScalarValue::UInt64(Some(v)) => Some(*v as f64),
                ScalarValue::Float64(Some(v)) => Some(*v),
                ScalarValue::Int32(Some(v)) => Some(*v as f64),
                ScalarValue::UInt32(Some(v)) => Some(*v as f64),
                ScalarValue::Float32(Some(v)) => Some(*v as f64),
                _ => None,
            };
            f64_value.map(|v| (key.clone(), v))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_angle_unit_to_degrees() {
        // Test degrees (identity)
        assert_eq!(AngleUnit::Deg.to_degrees(180.0), 180.0);
        assert_eq!(AngleUnit::Deg.to_degrees(360.0), 360.0);

        // Test radians
        use std::f64::consts::PI;
        assert!((AngleUnit::Rad.to_degrees(PI) - 180.0).abs() < 0.01);
        assert!((AngleUnit::Rad.to_degrees(2.0 * PI) - 360.0).abs() < 0.01);
        assert!((AngleUnit::Rad.to_degrees(PI / 2.0) - 90.0).abs() < 0.01);

        // Test gradians
        assert_eq!(AngleUnit::Grad.to_degrees(200.0), 180.0);
        assert_eq!(AngleUnit::Grad.to_degrees(400.0), 360.0);
        assert_eq!(AngleUnit::Grad.to_degrees(100.0), 90.0);

        // Test turns
        assert_eq!(AngleUnit::Turn.to_degrees(0.5), 180.0);
        assert_eq!(AngleUnit::Turn.to_degrees(1.0), 360.0);
        assert_eq!(AngleUnit::Turn.to_degrees(0.25), 90.0);
    }

    #[test]
    fn test_theme_value_as_angle_degrees() {
        // Test plain number (interpreted as degrees)
        let value = ThemeValue::Number(120.0);
        assert_eq!(value.as_angle_degrees(), Some(120.0));

        // Test angle with degrees
        let value = ThemeValue::Angle(120.0, AngleUnit::Deg);
        assert_eq!(value.as_angle_degrees(), Some(120.0));

        // Test angle with turns
        let value = ThemeValue::Angle(0.5, AngleUnit::Turn);
        assert_eq!(value.as_angle_degrees(), Some(180.0));

        // Test angle with radians
        use std::f64::consts::PI;
        let value = ThemeValue::Angle(PI, AngleUnit::Rad);
        assert!((value.as_angle_degrees().unwrap() - 180.0).abs() < 0.01);

        // Test angle with gradians
        let value = ThemeValue::Angle(200.0, AngleUnit::Grad);
        assert_eq!(value.as_angle_degrees(), Some(180.0));

        // Test non-angle value
        let value = ThemeValue::String("not an angle".to_string());
        assert_eq!(value.as_angle_degrees(), None);
    }

    // ===================================================================================
    // Tests for new type-directed evaluation methods
    // ===================================================================================

    #[test]
    fn test_eval_as_color_direct() {
        use crate::theme::eval::EvalContext;
        use indexmap::IndexMap;

        let params = IndexMap::new();
        let ctx = EvalContext::new(&params, 16.0);

        // Direct color value
        let rgba = CssRgba {
            red: 255,
            green: 0,
            blue: 0,
            alpha: 255,
        };
        let value = ThemeValue::Color(rgba);
        assert_eq!(value.eval_as_color(&ctx).unwrap(), rgba);

        // String color (hex)
        let value = ThemeValue::String("#ff0000".to_string());
        let result = value.eval_as_color(&ctx).unwrap();
        assert_eq!(result.red, 255);
        assert_eq!(result.green, 0);
        assert_eq!(result.blue, 0);

        // String color (named)
        let value = ThemeValue::String("red".to_string());
        let result = value.eval_as_color(&ctx).unwrap();
        assert_eq!(result.red, 255);
        assert_eq!(result.green, 0);
        assert_eq!(result.blue, 0);
    }

    #[test]
    fn test_eval_as_color_variable() {
        use crate::theme::eval::EvalContext;
        use datafusion_common::ScalarValue;
        use indexmap::IndexMap;

        let mut params = IndexMap::new();
        params.insert(
            "--accent".to_string(),
            ScalarValue::Utf8(Some("#2563eb".into())),
        );

        let ctx = EvalContext::new(&params, 16.0);

        // Variable that resolves to color
        let value = ThemeValue::Variable("--accent".to_string());
        let result = value.eval_as_color(&ctx).unwrap();
        assert_eq!(result.red, 37);
        assert_eq!(result.green, 99);
        assert_eq!(result.blue, 235);
    }

    #[test]
    fn test_eval_as_color_variable_not_found() {
        use crate::theme::eval::EvalContext;
        use indexmap::IndexMap;

        let params = IndexMap::new();
        let ctx = EvalContext::new(&params, 16.0);

        // Variable not in params
        let value = ThemeValue::Variable("--missing".to_string());
        let result = value.eval_as_color(&ctx);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_eval_as_number_direct() {
        use crate::theme::eval::EvalContext;
        use indexmap::IndexMap;

        let params = IndexMap::new();
        let ctx = EvalContext::new(&params, 16.0);

        // Direct number
        let value = ThemeValue::Number(42.5);
        assert_eq!(value.eval_as_number(&ctx).unwrap(), 42.5);

        // Percentage as decimal
        let value = ThemeValue::Percentage(0.75);
        assert_eq!(value.eval_as_number(&ctx).unwrap(), 0.75);

        // Angle in degrees
        let value = ThemeValue::Angle(180.0, AngleUnit::Deg);
        assert_eq!(value.eval_as_number(&ctx).unwrap(), 180.0);
    }

    #[test]
    fn test_eval_as_number_variable() {
        use crate::theme::eval::EvalContext;
        use datafusion_common::ScalarValue;
        use indexmap::IndexMap;

        let mut params = IndexMap::new();
        params.insert("--size".to_string(), ScalarValue::Float64(Some(100.0)));

        let ctx = EvalContext::new(&params, 16.0);

        // Variable that resolves to number
        let value = ThemeValue::Variable("--size".to_string());
        assert_eq!(value.eval_as_number(&ctx).unwrap(), 100.0);
    }

    #[test]
    fn test_eval_as_number_type_mismatch() {
        use crate::theme::eval::EvalContext;
        use indexmap::IndexMap;

        let params = IndexMap::new();
        let ctx = EvalContext::new(&params, 16.0);

        // String cannot be number
        let value = ThemeValue::String("hello".to_string());
        let result = value.eval_as_number(&ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_eval_as_length_direct() {
        use crate::theme::eval::EvalContext;
        use indexmap::IndexMap;

        let params = IndexMap::new();
        let ctx = EvalContext::new(&params, 16.0);

        // Number as pixels
        let value = ThemeValue::Number(100.0);
        assert_eq!(value.eval_as_length(&ctx).unwrap(), 100.0);

        // Pixels
        let value = ThemeValue::Length(50.0, LengthUnit::Px);
        assert_eq!(value.eval_as_length(&ctx).unwrap(), 50.0);

        // Rem conversion (16px base)
        let value = ThemeValue::Length(2.0, LengthUnit::Rem);
        assert_eq!(value.eval_as_length(&ctx).unwrap(), 32.0);
    }

    #[test]
    fn test_eval_as_string_direct() {
        use crate::theme::eval::EvalContext;
        use indexmap::IndexMap;

        let params = IndexMap::new();
        let ctx = EvalContext::new(&params, 16.0);

        // Direct string
        let value = ThemeValue::String("hello".to_string());
        assert_eq!(value.eval_as_string(&ctx).unwrap(), "hello");

        // Number to string
        let value = ThemeValue::Number(42.5);
        assert_eq!(value.eval_as_string(&ctx).unwrap(), "42.5");

        // Color to hex string
        let rgba = CssRgba {
            red: 255,
            green: 0,
            blue: 0,
            alpha: 255,
        };
        let value = ThemeValue::Color(rgba);
        assert_eq!(value.eval_as_string(&ctx).unwrap(), "#ff0000");
    }

    #[test]
    fn test_eval_as_string_variable() {
        use crate::theme::eval::EvalContext;
        use datafusion_common::ScalarValue;
        use indexmap::IndexMap;

        let mut params = IndexMap::new();
        params.insert(
            "--family".to_string(),
            ScalarValue::Utf8(Some("Arial".into())),
        );

        let ctx = EvalContext::new(&params, 16.0);

        // Variable that resolves to string
        let value = ThemeValue::Variable("--family".to_string());
        assert_eq!(value.eval_as_string(&ctx).unwrap(), "Arial");
    }

    #[test]
    fn test_eval_unsupported_variants() {
        use crate::theme::eval::EvalContext;
        use indexmap::IndexMap;

        let params = IndexMap::new();
        let ctx = EvalContext::new(&params, 16.0);

        // Initial variant
        let value = ThemeValue::Initial;
        assert!(value.eval_as_color(&ctx).is_err());
        assert!(value.eval_as_number(&ctx).is_err());

        // Inherit variant
        let value = ThemeValue::Inherit;
        assert!(value.eval_as_length(&ctx).is_err());

        // None variant
        let value = ThemeValue::None;
        assert!(value.eval_as_string(&ctx).is_err());
    }
}
