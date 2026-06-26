use avenger_typst::{MathDelimiterOptions, MathLimits, MathStyle, MathSyntaxMode};

pub(crate) const DEFAULT_MARKUP_LINE_LEADING_FACTOR: f32 = 0.65;

#[derive(Debug, Clone, PartialEq)]
pub struct TextMarkupConfig {
    pub delimiters: MathDelimiterOptions,
    pub math_style: MathStyle,
    pub syntax: MathSyntaxMode,
    pub limits: MathLimits,
}

impl Default for TextMarkupConfig {
    fn default() -> Self {
        Self {
            delimiters: MathDelimiterOptions::default(),
            math_style: MathStyle::default(),
            syntax: MathSyntaxMode::default(),
            limits: MathLimits::default(),
        }
    }
}

impl TextMarkupConfig {
    pub(crate) fn plain_text(&self) -> Self {
        let mut config = self.clone();
        config.syntax = MathSyntaxMode::PlainText;
        config
    }
}
