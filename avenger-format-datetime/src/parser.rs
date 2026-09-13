use crate::{
    error::DateTimeParseError,
    fields::{Pattern, PatternToken},
};

/// Parse documented D3 datetime directives and their padding modifiers.
pub fn parse_datetime_spec(spec: &str) -> Result<Pattern, DateTimeParseError> {
    let mut tokens = Vec::new();
    let mut literal = String::new();
    let mut chars = spec.char_indices();
    while let Some((position, ch)) = chars.next() {
        if ch != '%' {
            literal.push(ch);
            continue;
        }
        if !literal.is_empty() {
            tokens.push(PatternToken::Literal(std::mem::take(&mut literal)));
        }
        let (_, mut code) = chars
            .next()
            .ok_or_else(|| DateTimeParseError::invalid(position, "missing directive after `%`"))?;
        let padding = match code {
            '-' | '_' | '0' => {
                let padding = match code {
                    '-' => None,
                    '_' => Some(' '),
                    _ => Some('0'),
                };
                code = chars
                    .next()
                    .ok_or_else(|| {
                        DateTimeParseError::invalid(
                            position,
                            "missing directive after padding modifier",
                        )
                    })?
                    .1;
                padding
            }
            _ => Some(if code == 'e' { ' ' } else { '0' }),
        };
        if !"aAbBcdefgGHIjLmMpqQsSuUVwWxXyYZ%".contains(code) {
            return Err(DateTimeParseError::invalid(
                position,
                format!("unknown D3 datetime directive `%{code}`"),
            ));
        }
        tokens.push(PatternToken::Directive { code, padding });
    }
    if !literal.is_empty() {
        tokens.push(PatternToken::Literal(literal));
    }
    Ok(Pattern(tokens))
}
