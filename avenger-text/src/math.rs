use avenger_typst::{MathDelimiterOptions, MathLimits, MathStyle, MathSyntaxMode};

pub(crate) const DEFAULT_MATH_LINE_LEADING_FACTOR: f32 = 0.65;

#[derive(Debug, Clone, PartialEq)]
pub enum TextMarkupMode {
    Plain,
    TypstMathDelimited(MathDelimiterOptions),
}

impl Default for TextMarkupMode {
    fn default() -> Self {
        Self::TypstMathDelimited(MathDelimiterOptions::default())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathMarkupErrorPolicy {
    TreatInvalidMathAsLiteral,
    UseFallbackBounds,
    ErrorOnPathExtraction,
}

impl Default for MathMarkupErrorPolicy {
    fn default() -> Self {
        Self::TreatInvalidMathAsLiteral
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextMathConfig {
    pub mode: TextMarkupMode,
    pub math_style: MathStyle,
    pub syntax: MathSyntaxMode,
    pub limits: MathLimits,
    pub error_policy: MathMarkupErrorPolicy,
}

impl Default for TextMathConfig {
    fn default() -> Self {
        Self {
            mode: TextMarkupMode::default(),
            math_style: MathStyle::default(),
            syntax: MathSyntaxMode::default(),
            limits: MathLimits::default(),
            error_policy: MathMarkupErrorPolicy::default(),
        }
    }
}
