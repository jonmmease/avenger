//! CSS stylesheet parser using cssparser's high-level APIs

use super::CompiledRule;
use super::selector_impl::{ChartPseudoClass, ChartSelectors};
use super::value::{parse_hex_color, parse_named_color, parse_rgb_function};
use crate::theme::{LengthUnit, ThemeValue};
use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserInput, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser, Token,
};
use indexmap::IndexMap;
use selectors::parser::{ParseRelative, Parser as SelectorParser, SelectorList};

/// Parse a CSS stylesheet into rules
pub fn parse_stylesheet(css: &str) -> Result<Vec<CompiledRule>, String> {
    let mut input = ParserInput::new(css);
    let mut parser = Parser::new(&mut input);
    let mut chart_parser = ChartStyleParser::new();
    let mut source_order = 0;

    let rules: Vec<CompiledRule> = StyleSheetParser::new(&mut parser, &mut chart_parser)
        .filter_map(|result| {
            match result {
                Ok(mut rule) => {
                    rule.source_order = source_order;
                    source_order += 1;
                    Some(rule)
                }
                Err((_, _)) => None, // Ignore invalid rules
            }
        })
        .collect();

    Ok(rules)
}

/// Parser struct that implements the required traits for StyleSheetParser
struct ChartStyleParser;

impl ChartStyleParser {
    fn new() -> Self {
        Self
    }
}

/// Implementation of QualifiedRuleParser for parsing style rules
impl<'i> QualifiedRuleParser<'i> for ChartStyleParser {
    type Prelude = String;
    type QualifiedRule = CompiledRule;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i, 't>,
    ) -> Result<String, ParseError<'i, ()>> {
        // Capture the starting position
        let start = input.position();

        // Parse everything before the declaration block
        loop {
            let state = input.state();
            match input.next_including_whitespace() {
                Ok(Token::SquareBracketBlock) => {
                    // This is part of the selector (attribute selector)
                    // We need to consume it so slice_from includes it
                    let _ = input.parse_nested_block(|_| Ok::<(), ParseError<'i, ()>>(()));
                }
                Ok(Token::CurlyBracketBlock) => {
                    // Found the declaration block, reset to before it
                    input.reset(&state);
                    break;
                }
                Ok(_) => {
                    // Continue consuming selector tokens
                }
                Err(_) => break,
            }
        }

        // Get the raw selector string from the input
        let selector = input.slice_from(start).trim();

        if selector.is_empty() {
            Err(input.new_custom_error(()))
        } else {
            Ok(selector.to_string())
        }
    }

    fn parse_block<'t>(
        &mut self,
        selector_str: String,
        _start: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<CompiledRule, ParseError<'i, ()>> {
        // Handle :root specially
        let (selector, specificity) = if selector_str == ":root" {
            // Create a universal selector for :root
            let mut selector_input = ParserInput::new("*");
            let mut selector_parser = Parser::new(&mut selector_input);
            let selector_list = SelectorList::parse(
                &ChartSelectorParser,
                &mut selector_parser,
                ParseRelative::No,
            )
            .map_err(|_| input.new_custom_error(()))?;
            let selector = selector_list
                .slice()
                .first()
                .ok_or_else(|| input.new_custom_error(()))?
                .clone();
            (selector, 0)
        } else {
            // Parse selector using selectors crate
            let mut selector_input = ParserInput::new(&selector_str);
            let mut selector_parser = Parser::new(&mut selector_input);
            let selector_list = SelectorList::parse(
                &ChartSelectorParser,
                &mut selector_parser,
                ParseRelative::No,
            )
            .map_err(|_| input.new_custom_error(()))?;
            let selector = selector_list
                .slice()
                .first()
                .ok_or_else(|| input.new_custom_error(()))?
                .clone();
            let specificity = selector.specificity();
            (selector, specificity)
        };

        // Parse declarations using RuleBodyParser
        let mut declaration_parser = DeclarationParserImpl {
            declarations: IndexMap::new(),
        };

        let _ = RuleBodyParser::new(input, &mut declaration_parser)
            .filter_map(|result| result.ok())
            .collect::<Vec<_>>();

        Ok(CompiledRule {
            selector,
            specificity,
            source_order: 0, // Will be set later
            declarations: declaration_parser.declarations,
        })
    }
}

/// Internal struct for parsing declarations
struct DeclarationParserImpl {
    declarations: IndexMap<String, ThemeValue>,
}

/// Implementation of DeclarationParser for parsing CSS property declarations
impl<'i> DeclarationParser<'i> for DeclarationParserImpl {
    type Declaration = ();
    type Error = ();

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _start: &ParserState,
    ) -> Result<(), ParseError<'i, ()>> {
        let property = name.to_string();

        // Parse value
        // For CSS variables and range properties, collect all values
        if property.starts_with("--")
            || property.ends_with("-discrete")
            || property.ends_with("-continuous")
        {
            // Parse comma-separated list
            match parse_value_list(input) {
                Ok(values) => {
                    if values.len() == 1 {
                        self.declarations
                            .insert(property, values.into_iter().next().unwrap());
                    } else {
                        // Join values as a comma-separated string
                        let joined = values
                            .iter()
                            .map(|v| match v {
                                ThemeValue::Color(rgba) => {
                                    format!("#{:02x}{:02x}{:02x}", rgba.red, rgba.green, rgba.blue)
                                }
                                ThemeValue::String(s) | ThemeValue::Keyword(s) => s.clone(),
                                ThemeValue::Double(n) => n.to_string(),
                                ThemeValue::Float(f) => f.to_string(),
                                _ => String::new(),
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                        self.declarations
                            .insert(property, ThemeValue::String(joined));
                    }
                }
                Err(_) => return Err(input.new_custom_error(())),
            }
        } else {
            // Parse single value
            match parse_single_value(input) {
                Ok(value) => {
                    self.declarations.insert(property, value);
                }
                Err(_) => return Err(input.new_custom_error(())),
            }
        }

        Ok(())
    }
}

/// Implementation of AtRuleParser - we don't support at-rules
/// The default implementation rejects all at-rules, which is what we want
impl<'i> AtRuleParser<'i> for ChartStyleParser {
    type Prelude = ();
    type AtRule = CompiledRule;
    type Error = ();
    // Using default implementations - they reject all at-rules
}

/// Implementation of RuleBodyItemParser
impl<'i> RuleBodyItemParser<'i, (), ()> for DeclarationParserImpl {
    fn parse_declarations(&self) -> bool {
        true // We do parse declarations
    }

    fn parse_qualified(&self) -> bool {
        false // We don't parse nested qualified rules
    }
}

/// We also need AtRuleParser for DeclarationParserImpl
/// Using default implementation which rejects all at-rules
impl<'i> AtRuleParser<'i> for DeclarationParserImpl {
    type Prelude = ();
    type AtRule = ();
    type Error = ();
}

/// We also need QualifiedRuleParser for DeclarationParserImpl
/// Using default implementation which rejects all qualified rules
impl<'i> QualifiedRuleParser<'i> for DeclarationParserImpl {
    type Prelude = ();
    type QualifiedRule = ();
    type Error = ();
}

/// Parse a list of CSS values (comma-separated)
fn parse_value_list<'i, 't>(
    parser: &mut Parser<'i, 't>,
) -> Result<Vec<ThemeValue>, ParseError<'i, ()>> {
    let values = parser.parse_comma_separated(|p| parse_single_value(p))?;

    if values.is_empty() {
        Err(parser.new_custom_error(()))
    } else {
        Ok(values)
    }
}

/// Parse a single CSS value
fn parse_single_value<'i, 't>(
    parser: &mut Parser<'i, 't>,
) -> Result<ThemeValue, ParseError<'i, ()>> {
    parser.skip_whitespace();

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
                Err(parser.new_custom_error(()))
            }
        }
        Ok(Token::QuotedString(s)) => Ok(ThemeValue::String(s.to_string())),
        Ok(Token::Function(name)) => {
            let name_str = name.to_string();
            let args = parse_function_args(parser).map_err(|_| parser.new_custom_error(()))?;

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
                        Err(parser.new_custom_error(()))
                    }
                }
                _ => Ok(ThemeValue::Function(name_str, args)),
            }
        }
        _ => Err(parser.new_custom_error(())),
    }
}

/// Parse function arguments
fn parse_function_args<'i, 't>(
    parser: &mut Parser<'i, 't>,
) -> Result<Vec<ThemeValue>, ParseError<'i, ()>> {
    parser.parse_nested_block(|p| {
        p.parse_comma_separated(|parser| {
            parser.skip_whitespace();
            match parser.next() {
                Ok(Token::Number { value, .. }) => Ok(ThemeValue::Double(*value as f64)),
                Ok(Token::Ident(s)) => Ok(ThemeValue::String(s.to_string())),
                Ok(Token::QuotedString(s)) => Ok(ThemeValue::String(s.to_string())),
                _ => Err(parser.new_custom_error(())),
            }
        })
    })
}

// Value parsing functions use cssparser's built-in methods

/// Parser for chart selectors
struct ChartSelectorParser;

impl<'i> SelectorParser<'i> for ChartSelectorParser {
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
            _ => Err(location.new_custom_error(
                selectors::parser::SelectorParseErrorKind::UnsupportedPseudoClassOrElement(name),
            )),
        }
    }

    fn parse_non_ts_functional_pseudo_class<'t>(
        &self,
        name: cssparser::CowRcStr<'i>,
        parser: &mut Parser<'i, 't>,
        _after_part: bool,
    ) -> Result<ChartPseudoClass, cssparser::ParseError<'i, Self::Error>> {
        use ChartPseudoClass::*;

        match name.as_ref() {
            "nth-child" => {
                // Parse the argument (e.g., "2" from nth-child(2))
                let n = parser.expect_integer()?;
                Ok(NthChild(n))
            }
            _ => Err(parser.new_custom_error(
                selectors::parser::SelectorParseErrorKind::UnsupportedPseudoClassOrElement(name),
            )),
        }
    }

    fn default_namespace(&self) -> Option<super::selector_impl::ChartString> {
        None
    }

    fn namespace_for_prefix(
        &self,
        _prefix: &super::selector_impl::ChartString,
    ) -> Option<super::selector_impl::ChartString> {
        None
    }
}
