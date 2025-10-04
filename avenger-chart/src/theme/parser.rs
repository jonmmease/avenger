//! CSS stylesheet parser using cssparser's high-level APIs

use crate::theme::color_mix::parse_color_mix_function;
use crate::theme::css_value::{parse_hsl_function, parse_rgb_function};
use crate::theme::lab_color::{
    parse_lab_function, parse_lch_function, parse_oklab_function, parse_oklch_function,
};
use crate::theme::selector_impl::{ChartPseudoClass, ChartSelectors};
use crate::theme::theme::CompiledRule;
use crate::theme::value::parse_color_string;
use crate::theme::{AngleUnit, CssRgba, LengthUnit, ThemeValue};
use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserInput, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser, Token,
    color::{OPAQUE, parse_hash_color},
};
use indexmap::IndexMap;
use selectors::parser::{ParseRelative, Parser as SelectorParser, SelectorList};
use std::cell::RefCell;

/// Parse a CSS stylesheet into rules
pub fn parse_stylesheet(css: &str) -> Result<Vec<CompiledRule>, String> {
    let mut input = ParserInput::new(css);
    let mut parser = Parser::new(&mut input);
    let unsupported_units = RefCell::new(Vec::new());
    let mut chart_parser = ChartStyleParser::new(&unsupported_units);
    let mut source_order = 0;

    let rules: Vec<CompiledRule> = StyleSheetParser::new(&mut parser, &mut chart_parser)
        .filter_map(|result| {
            match result {
                Ok(mut rule) => {
                    rule.source_order = source_order;
                    source_order += 1;
                    Some(rule)
                }
                Err(_) => None, // Ignore invalid rules
            }
        })
        .collect();

    // Check if any unsupported units were encountered
    let errors = unsupported_units.into_inner();
    if !errors.is_empty() {
        // Deduplicate errors
        let mut unique_units: Vec<String> = errors;
        unique_units.sort();
        unique_units.dedup();
        return Err(format!(
            "Unsupported CSS units: {}",
            unique_units.join(", ")
        ));
    }

    Ok(rules)
}

/// Parser struct that implements the required traits for StyleSheetParser
struct ChartStyleParser<'a> {
    unsupported_units: &'a RefCell<Vec<String>>,
}

impl<'a> ChartStyleParser<'a> {
    fn new(unsupported_units: &'a RefCell<Vec<String>>) -> Self {
        Self { unsupported_units }
    }
}

/// Implementation of QualifiedRuleParser for parsing style rules
impl<'i, 'a> QualifiedRuleParser<'i> for ChartStyleParser<'a> {
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
            unsupported_units: self.unsupported_units,
        };

        // Collect all declaration parsing results, propagating errors
        let results: Result<Vec<_>, _> =
            RuleBodyParser::new(input, &mut declaration_parser).collect();

        // If there were parse errors, convert to ParseError and return
        if let Err((err, _context)) = results {
            return Err(err);
        }

        Ok(CompiledRule {
            selector,
            specificity,
            source_order: 0, // Will be set later
            declarations: declaration_parser.declarations,
        })
    }
}

/// Internal struct for parsing declarations
struct DeclarationParserImpl<'a> {
    declarations: IndexMap<String, ThemeValue>,
    unsupported_units: &'a RefCell<Vec<String>>,
}

/// Implementation of DeclarationParser for parsing CSS property declarations
impl<'i, 'a> DeclarationParser<'i> for DeclarationParserImpl<'a> {
    type Declaration = ();
    type Error = ();

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _start: &ParserState,
    ) -> Result<(), ParseError<'i, ()>> {
        let property = name.to_string();

        // Always try to parse as comma-separated list
        let values =
            input.parse_comma_separated(|p| parse_single_value(p, self.unsupported_units))?;

        // Store based on number of values
        match values.len() {
            0 => return Err(input.new_custom_error(())),
            1 => {
                // Single value - store directly
                self.declarations
                    .insert(property, values.into_iter().next().unwrap());
            }
            _ => {
                // Multiple values - store as list
                self.declarations.insert(property, ThemeValue::List(values));
            }
        }

        Ok(())
    }
}

/// Implementation of AtRuleParser - we don't support at-rules
/// The default implementation rejects all at-rules, which is what we want
impl<'i, 'a> AtRuleParser<'i> for ChartStyleParser<'a> {
    type Prelude = ();
    type AtRule = CompiledRule;
    type Error = ();
    // Using default implementations - they reject all at-rules
}

/// Implementation of RuleBodyItemParser
impl<'i, 'a> RuleBodyItemParser<'i, (), ()> for DeclarationParserImpl<'a> {
    fn parse_declarations(&self) -> bool {
        true // We do parse declarations
    }

    fn parse_qualified(&self) -> bool {
        false // We don't parse nested qualified rules
    }
}

/// We also need AtRuleParser for DeclarationParserImpl
/// Using default implementation which rejects all at-rules
impl<'i, 'a> AtRuleParser<'i> for DeclarationParserImpl<'a> {
    type Prelude = ();
    type AtRule = ();
    type Error = ();
}

/// We also need QualifiedRuleParser for DeclarationParserImpl
/// Using default implementation which rejects all qualified rules
impl<'i, 'a> QualifiedRuleParser<'i> for DeclarationParserImpl<'a> {
    type Prelude = ();
    type QualifiedRule = ();
    type Error = ();
}

/// Convert a CSS token to a ThemeValue
fn token_to_theme_value<'i>(
    token: &Token<'i>,
    unsupported_units: &RefCell<Vec<String>>,
) -> Result<ThemeValue, ()> {
    match token {
        Token::Ident(s) => {
            let s_str = s.to_string();
            // Check for special keywords
            match s_str.as_str() {
                "none" => Ok(ThemeValue::None),
                "initial" => Ok(ThemeValue::Initial),
                "inherit" => Ok(ThemeValue::Inherit),
                _ => {
                    // Always try to parse as color first
                    if let Some(color) = parse_color_string(&s_str) {
                        Ok(ThemeValue::Color(color))
                    } else {
                        Ok(ThemeValue::String(s_str))
                    }
                }
            }
        }
        Token::Number { value, .. } => Ok(ThemeValue::Number(*value as f64)),
        Token::Dimension { value, unit, .. } => {
            let num_value = *value as f64;
            match unit.as_ref() {
                "px" => Ok(ThemeValue::Length(num_value, LengthUnit::Px)),
                "rem" => Ok(ThemeValue::Length(num_value, LengthUnit::Rem)),
                "deg" => Ok(ThemeValue::Angle(num_value, AngleUnit::Deg)),
                "rad" => Ok(ThemeValue::Angle(num_value, AngleUnit::Rad)),
                "grad" => Ok(ThemeValue::Angle(num_value, AngleUnit::Grad)),
                "turn" => Ok(ThemeValue::Angle(num_value, AngleUnit::Turn)),
                unit_str => {
                    // Track unsupported unit
                    unsupported_units.borrow_mut().push(unit_str.to_string());
                    Err(()) // Return error
                }
            }
        }
        Token::Percentage { unit_value, .. } => {
            Ok(ThemeValue::Percentage((*unit_value * 100.0) as f64))
        }
        Token::Hash(h) | Token::IDHash(h) => {
            // Use cssparser's parse_hash_color for efficient hex parsing
            if let Ok((r, g, b, a)) = parse_hash_color(h.as_bytes()) {
                Ok(ThemeValue::Color(CssRgba {
                    red: r,
                    green: g,
                    blue: b,
                    alpha: if a == OPAQUE { 255 } else { (a * 255.0) as u8 },
                }))
            } else {
                Err(())
            }
        }
        Token::QuotedString(s) => {
            let s_str = s.to_string();
            // Quoted strings stay as strings (don't parse as colors)
            Ok(ThemeValue::String(s_str))
        }
        _ => Err(()),
    }
}

/// Parse a single CSS value
fn parse_single_value<'i, 't>(
    parser: &mut Parser<'i, 't>,
    unsupported_units: &RefCell<Vec<String>>,
) -> Result<ThemeValue, ParseError<'i, ()>> {
    parser.skip_whitespace();

    let token = parser.next()?;

    // Special handling for functions and numbers with potential units
    match token {
        Token::Function(name) => {
            let name_str = name.to_string();

            // Special case: color-mix has complex syntax (keywords + values)
            // Special case: lab/lch/oklab/oklch use space-separated values, not commas
            let args = if name_str == "color-mix" {
                parse_color_mix_args(parser, unsupported_units)
                    .map_err(|_| parser.new_custom_error(()))?
            } else if matches!(name_str.as_str(), "lab" | "lch" | "oklab" | "oklch") {
                parse_space_separated_args(parser, unsupported_units)
                    .map_err(|_| parser.new_custom_error(()))?
            } else {
                parse_function_args(parser, unsupported_units)
                    .map_err(|_| parser.new_custom_error(()))?
            };

            match name_str.as_str() {
                "rgb" | "rgba" => {
                    if let Some(color) = parse_rgb_function(&args) {
                        Ok(ThemeValue::Color(color))
                    } else {
                        Ok(ThemeValue::Function(name_str, args))
                    }
                }
                "hsl" | "hsla" => {
                    if let Some(color) = parse_hsl_function(&args) {
                        Ok(ThemeValue::Color(color))
                    } else {
                        Ok(ThemeValue::Function(name_str, args))
                    }
                }
                "color-mix" => {
                    if let Some(color) = parse_color_mix_function(&args) {
                        Ok(ThemeValue::Color(color))
                    } else {
                        Ok(ThemeValue::Function(name_str, args))
                    }
                }
                "oklab" => {
                    if let Some(color) = parse_oklab_function(&args) {
                        Ok(ThemeValue::Color(color))
                    } else {
                        Ok(ThemeValue::Function(name_str, args))
                    }
                }
                "oklch" => {
                    if let Some(color) = parse_oklch_function(&args) {
                        Ok(ThemeValue::Color(color))
                    } else {
                        Ok(ThemeValue::Function(name_str, args))
                    }
                }
                "lab" => {
                    if let Some(color) = parse_lab_function(&args) {
                        Ok(ThemeValue::Color(color))
                    } else {
                        Ok(ThemeValue::Function(name_str, args))
                    }
                }
                "lch" => {
                    if let Some(color) = parse_lch_function(&args) {
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
                "light-dark" => {
                    // light-dark(light-value, dark-value) function
                    if args.len() == 2 {
                        Ok(ThemeValue::LightDark(
                            Box::new(args[0].clone()),
                            Box::new(args[1].clone()),
                        ))
                    } else {
                        Err(parser.new_custom_error(()))
                    }
                }
                _ => Ok(ThemeValue::Function(name_str, args)),
            }
        }
        Token::Number { value, .. } => {
            let num_value = *value as f64;
            // Check for unit - save state first in case it's not a unit
            let state = parser.state();
            match parser.next() {
                Ok(Token::Ident(unit)) => {
                    match unit.as_ref() {
                        "px" => Ok(ThemeValue::Length(num_value, LengthUnit::Px)),
                        "rem" => Ok(ThemeValue::Length(num_value, LengthUnit::Rem)),
                        "%" => Ok(ThemeValue::Percentage(num_value)),
                        unit_str => {
                            // Track unsupported unit
                            unsupported_units.borrow_mut().push(unit_str.to_string());
                            // Unsupported unit - return error
                            Err(parser.new_custom_error(()))
                        }
                    }
                }
                _ => {
                    // No unit follows, reset and treat as plain number
                    parser.reset(&state);
                    Ok(ThemeValue::Number(num_value))
                }
            }
        }
        _ => {
            token_to_theme_value(&token, unsupported_units).map_err(|_| parser.new_custom_error(()))
        }
    }
}

/// Parse function arguments for color-mix (special token-based syntax)
/// color-mix has syntax like: color-mix(in srgb, red 75%, blue 25%)
/// which includes keywords and space-separated tokens
fn parse_color_mix_args<'i, 't>(
    parser: &mut Parser<'i, 't>,
    unsupported_units: &RefCell<Vec<String>>,
) -> Result<Vec<ThemeValue>, ParseError<'i, ()>> {
    parser.parse_nested_block(|p| {
        let mut values = Vec::new();

        loop {
            p.skip_whitespace();

            // Try to get next token
            match p.next() {
                Ok(token) => {
                    match token {
                        Token::Comma => {
                            // Comma is a separator, skip it
                            continue;
                        }
                        Token::Number { value, .. } => {
                            values.push(ThemeValue::Number(*value as f64));
                        }
                        _ => {
                            if let Ok(theme_value) = token_to_theme_value(&token, unsupported_units)
                            {
                                values.push(theme_value);
                            } else {
                                return Err(p.new_custom_error(()));
                            }
                        }
                    }
                }
                Err(_) => {
                    // End of function
                    break;
                }
            }
        }

        Ok(values)
    })
}

/// Parse function arguments
/// Parses comma-separated values, supporting nested function calls
/// Each argument is recursively parsed as a complete value
fn parse_function_args<'i, 't>(
    parser: &mut Parser<'i, 't>,
    unsupported_units: &RefCell<Vec<String>>,
) -> Result<Vec<ThemeValue>, ParseError<'i, ()>> {
    parser.parse_nested_block(|p| {
        // Parse comma-separated list of values
        // Each value is parsed recursively, enabling nested functions like:
        // light-dark(var(--color), #000)
        let values: Vec<ThemeValue> =
            p.parse_comma_separated(|parser| parse_single_value(parser, unsupported_units))?;

        Ok(values)
    })
}

/// Parse space-separated function arguments (for lab/lch/oklab/oklch)
/// These color functions use space-separated values, not commas
/// Example: oklab(0.6 0.1 -0.1) or lab(60 20 -30 / 0.5)
fn parse_space_separated_args<'i, 't>(
    parser: &mut Parser<'i, 't>,
    unsupported_units: &RefCell<Vec<String>>,
) -> Result<Vec<ThemeValue>, ParseError<'i, ()>> {
    parser.parse_nested_block(|p| {
        let mut values = Vec::new();

        // Parse space-separated values until we hit a slash or end
        loop {
            // Try to parse a value
            match parse_single_value(p, unsupported_units) {
                Ok(value) => values.push(value),
                Err(_) => break,
            }

            // Check if next token is a slash (for alpha)
            let state = p.state();
            match p.next() {
                Ok(Token::Delim('/')) => {
                    // Parse alpha value after slash
                    if let Ok(alpha) = parse_single_value(p, unsupported_units) {
                        values.push(alpha);
                    }
                    break;
                }
                _ => {
                    // Not a slash, restore state and continue
                    p.reset(&state);
                }
            }

            // Check if we're at the end
            if p.is_exhausted() {
                break;
            }
        }

        Ok(values)
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
        // No pseudo-classes are supported
        Err(location.new_custom_error(
            selectors::parser::SelectorParseErrorKind::UnsupportedPseudoClassOrElement(name),
        ))
    }

    fn parse_non_ts_functional_pseudo_class<'t>(
        &self,
        name: cssparser::CowRcStr<'i>,
        parser: &mut Parser<'i, 't>,
        _after_part: bool,
    ) -> Result<ChartPseudoClass, cssparser::ParseError<'i, Self::Error>> {
        // No functional pseudo-classes supported
        Err(parser.new_custom_error(
            selectors::parser::SelectorParseErrorKind::UnsupportedPseudoClassOrElement(name),
        ))
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
