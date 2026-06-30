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
    if pattern.contains('*') {
        return Err(FormatError::InvalidLocaleData(
            "number pattern padding escapes are not supported".to_string(),
        ));
    }
    if pattern.matches(';').count() > 1 {
        return Err(FormatError::InvalidLocaleData(
            "number pattern must contain at most one negative subpattern".to_string(),
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
    let (first_digit, last_digit) = digit_placeholder_bounds(pattern)?;
    let numeric = &pattern[first_digit..last_digit];
    validate_numeric_pattern(numeric)?;
    Ok((
        unquote_affix(&pattern[..first_digit])?,
        unquote_affix(&pattern[last_digit..])?,
    ))
}

fn digit_placeholder_bounds(pattern: &str) -> Result<(usize, usize), FormatError> {
    let mut first = None;
    let mut last = None;
    let mut in_quote = false;
    let mut chars = pattern.char_indices().peekable();

    while let Some((idx, ch)) = chars.next() {
        if ch == '\'' {
            if matches!(chars.peek(), Some((_, '\''))) {
                chars.next();
            } else {
                in_quote = !in_quote;
            }
            continue;
        }
        if !in_quote && matches!(ch, '0' | '#' | '@') {
            first.get_or_insert(idx);
            last = Some(idx + ch.len_utf8());
        }
    }

    if in_quote {
        return Err(FormatError::InvalidLocaleData(
            "number pattern has an unterminated quoted literal".to_string(),
        ));
    }

    let first_digit = first.ok_or_else(|| {
        FormatError::InvalidLocaleData("pattern has no digit placeholders".to_string())
    })?;
    Ok((first_digit, last.unwrap()))
}

fn validate_numeric_pattern(pattern: &str) -> Result<(), FormatError> {
    for ch in pattern.chars() {
        if matches!(
            ch,
            '0' | '#' | '@' | ',' | '.' | '%' | '\u{2030}' | 'E' | '+' | '-'
        ) {
            continue;
        }
        return Err(FormatError::InvalidLocaleData(format!(
            "unsupported character `{ch}` in number pattern digit section"
        )));
    }
    Ok(())
}

fn unquote_affix(input: &str) -> Result<String, FormatError> {
    let mut output = String::new();
    let mut in_quote = false;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\'' {
            if matches!(chars.peek(), Some('\'')) {
                chars.next();
                output.push('\'');
            } else {
                in_quote = !in_quote;
            }
        } else {
            output.push(ch);
        }
    }

    if in_quote {
        return Err(FormatError::InvalidLocaleData(
            "number pattern has an unterminated quoted literal".to_string(),
        ));
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::normalize_number_pattern;

    #[test]
    fn normalizes_decimal_affixes() {
        let pattern = normalize_number_pattern("#,##0.###;(#,##0.###)").unwrap();
        assert_eq!(pattern.positive_prefix, "");
        assert_eq!(pattern.positive_suffix, "");
        assert_eq!(pattern.negative_prefix, "(");
        assert_eq!(pattern.negative_suffix, ")");
    }

    #[test]
    fn normalizes_quoted_affixes() {
        let pattern = normalize_number_pattern("'~'#,##0 'items'").unwrap();
        assert_eq!(pattern.positive_prefix, "~");
        assert_eq!(pattern.positive_suffix, " items");
        assert_eq!(pattern.negative_prefix, "-~");
        assert_eq!(pattern.negative_suffix, " items");
    }

    #[test]
    fn rejects_unsupported_patterns() {
        assert!(normalize_number_pattern("*x#,##0").is_err());
        assert!(normalize_number_pattern("#,##0;(#,##0);-#,##0").is_err());
        assert!(normalize_number_pattern("'bad#,##0").is_err());
        assert!(normalize_number_pattern("#,##x0").is_err());
    }
}
