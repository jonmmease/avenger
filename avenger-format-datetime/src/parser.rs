use crate::{
    error::DateTimeParseError,
    fields::{DateTimeField, FieldToken, Pattern, PatternToken, StyleBlock, StyleKind},
    style::DateTimeStyleLength,
};

pub fn parse_datetime_spec(spec: &str) -> Result<Pattern, DateTimeParseError> {
    let chars: Vec<(usize, char)> = spec.char_indices().collect();
    let mut tokens = Vec::new();
    let mut literal = String::new();
    let mut i = 0;

    while let Some((pos, ch)) = chars.get(i).copied() {
        match ch {
            '\'' => {
                if matches!(chars.get(i + 1), Some((_, '\''))) {
                    literal.push('\'');
                    i += 2;
                    continue;
                }
                i += 1;
                let mut closed = false;
                while let Some((_, quoted)) = chars.get(i).copied() {
                    if quoted == '\'' {
                        if matches!(chars.get(i + 1), Some((_, '\''))) {
                            literal.push('\'');
                            i += 2;
                        } else {
                            closed = true;
                            i += 1;
                            break;
                        }
                    } else {
                        literal.push(quoted);
                        i += 1;
                    }
                }
                if !closed {
                    return Err(DateTimeParseError::invalid(
                        pos,
                        "unterminated apostrophe-quoted literal",
                    ));
                }
            }
            '%' => {
                if matches!(chars.get(i + 1), Some((_, next)) if next.is_ascii_alphabetic()) {
                    return Err(DateTimeParseError::invalid(
                        pos,
                        "d3/strftime/chrono `%` datetime syntax is not accepted; use LDML fields",
                    ));
                }
                literal.push(ch);
                i += 1;
            }
            '{' => {
                flush_literal(&mut tokens, &mut literal);
                let (style, next) = parse_style_block(spec, &chars, i)?;
                tokens.push(PatternToken::Style(style));
                i = next;
            }
            '}' => {
                return Err(DateTimeParseError::invalid(
                    pos,
                    "literal braces must be apostrophe-quoted",
                ));
            }
            _ if ch.is_ascii_alphabetic() => {
                flush_literal(&mut tokens, &mut literal);
                let start_i = i;
                i += 1;
                while matches!(chars.get(i), Some((_, next)) if *next == ch) {
                    i += 1;
                }
                let width = i - start_i;
                if matches!(ch, 'z' | 'v' | 'V') {
                    return Err(DateTimeParseError::invalid(
                        pos,
                        format!("timezone-name field `{ch}` is not supported in v1"),
                    ));
                }
                let field = DateTimeField::from_char(ch).ok_or_else(|| {
                    DateTimeParseError::invalid(pos, format!("unsupported LDML field `{ch}`"))
                })?;
                tokens.push(PatternToken::Field(FieldToken { field, width }));
            }
            _ => {
                literal.push(ch);
                i += 1;
            }
        }
    }

    flush_literal(&mut tokens, &mut literal);
    Ok(Pattern { tokens })
}

fn flush_literal(tokens: &mut Vec<PatternToken>, literal: &mut String) {
    if !literal.is_empty() {
        tokens.push(PatternToken::Literal(std::mem::take(literal)));
    }
}

fn parse_style_block(
    spec: &str,
    chars: &[(usize, char)],
    open_i: usize,
) -> Result<(StyleBlock, usize), DateTimeParseError> {
    let open_pos = chars[open_i].0;
    let mut close_i = None;
    let mut i = open_i + 1;
    while let Some((_, ch)) = chars.get(i) {
        if *ch == '}' {
            close_i = Some(i);
            break;
        }
        if *ch == '{' {
            return Err(DateTimeParseError::invalid(
                chars[i].0,
                "nested style blocks are not supported",
            ));
        }
        i += 1;
    }
    let close_i = close_i.ok_or_else(|| {
        DateTimeParseError::invalid(open_pos, "datetime style block must end with `}`")
    })?;
    let start = chars
        .get(open_i + 1)
        .map(|(pos, _)| *pos)
        .unwrap_or(spec.len());
    let end = chars
        .get(close_i)
        .map(|(pos, _)| *pos)
        .unwrap_or(spec.len());
    let body = &spec[start..end];
    let parts: Vec<&str> = body.split(':').collect();
    if parts.is_empty() || parts.len() > 2 || parts[0].is_empty() {
        return Err(DateTimeParseError::invalid(
            open_pos,
            "malformed datetime style block",
        ));
    }
    let kind = match parts[0] {
        "date" => StyleKind::Date,
        "time" => StyleKind::Time,
        "datetime" => StyleKind::DateTime,
        "skeleton" => {
            return Err(DateTimeParseError::invalid(
                open_pos,
                "LDML skeleton matching is not supported in v1",
            ));
        }
        other => {
            return Err(DateTimeParseError::invalid(
                open_pos,
                format!("unknown datetime style block `{other}`"),
            ));
        }
    };
    let length = if parts.len() == 2 {
        Some(DateTimeStyleLength::from_str(parts[1]).ok_or_else(|| {
            DateTimeParseError::invalid(open_pos, format!("unknown datetime style `{}`", parts[1]))
        })?)
    } else {
        None
    };
    Ok((StyleBlock { kind, length }, close_i + 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fields::{DateTimeField, PatternToken, StyleKind};

    #[test]
    fn parses_literals_fields_and_quoting() {
        let pattern = parse_datetime_spec("MMM d 'at' HH:mm ''yy").unwrap();
        assert!(matches!(
            pattern.tokens[0],
            PatternToken::Field(FieldToken {
                field: DateTimeField::MonthFormat,
                width: 3
            })
        ));
        assert!(pattern
            .tokens
            .iter()
            .any(|token| matches!(token, PatternToken::Literal(value) if value.contains(" at "))));
        assert!(pattern
            .tokens
            .iter()
            .any(|token| matches!(token, PatternToken::Literal(value) if value.contains("'"))));
    }

    #[test]
    fn parses_style_blocks() {
        let pattern = parse_datetime_spec("{date:long} {time}").unwrap();
        assert!(matches!(
            pattern.tokens[0],
            PatternToken::Style(StyleBlock {
                kind: StyleKind::Date,
                length: Some(DateTimeStyleLength::Long),
            })
        ));
        assert!(matches!(
            pattern.tokens[2],
            PatternToken::Style(StyleBlock {
                kind: StyleKind::Time,
                length: None,
            })
        ));
    }

    #[test]
    fn rejects_strftime_and_unsupported_fields() {
        assert!(parse_datetime_spec("%Y").is_err());
        assert!(parse_datetime_spec("{skeleton:yMMMd}").is_err());
        assert!(parse_datetime_spec("yyyy z").is_err());
        assert!(parse_datetime_spec("{foo:bar}").is_err());
        assert!(parse_datetime_spec("yyyy {").is_err());
    }

    #[test]
    fn supports_v1_field_subset() {
        for field in [
            "G", "y", "M", "L", "d", "D", "E", "e", "c", "q", "Q", "a", "h", "H", "K", "k", "m",
            "s", "S", "X", "x", "Z", "O",
        ] {
            assert!(parse_datetime_spec(field).is_ok(), "{field}");
        }
    }
}
