use avenger_typst::{
    MathDelimiterOptions, MathLimits, MathStyle, MathSyntaxMode, UnmatchedDelimiterPolicy,
};

use crate::types::TextSyntaxMode;

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
            syntax: MathSyntaxMode::PlainText,
            limits: MathLimits::default(),
        }
    }
}

impl TextMarkupConfig {
    pub(crate) fn with_syntax_mode(&self, mode: TextSyntaxMode) -> Self {
        let mut config = self.clone();
        match mode {
            TextSyntaxMode::Plain => {
                config.syntax = MathSyntaxMode::PlainText;
            }
            TextSyntaxMode::TypstMarkup => {
                config.syntax = MathSyntaxMode::TypstFragmentStrict;
                config.delimiters.unmatched = UnmatchedDelimiterPolicy::Error;
            }
        }
        config
    }

    pub(crate) fn plain_text(&self) -> Self {
        self.with_syntax_mode(TextSyntaxMode::Plain)
    }
}
