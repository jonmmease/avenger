/// A localized label with optional parts for scientific notation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormattedNumber {
    /// Complete label, including signs, affixes, grouping, and padding.
    pub text: String,
    /// Structure for rendering a decimal exponent without parsing the label.
    pub typesetting: NumberTypesetting,
}

impl FormattedNumber {
    /// Construct a label without scientific notation parts.
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            typesetting: NumberTypesetting::Plain,
        }
    }
}

/// Decimal scientific notation parts when the label has no affixes, padding, or custom numerals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NumberTypesetting {
    /// Render [`FormattedNumber::text`] directly.
    Plain,
    /// Decimal scientific notation suitable for rendering as a mantissa times a power of ten.
    Exponent {
        /// Localized coefficient, including its sign.
        mantissa: String,
        /// Signed power of ten.
        exponent: i32,
    },
}
