use avenger_typst_label::{LabelLimits, MathStyle};

use crate::types::TextSyntaxMode;

pub(crate) const DEFAULT_MARKUP_LINE_LEADING_FACTOR: f32 = 0.65;

#[derive(Debug, Clone, PartialEq)]
pub struct TextMarkupConfig {
    pub math_style: MathStyle,
    pub syntax_mode: TextSyntaxMode,
    pub limits: LabelLimits,
}

impl Default for TextMarkupConfig {
    fn default() -> Self {
        Self {
            math_style: MathStyle::default(),
            syntax_mode: TextSyntaxMode::Plain,
            limits: LabelLimits::default(),
        }
    }
}

impl TextMarkupConfig {
    pub(crate) fn with_syntax_mode(&self, mode: TextSyntaxMode) -> Self {
        let mut config = self.clone();
        config.syntax_mode = mode;
        config
    }

    pub(crate) fn plain_text(&self) -> Self {
        self.with_syntax_mode(TextSyntaxMode::Plain)
    }
}
