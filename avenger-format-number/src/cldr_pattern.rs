use crate::error::FormatError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedNumberPattern {
    pub positive_prefix: String,
    pub positive_suffix: String,
    pub negative_prefix: String,
    pub negative_suffix: String,
}

pub fn normalize_number_pattern(pattern: &str) -> Result<NormalizedNumberPattern, FormatError> {
    if pattern.is_empty() {
        return Err(FormatError::InvalidLocaleData(
            "number pattern must not be empty".to_string(),
        ));
    }

    let (positive, negative) = pattern
        .split_once(';')
        .map(|(positive, negative)| (positive, Some(negative)))
        .unwrap_or((pattern, None));

    let positive = split_pattern_affixes(positive)?;
    let negative = match negative {
        Some(value) => split_pattern_affixes(value)?,
        None => (format!("-{}", positive.0), positive.1.clone()),
    };

    Ok(NormalizedNumberPattern {
        positive_prefix: positive.0,
        positive_suffix: positive.1,
        negative_prefix: negative.0,
        negative_suffix: negative.1,
    })
}

fn split_pattern_affixes(pattern: &str) -> Result<(String, String), FormatError> {
    let first_digit = pattern.find(['0', '#', '@']).ok_or_else(|| {
        FormatError::InvalidLocaleData("pattern has no digit placeholders".to_string())
    })?;
    let last_digit = pattern.rfind(['0', '#', '@']).ok_or_else(|| {
        FormatError::InvalidLocaleData("pattern has no digit placeholders".to_string())
    })?;
    Ok((
        pattern[..first_digit].to_string(),
        pattern[last_digit + 1..].to_string(),
    ))
}
