use crate::error::FormatError;

pub fn validate_currency_code(code: &str) -> Result<(), FormatError> {
    if code.len() == 3 && code.chars().all(|ch| ch.is_ascii_uppercase()) {
        Ok(())
    } else {
        Err(FormatError::InvalidCurrencyCode(code.to_string()))
    }
}
