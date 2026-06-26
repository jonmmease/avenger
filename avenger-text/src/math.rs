use avenger_typst::{MathDelimiterOptions, MathLimits, MathStyle, MathSyntaxMode};

pub(crate) const DEFAULT_MARKUP_LINE_LEADING_FACTOR: f32 = 0.65;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextMarkupErrorPolicy {
    TreatInvalidMathAsLiteral,
    UseFallbackBounds,
    ErrorOnPathExtraction,
}

impl Default for TextMarkupErrorPolicy {
    fn default() -> Self {
        Self::TreatInvalidMathAsLiteral
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextMarkupConfig {
    pub delimiters: MathDelimiterOptions,
    pub math_style: MathStyle,
    pub syntax: MathSyntaxMode,
    pub limits: MathLimits,
    pub error_policy: TextMarkupErrorPolicy,
}

impl Default for TextMarkupConfig {
    fn default() -> Self {
        Self {
            delimiters: MathDelimiterOptions::default(),
            math_style: MathStyle::default(),
            syntax: MathSyntaxMode::default(),
            limits: MathLimits::default(),
            error_policy: TextMarkupErrorPolicy::default(),
        }
    }
}
