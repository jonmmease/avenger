/// A localized label with optional parts for scientific notation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormattedNumber {
    pub text: String,
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
    Plain,
    Exponent { mantissa: String, exponent: i32 },
}
