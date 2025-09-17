//! CSS stylesheet parser

use super::CompiledRule;
use super::selector_impl::{ChartPseudoClass, ChartSelectors};
use super::value::{parse_hex_color, parse_named_color, parse_rgb_function};
use crate::theme::{LengthUnit, ThemeValue};
use cssparser::{BasicParseErrorKind, Parser as CssParser, ParserInput, Token};
use indexmap::IndexMap;
use selectors::parser::{ParseRelative, Parser, SelectorList};

/// Parse a CSS stylesheet into rules
pub fn parse_stylesheet(css: &str) -> Result<Vec<CompiledRule>, String> {
    let mut input = ParserInput::new(css);
    let mut parser = CssParser::new(&mut input);
    let mut rules = Vec::new();
    let mut source_order = 0;

    while !parser.is_exhausted() {
        // Skip whitespace
        skip_whitespace(&mut parser);

        if parser.is_exhausted() {
            break;
        }

        // Try to parse a rule
        match parse_rule(&mut parser, source_order) {
            Ok(rule) => {
                rules.push(rule);
                source_order += 1;
            }
            Err(_) => {
                // Skip to next rule
                skip_to_next_rule(&mut parser);
            }
        }
    }

    Ok(rules)
}

/// Parse a single CSS rule
fn parse_rule(parser: &mut CssParser, source_order: usize) -> Result<CompiledRule, String> {
    // Parse selector
    let (selector_str, _found_block) = parse_selector_string(parser)?;

    if selector_str.is_empty() {
        return Err("Empty selector".to_string());
    }

    // Handle :root specially - it applies to the root element
    // For our purposes, we'll treat it as a universal selector for variable declarations
    if selector_str == ":root" {
        // Parse declarations for :root
        let declarations = parse_declaration_block(parser)?;

        // Create a simple universal selector for :root
        let mut selector_input = ParserInput::new("*");
        let mut selector_parser = CssParser::new(&mut selector_input);
        let selector_list =
            SelectorList::parse(&ChartParser, &mut selector_parser, ParseRelative::No)
                .map_err(|_| "Failed to create universal selector".to_string())?;
        let selector = selector_list
            .slice()
            .first()
            .ok_or_else(|| "No selector found".to_string())?
            .clone();

        return Ok(CompiledRule {
            selector,
            specificity: 0,
            source_order,
            declarations,
        });
    }

    // Parse selector using selectors crate
    let mut selector_input = ParserInput::new(&selector_str);
    let mut selector_parser = CssParser::new(&mut selector_input);
    let selector_list = SelectorList::parse(&ChartParser, &mut selector_parser, ParseRelative::No)
        .map_err(|_| format!("Failed to parse selector: {}", selector_str))?;

    // For simplicity, use the first selector
    let selector = selector_list
        .slice()
        .first()
        .ok_or_else(|| "No selector found".to_string())?
        .clone();

    let specificity = selector.specificity();

    // Parse declarations
    let declarations = parse_declaration_block(parser)?;

    Ok(CompiledRule {
        selector,
        specificity,
        source_order,
        declarations,
    })
}

/// Parse the selector part of a rule
fn parse_selector_string(parser: &mut CssParser) -> Result<(String, bool), String> {
    let mut selector = String::new();
    let mut found_block = false;
    let mut last_was_delim = false;

    loop {
        let state = parser.state();
        match parser.next() {
            Ok(Token::CurlyBracketBlock) => {
                // Found start of declarations - need to reset to before the block
                parser.reset(&state);
                found_block = true;
                return Ok((selector.trim().to_string(), found_block));
            }
            Ok(token) => {
                // Handle tokens more carefully for selector syntax
                match token {
                    Token::Ident(s) => {
                        if !selector.is_empty() && !last_was_delim && !selector.ends_with(' ') {
                            selector.push(' ');
                        }
                        selector.push_str(&s.to_string());
                        last_was_delim = false;
                    }
                    Token::Delim(c) => {
                        selector.push(*c);
                        last_was_delim = true;
                    }
                    Token::Hash(s) | Token::IDHash(s) => {
                        if !selector.is_empty() && !selector.ends_with(' ') {
                            selector.push(' ');
                        }
                        selector.push('#');
                        selector.push_str(&s);
                        last_was_delim = false;
                    }
                    Token::Colon => {
                        selector.push(':');
                        last_was_delim = true;
                    }
                    Token::WhiteSpace(_) => {
                        if !selector.is_empty() && !selector.ends_with(' ') {
                            selector.push(' ');
                        }
                        last_was_delim = false;
                    }
                    _ => {
                        // Other tokens
                        selector.push_str(&token_to_string(token));
                        last_was_delim = false;
                    }
                }
            }
            Err(_) => {
                if !selector.is_empty() {
                    return Ok((selector.trim().to_string(), found_block));
                }
                return Err("Expected selector".to_string());
            }
        }
    }
}

/// Parse a declaration block
fn parse_declaration_block(parser: &mut CssParser) -> Result<IndexMap<String, ThemeValue>, String> {
    let mut declarations = IndexMap::new();

    // Expect a curly bracket block
    match parser.next() {
        Ok(Token::CurlyBracketBlock) => {}
        _ => return Err("Expected declaration block".to_string()),
    }

    parser
        .parse_nested_block::<_, _, BasicParseErrorKind>(|parser| {
            loop {
                skip_whitespace(parser);

                if parser.is_exhausted() {
                    break;
                }

                // Parse property name
                let property = match parser.next() {
                    Ok(Token::Ident(name)) => name.to_string(),
                    Ok(Token::Semicolon) => {
                        // Empty declaration, skip
                        continue;
                    }
                    _ => {
                        skip_to_semicolon(parser);
                        continue;
                    }
                };

                // Expect colon
                match parser.next() {
                    Ok(Token::Colon) => {}
                    _ => {
                        skip_to_semicolon(parser);
                        continue;
                    }
                };

                // Parse value
                // For CSS variables and range properties, collect all values until semicolon
                if property.starts_with("--")
                    || property.ends_with("-discrete")
                    || property.ends_with("-continuous")
                {
                    match parse_value_list(parser) {
                        Ok(values) => {
                            // If it's a single value, store it directly
                            // If it's multiple values, store as a string to be parsed later
                            if values.len() == 1 {
                                declarations.insert(property, values.into_iter().next().unwrap());
                            } else {
                                // Join values as a comma-separated string
                                let joined = values
                                    .iter()
                                    .map(|v| match v {
                                        ThemeValue::Color(rgba) => format!(
                                            "#{:02x}{:02x}{:02x}",
                                            rgba.red, rgba.green, rgba.blue
                                        ),
                                        ThemeValue::String(s) | ThemeValue::Keyword(s) => s.clone(),
                                        ThemeValue::Double(n) => n.to_string(),
                                        ThemeValue::Float(f) => f.to_string(),
                                        _ => String::new(),
                                    })
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                declarations.insert(property, ThemeValue::String(joined));
                            }
                        }
                        Err(_) => {
                            skip_to_semicolon(parser);
                        }
                    }
                } else {
                    match parse_value(parser, &property) {
                        Ok(value) => {
                            declarations.insert(property, value);
                        }
                        Err(_) => {
                            // Skip invalid values
                        }
                    }
                    // Skip to semicolon or end
                    skip_to_semicolon(parser);
                }
            }

            Ok(())
        })
        .map_err(|_| "Failed to parse declaration block".to_string())?;

    Ok(declarations)
}

/// Parse a list of CSS values (comma-separated)
fn parse_value_list(parser: &mut CssParser) -> Result<Vec<ThemeValue>, String> {
    let mut values = Vec::new();

    loop {
        skip_whitespace(parser);

        // Check if we've reached the end
        if parser.is_exhausted() {
            break;
        }

        // Try to parse a value
        match parse_value(parser, "") {
            Ok(value) => {
                values.push(value);
                skip_whitespace(parser);

                // Check for comma or end
                let state = parser.state();
                match parser.next() {
                    Ok(Token::Comma) => {
                        // Continue to next value
                        continue;
                    }
                    Ok(Token::Semicolon) => {
                        parser.reset(&state);
                        break;
                    }
                    Err(_) => {
                        // End of input
                        break;
                    }
                    _ => {
                        // Unexpected token, assume end of list
                        parser.reset(&state);
                        break;
                    }
                }
            }
            Err(_) => {
                // Check if it's just a comma or semicolon
                let state = parser.state();
                match parser.next() {
                    Ok(Token::Comma) => continue,
                    Ok(Token::Semicolon) => {
                        parser.reset(&state);
                        break;
                    }
                    _ => break,
                }
            }
        }
    }

    if values.is_empty() {
        Err("No values found".to_string())
    } else {
        Ok(values)
    }
}

/// Parse a CSS value
fn parse_value(parser: &mut CssParser, _property: &str) -> Result<ThemeValue, String> {
    skip_whitespace(parser);

    match parser.next() {
        Ok(Token::Ident(s)) => {
            let s_str = s.to_string();
            // Check for special keywords
            match s_str.as_str() {
                "none" => Ok(ThemeValue::None),
                "initial" => Ok(ThemeValue::Initial),
                "inherit" => Ok(ThemeValue::Inherit),
                _ => {
                    // Check if it's a color
                    if let Some(color) = parse_named_color(&s_str) {
                        Ok(ThemeValue::Color(color))
                    } else {
                        Ok(ThemeValue::Keyword(s_str))
                    }
                }
            }
        }
        Ok(Token::Number { value, .. }) => {
            let num_value = *value as f64;
            // Check for unit - save state first in case it's not a unit
            let state = parser.state();
            match parser.next() {
                Ok(Token::Ident(unit)) => {
                    match unit.as_ref() {
                        "px" => Ok(ThemeValue::Length(num_value, LengthUnit::Px)),
                        "em" => Ok(ThemeValue::Length(num_value, LengthUnit::Em)),
                        "rem" => Ok(ThemeValue::Length(num_value, LengthUnit::Rem)),
                        "pt" => Ok(ThemeValue::Length(num_value, LengthUnit::Pt)),
                        "%" => Ok(ThemeValue::Percentage(num_value)),
                        _ => {
                            // Not a recognized unit, reset and treat as plain number
                            parser.reset(&state);
                            Ok(ThemeValue::Double(num_value))
                        }
                    }
                }
                _ => {
                    // No unit follows, reset and treat as plain number
                    parser.reset(&state);
                    Ok(ThemeValue::Double(num_value))
                }
            }
        }
        Ok(Token::Dimension { value, unit, .. }) => {
            let num_value = *value as f64;
            match unit.as_ref() {
                "px" => Ok(ThemeValue::Length(num_value, LengthUnit::Px)),
                "em" => Ok(ThemeValue::Length(num_value, LengthUnit::Em)),
                "rem" => Ok(ThemeValue::Length(num_value, LengthUnit::Rem)),
                "pt" => Ok(ThemeValue::Length(num_value, LengthUnit::Pt)),
                _ => Ok(ThemeValue::Double(num_value)),
            }
        }
        Ok(Token::Percentage { unit_value, .. }) => {
            Ok(ThemeValue::Percentage((*unit_value * 100.0) as f64))
        }
        Ok(Token::Hash(h)) | Ok(Token::IDHash(h)) => {
            if let Some(color) = parse_hex_color(h.as_ref()) {
                Ok(ThemeValue::Color(color))
            } else {
                Err(format!("Invalid color: #{}", h))
            }
        }
        Ok(Token::QuotedString(s)) => Ok(ThemeValue::String(s.to_string())),
        Ok(Token::Function(name)) => {
            let name_str = name.to_string();
            let args = parse_function_args(parser)?;

            match name_str.as_str() {
                "rgb" | "rgba" => {
                    if let Some(color) = parse_rgb_function(&args) {
                        Ok(ThemeValue::Color(color))
                    } else {
                        Ok(ThemeValue::Function(name_str, args))
                    }
                }
                "var" => {
                    // CSS variable
                    if let Some(ThemeValue::String(var_name)) = args.first() {
                        Ok(ThemeValue::Variable(var_name.clone()))
                    } else {
                        Err("Invalid var() function".to_string())
                    }
                }
                _ => Ok(ThemeValue::Function(name_str, args)),
            }
        }
        _ => Err("Unexpected token".to_string()),
    }
}

/// Parse function arguments
fn parse_function_args(parser: &mut CssParser) -> Result<Vec<ThemeValue>, String> {
    let mut args = Vec::new();

    parser
        .parse_nested_block::<_, _, BasicParseErrorKind>(|parser| {
            loop {
                skip_whitespace(parser);

                if parser.is_exhausted() {
                    break;
                }

                // Parse argument value
                match parser.next() {
                    Ok(Token::Number { value, .. }) => args.push(ThemeValue::Double(*value as f64)),
                    Ok(Token::Ident(s)) => args.push(ThemeValue::String(s.to_string())),
                    Ok(Token::QuotedString(s)) => args.push(ThemeValue::String(s.to_string())),
                    Ok(Token::Comma) => {} // Skip commas
                    _ => break,
                }
            }

            Ok(())
        })
        .map_err(|_| "Failed to parse function arguments".to_string())?;

    Ok(args)
}

/// Convert token to string
fn token_to_string(token: &Token) -> String {
    match token {
        Token::Ident(s) => s.to_string(),
        Token::IDHash(s) | Token::Hash(s) => format!("#{}", s),
        Token::QuotedString(s) => format!("\"{}\"", s),
        Token::Delim(c) => c.to_string(),
        Token::Colon => ":".to_string(),
        Token::Comma => ",".to_string(),
        Token::WhiteSpace(_) => " ".to_string(),
        _ => "".to_string(),
    }
}

/// Skip whitespace tokens
fn skip_whitespace(parser: &mut CssParser) {
    while parser
        .try_parse(|p| -> Result<(), ()> {
            match p.next() {
                Ok(Token::WhiteSpace(_)) => Ok(()),
                _ => Err(()),
            }
        })
        .is_ok()
    {}
}

/// Skip to next semicolon or end
fn skip_to_semicolon(parser: &mut CssParser) {
    loop {
        match parser.next() {
            Ok(Token::Semicolon) | Err(_) => break,
            _ => {}
        }
    }
}

/// Skip to next rule (after })
fn skip_to_next_rule(parser: &mut CssParser) {
    loop {
        match parser.next() {
            Ok(Token::CurlyBracketBlock) => {
                // Enter nested block
                let _ = parser.parse_nested_block::<_, _, BasicParseErrorKind>(|_| Ok(()));
                break;
            }
            Err(_) => break,
            _ => {}
        }
    }
}

/// Parser for chart selectors
struct ChartParser;

impl<'i> Parser<'i> for ChartParser {
    type Impl = ChartSelectors;
    type Error = selectors::parser::SelectorParseErrorKind<'i>;

    fn parse_non_ts_pseudo_class(
        &self,
        location: cssparser::SourceLocation,
        name: cssparser::CowRcStr<'i>,
    ) -> Result<ChartPseudoClass, cssparser::ParseError<'i, Self::Error>> {
        use ChartPseudoClass::*;

        match name.as_ref() {
            "first-child" => Ok(FirstChild),
            "last-child" => Ok(LastChild),
            "hover" => Ok(Hover),
            "active" => Ok(Active),
            _ if name.starts_with("nth-child") => {
                // Simple nth-child parsing (just handles numbers for now)
                Ok(NthChild(1))
            }
            _ => Err(location.new_custom_error(
                selectors::parser::SelectorParseErrorKind::UnsupportedPseudoClassOrElement(name),
            )),
        }
    }
}
