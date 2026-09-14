#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormattedNumber {
    pub text: String,
    pub typesetting: NumberTypesetting,
}

impl FormattedNumber {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            typesetting: NumberTypesetting::Plain,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NumberTypesetting {
    Plain,
    Exponent {
        mantissa: String,
        exponent: i32,
        marker: ExponentMarker,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExponentMarker {
    LowerE,
}
