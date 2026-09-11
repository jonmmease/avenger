use crate::{
    error::ParseError,
    spec::{Align, FormatType, NumberFormatSpec, SignPolicy, Symbol},
};

pub fn parse_number_spec(spec: &str) -> Result<NumberFormatSpec, ParseError> {
    let mut chars: Vec<(usize, char)> = spec.char_indices().collect();
    let mut i = 0;
    let mut parsed = NumberFormatSpec::default();

    if chars.len() >= 2 && Align::from_char(chars[1].1).is_some() {
        parsed.fill = Some(chars[0].1);
        parsed.align = Align::from_char(chars[1].1);
        i = 2;
    } else if let Some((_, ch)) = chars.get(i) {
        if let Some(align) = Align::from_char(*ch) {
            parsed.align = Some(align);
            i += 1;
        }
    }

    if let Some((_, ch)) = chars.get(i) {
        if let Some(sign) = SignPolicy::from_char(*ch) {
            parsed.sign = Some(sign);
            i += 1;
        }
    }

    if let Some((_, ch)) = chars.get(i) {
        if let Some(symbol) = Symbol::from_char(*ch) {
            parsed.symbol = Some(symbol);
            i += 1;
        }
    }

    if let Some((_, '0')) = chars.get(i) {
        parsed.zero = true;
        parsed.fill = Some('0');
        parsed.align = Some(Align::AfterSign);
        i += 1;
    }

    if let Some((start, ch)) = chars.get(i) {
        if ch.is_ascii_digit() {
            let start_i = i;
            i += 1;
            while matches!(chars.get(i), Some((_, ch)) if ch.is_ascii_digit()) {
                i += 1;
            }
            let end = byte_end(spec, &chars, i);
            let width = spec[*start..end].parse::<usize>().map_err(|_| {
                ParseError::invalid(chars[start_i].0, "width is too large to represent")
            })?;
            parsed.width = Some(width);
        }
    }

    if let Some((_, ',')) = chars.get(i) {
        parsed.group = Some(true);
        i += 1;
    }

    if let Some((dot_pos, '.')) = chars.get(i) {
        i += 1;
        let precision_start = i;
        while matches!(chars.get(i), Some((_, ch)) if ch.is_ascii_digit()) {
            i += 1;
        }
        if precision_start == i {
            return Err(ParseError::invalid(
                *dot_pos,
                "precision requires one or more digits after `.`",
            ));
        }
        let start = chars[precision_start].0;
        let end = byte_end(spec, &chars, i);
        let precision = spec[start..end].parse::<u16>().map_err(|_| {
            ParseError::invalid(
                chars[precision_start].0,
                "precision is too large to represent",
            )
        })?;
        if precision > u8::MAX as u16 {
            return Err(ParseError::invalid(
                chars[precision_start].0,
                "precision must be less than 256",
            ));
        }
        parsed.precision = Some(precision as u8);
    }

    if let Some((_, '~')) = chars.get(i) {
        parsed.trim = Some(true);
        i += 1;
    }

    if let Some((type_pos, ch)) = chars.get(i) {
        if *ch == 'E' {
            return Err(ParseError::invalid(
                *type_pos,
                "`E` is reserved and is not part of the v1 public grammar",
            ));
        }
        let format_type = FormatType::from_char(*ch).ok_or_else(|| {
            ParseError::invalid(*type_pos, format!("unknown number format type `{ch}`"))
        })?;
        parsed.format_type = Some(format_type);
        i += 1;

        if let Some((param_pos, '[')) = chars.get(i) {
            if format_type != FormatType::Currency {
                return Err(ParseError::invalid(
                    *param_pos,
                    "type parameters are only supported for `C`",
                ));
            }
            i += 1;
            let code_start_i = i;
            while matches!(chars.get(i), Some((_, ch)) if ch.is_ascii_uppercase()) {
                i += 1;
            }
            let code_start = chars
                .get(code_start_i)
                .map(|(pos, _)| *pos)
                .unwrap_or_else(|| spec.len());
            let code_end = byte_end(spec, &chars, i);
            let code = &spec[code_start..code_end];
            if code.len() != 3 {
                return Err(ParseError::invalid(
                    code_start,
                    "currency code must be three uppercase ASCII letters",
                ));
            }
            match chars.get(i) {
                Some((_, ']')) => i += 1,
                Some((pos, _)) => {
                    return Err(ParseError::invalid(
                        *pos,
                        "currency type parameter must end with `]`",
                    ));
                }
                None => {
                    return Err(ParseError::invalid(
                        spec.len(),
                        "currency type parameter must end with `]`",
                    ));
                }
            }
            parsed.currency = Some(code.to_string());
        }
    }

    if let Some((pos, _)) = chars.get(i) {
        return Err(ParseError::invalid(
            *pos,
            "trailing characters in format specifier",
        ));
    }

    chars.clear();
    Ok(parsed)
}

fn byte_end(spec: &str, chars: &[(usize, char)], i: usize) -> usize {
    chars.get(i).map(|(pos, _)| *pos).unwrap_or(spec.len())
}

#[cfg(test)]
mod tests {
    use super::parse_number_spec;
    use crate::spec::{Align, FormatType, SignPolicy, Symbol};

    #[test]
    fn parses_all_d3_fields() {
        let spec = parse_number_spec(".=+08,.2~f").unwrap();
        assert_eq!(spec.fill, Some('0'));
        assert_eq!(spec.align, Some(Align::AfterSign));
        assert_eq!(spec.sign, Some(SignPolicy::Plus));
        assert!(spec.zero);
        assert_eq!(spec.width, Some(8));
        assert_eq!(spec.group, Some(true));
        assert_eq!(spec.precision, Some(2));
        assert_eq!(spec.trim, Some(true));
        assert_eq!(spec.format_type, Some(FormatType::Fixed));
    }

    #[test]
    fn parses_symbol_and_extension_types() {
        assert_eq!(
            parse_number_spec("#x").unwrap().symbol,
            Some(Symbol::Alternate)
        );
        assert_eq!(
            parse_number_spec("$,.2f").unwrap().symbol,
            Some(Symbol::CurrencyCompat)
        );
        assert_eq!(
            parse_number_spec(".1S").unwrap().format_type,
            Some(FormatType::CompactShort)
        );
        assert_eq!(
            parse_number_spec(".1L").unwrap().format_type,
            Some(FormatType::CompactLong)
        );
        let currency = parse_number_spec(",.2C[USD]").unwrap();
        assert_eq!(currency.format_type, Some(FormatType::Currency));
        assert_eq!(currency.currency.as_deref(), Some("USD"));
    }

    #[test]
    fn parses_empty_spec() {
        assert_eq!(parse_number_spec("").unwrap().format_type, None);
    }

    #[test]
    fn rejects_invalid_specs_without_panicking() {
        for spec in [
            ".", ".x", "q", ".2E", "C[]", "C[usd]", "C[US]", "C[USDD]", "C[USD",
        ] {
            assert!(parse_number_spec(spec).is_err(), "{spec} should fail");
        }
    }
}
