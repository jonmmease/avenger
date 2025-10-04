//! CSS stylesheet parser using cssparser's high-level APIs

use crate::theme::calc::{CalcLeaf, CalcNode, ChannelKeyword, RoundingStrategy};
use crate::theme::color_mix::parse_color_mix_function;
// Color parsing functions are now imported within _with_origin helper functions
use crate::theme::css_value;
use crate::theme::lab_color;
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

            // Handle calc and math functions first (they need special parsing, not args)
            match name_str.as_str() {
                "calc" => {
                    return parse_calc_expression(parser, unsupported_units)
                        .map(|node| ThemeValue::Calc(Box::new(node)));
                }
                "min" | "max" | "clamp" | "abs" | "sign" | "round" | "mod" | "rem" | "hypot" |
                "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "atan2" |
                "pow" | "sqrt" | "exp" | "log" => {
                    return parser.parse_nested_block(|p| {
                        parse_calc_math_function_args(p, &name_str, unsupported_units)
                    }).map(|node| ThemeValue::Calc(Box::new(node)));
                }
                // Handle color functions with potential "from" syntax
                "oklch" => {
                    return parse_oklch_with_origin(parser, unsupported_units);
                }
                "oklab" => {
                    return parse_oklab_with_origin(parser, unsupported_units);
                }
                "lch" => {
                    return parse_lch_with_origin(parser, unsupported_units);
                }
                "lab" => {
                    return parse_lab_with_origin(parser, unsupported_units);
                }
                "hsl" | "hsla" => {
                    return parse_hsl_with_origin(parser, unsupported_units);
                }
                "hwb" => {
                    return parse_hwb_with_origin(parser, unsupported_units);
                }
                "rgb" | "rgba" => {
                    return parse_rgb_with_origin(parser, unsupported_units);
                }
                _ => {
                    // Continue to regular arg parsing
                }
            }

            // Special case: color-mix has complex syntax (keywords + values)
            let args = if name_str == "color-mix" {
                parse_color_mix_args(parser, unsupported_units)
                    .map_err(|_| parser.new_custom_error(()))?
            } else {
                parse_function_args(parser, unsupported_units)
                    .map_err(|_| parser.new_custom_error(()))?
            };

            match name_str.as_str() {
                // All color functions are now handled earlier with "from" syntax support
                "color-mix" => {
                    if let Some(color) = parse_color_mix_function(&args) {
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

// ============================================================================
// CSS calc() Expression Parsing
// ============================================================================

/// Parse a calc() expression
///
/// Entry point for parsing CSS calc() expressions. Follows CSS Values and Units
/// Module Level 4 specification.
///
/// Grammar (simplified):
/// ```text
/// calc() = calc( <calc-sum> )
/// <calc-sum> = <calc-product> [ [ '+' | '-' ] <calc-product> ]*
/// <calc-product> = <calc-value> [ [ '*' | '/' ] <calc-value> ]*
/// <calc-value> = <number> | <dimension> | <percentage> | <calc-constant> | ( <calc-sum> ) | <math-function>
/// ```
fn parse_calc_expression<'i, 't>(
    parser: &mut Parser<'i, 't>,
    unsupported_units: &RefCell<Vec<String>>,
) -> Result<CalcNode, ParseError<'i, ()>> {
    parser.parse_nested_block(|p| parse_calc_sum(p, unsupported_units))
}

/// Parse sum/difference (lowest precedence)
///
/// CSS requires whitespace around + and - operators in calc()
/// Examples: calc(10px + 5px), calc(100% - 20px)
fn parse_calc_sum<'i, 't>(
    parser: &mut Parser<'i, 't>,
    unsupported_units: &RefCell<Vec<String>>,
) -> Result<CalcNode, ParseError<'i, ()>> {
    let mut terms = vec![parse_calc_product(parser, unsupported_units)?];

    loop {
        parser.skip_whitespace();

        let state = parser.state();
        match parser.next() {
            Ok(Token::Delim('+')) => {
                // Require whitespace before the operator (already consumed by skip_whitespace)
                // Check for whitespace after
                let next_state = parser.state();
                if let Ok(token) = parser.next_including_whitespace() {
                    if !matches!(token, Token::WhiteSpace(_)) {
                        parser.reset(&next_state);
                    }
                }

                let term = parse_calc_product(parser, unsupported_units)?;
                terms.push(term);
            }
            Ok(Token::Delim('-')) => {
                // Require whitespace before the operator (already consumed by skip_whitespace)
                // Check for whitespace after
                let next_state = parser.state();
                if let Ok(token) = parser.next_including_whitespace() {
                    if !matches!(token, Token::WhiteSpace(_)) {
                        parser.reset(&next_state);
                    }
                }

                let term = parse_calc_product(parser, unsupported_units)?;
                // Subtraction is represented as addition of negated term
                terms.push(CalcNode::Negate(Box::new(term)));
            }
            _ => {
                parser.reset(&state);
                break;
            }
        }
    }

    if terms.len() == 1 {
        Ok(terms.into_iter().next().unwrap())
    } else {
        Ok(CalcNode::Sum(terms))
    }
}

/// Parse multiplication/division (higher precedence)
///
/// No whitespace is required around * and / operators
/// Examples: calc(10px*2), calc(100%/2)
fn parse_calc_product<'i, 't>(
    parser: &mut Parser<'i, 't>,
    unsupported_units: &RefCell<Vec<String>>,
) -> Result<CalcNode, ParseError<'i, ()>> {
    let mut factors = vec![parse_calc_value(parser, unsupported_units)?];

    loop {
        parser.skip_whitespace();

        let state = parser.state();
        match parser.next() {
            Ok(Token::Delim('*')) => {
                parser.skip_whitespace();
                let factor = parse_calc_value(parser, unsupported_units)?;
                factors.push(factor);
            }
            Ok(Token::Delim('/')) => {
                parser.skip_whitespace();
                let divisor = parse_calc_value(parser, unsupported_units)?;
                // Division is represented as multiplication by inverted factor
                factors.push(CalcNode::Invert(Box::new(divisor)));
            }
            _ => {
                parser.reset(&state);
                break;
            }
        }
    }

    if factors.len() == 1 {
        Ok(factors.into_iter().next().unwrap())
    } else {
        Ok(CalcNode::Product(factors))
    }
}

/// Parse a single calc value (highest precedence)
///
/// Handles: numbers, dimensions, percentages, parentheses, constants, var(), channel keywords
fn parse_calc_value<'i, 't>(
    parser: &mut Parser<'i, 't>,
    unsupported_units: &RefCell<Vec<String>>,
) -> Result<CalcNode, ParseError<'i, ()>> {
    parser.skip_whitespace();

    let state = parser.state();
    match parser.next()? {
        Token::Number { value, .. } => {
            Ok(CalcNode::Leaf(CalcLeaf::Number(*value as f64)))
        }
        Token::Dimension { value, unit, .. } => {
            let num_value = *value as f64;
            match unit.as_ref() {
                "px" => Ok(CalcNode::Leaf(CalcLeaf::Length(num_value, LengthUnit::Px))),
                "rem" => Ok(CalcNode::Leaf(CalcLeaf::Length(num_value, LengthUnit::Rem))),
                "deg" => Ok(CalcNode::Leaf(CalcLeaf::Angle(num_value, AngleUnit::Deg))),
                "rad" => Ok(CalcNode::Leaf(CalcLeaf::Angle(num_value, AngleUnit::Rad))),
                "grad" => Ok(CalcNode::Leaf(CalcLeaf::Angle(num_value, AngleUnit::Grad))),
                "turn" => Ok(CalcNode::Leaf(CalcLeaf::Angle(num_value, AngleUnit::Turn))),
                unit_str => {
                    unsupported_units.borrow_mut().push(unit_str.to_string());
                    Err(parser.new_custom_error(()))
                }
            }
        }
        Token::Percentage { unit_value, .. } => {
            Ok(CalcNode::Leaf(CalcLeaf::Percentage((*unit_value * 100.0) as f64)))
        }
        Token::Ident(ident) => {
            // Check for calc constants
            match ident.as_ref() {
                "pi" => Ok(CalcNode::Leaf(CalcLeaf::Number(std::f64::consts::PI))),
                "e" => Ok(CalcNode::Leaf(CalcLeaf::Number(std::f64::consts::E))),
                "infinity" => Ok(CalcNode::Leaf(CalcLeaf::Number(f64::INFINITY))),
                "-infinity" => Ok(CalcNode::Leaf(CalcLeaf::Number(f64::NEG_INFINITY))),
                "nan" => Ok(CalcNode::Leaf(CalcLeaf::Number(f64::NAN))),
                // Check for channel keywords (for relative color syntax)
                _ => {
                    if let Some(keyword) = ChannelKeyword::from_ident(ident.as_ref()) {
                        Ok(CalcNode::Leaf(CalcLeaf::ChannelKeyword(keyword)))
                    } else {
                        Err(parser.new_custom_error(()))
                    }
                }
            }
        }
        Token::ParenthesisBlock => {
            // Nested expression in parentheses
            parser.parse_nested_block(|p| parse_calc_sum(p, unsupported_units))
        }
        Token::Function(name) => {
            let name_str = name.to_string();
            parser.reset(&state);

            // Parse the function (it will consume the function token again)
            match parser.next() {
                Ok(Token::Function(_)) => {
                    match name_str.as_str() {
                        "var" => {
                            // CSS variable reference
                            parser.parse_nested_block(|p| {
                                // Parse variable name (should be an ident with --)
                                match p.next()? {
                                    Token::Ident(var_name) => {
                                        // Store with -- prefix if present, or add it
                                        let full_name = if var_name.starts_with("--") {
                                            var_name.to_string()
                                        } else {
                                            format!("--{}", var_name)
                                        };
                                        Ok(CalcNode::Leaf(CalcLeaf::Variable(full_name)))
                                    }
                                    _ => Err(p.new_custom_error(()))
                                }
                            })
                        }
                        // Math functions
                        _ => {
                            parser.parse_nested_block(|p| {
                                parse_calc_math_function_args(p, &name_str, unsupported_units)
                            })
                        }
                    }
                }
                _ => Err(parser.new_custom_error(()))
            }
        }
        _ => Err(parser.new_custom_error(())),
    }
}

/// Parse math function arguments (min, max, clamp, abs, sign, etc.)
/// This is called from within an already-entered nested block
fn parse_calc_math_function_args<'i, 't>(
    p: &mut Parser<'i, 't>,
    fn_name: &str,
    unsupported_units: &RefCell<Vec<String>>,
) -> Result<CalcNode, ParseError<'i, ()>> {
    match fn_name {
            "min" => {
                let args = p.parse_comma_separated(|p| parse_calc_sum(p, unsupported_units))?;
                if args.is_empty() {
                    return Err(p.new_custom_error(()));
                }
                Ok(CalcNode::Min(args))
            }
            "max" => {
                let args = p.parse_comma_separated(|p| parse_calc_sum(p, unsupported_units))?;
                if args.is_empty() {
                    return Err(p.new_custom_error(()));
                }
                Ok(CalcNode::Max(args))
            }
            "clamp" => {
                let args = p.parse_comma_separated(|p| parse_calc_sum(p, unsupported_units))?;
                if args.len() != 3 {
                    return Err(p.new_custom_error(()));
                }
                let mut iter = args.into_iter();
                Ok(CalcNode::Clamp {
                    min: Box::new(iter.next().unwrap()),
                    center: Box::new(iter.next().unwrap()),
                    max: Box::new(iter.next().unwrap()),
                })
            }
            "abs" => {
                let arg = parse_calc_sum(p, unsupported_units)?;
                Ok(CalcNode::Abs(Box::new(arg)))
            }
            "sign" => {
                let arg = parse_calc_sum(p, unsupported_units)?;
                Ok(CalcNode::Sign(Box::new(arg)))
            }
            "round" => {
                let args = p.parse_comma_separated(|p| parse_calc_sum(p, unsupported_units))?;
                if args.len() == 2 {
                    // round(value, step) - defaults to nearest
                    let mut iter = args.into_iter();
                    Ok(CalcNode::Round {
                        strategy: RoundingStrategy::Nearest,
                        value: Box::new(iter.next().unwrap()),
                        step: Box::new(iter.next().unwrap()),
                    })
                } else if args.len() == 3 {
                    // round(strategy, value, step)
                    let mut iter = args.into_iter();
                    let strategy_node = iter.next().unwrap();

                    // Extract strategy from node (should be an ident)
                    let strategy = match strategy_node {
                        CalcNode::Leaf(CalcLeaf::Number(_)) => RoundingStrategy::Nearest,
                        _ => RoundingStrategy::Nearest, // Default if not recognized
                    };

                    Ok(CalcNode::Round {
                        strategy,
                        value: Box::new(iter.next().unwrap()),
                        step: Box::new(iter.next().unwrap()),
                    })
                } else {
                    Err(p.new_custom_error(()))
                }
            }
            "mod" => {
                let args = p.parse_comma_separated(|p| parse_calc_sum(p, unsupported_units))?;
                if args.len() != 2 {
                    return Err(p.new_custom_error(()));
                }
                let mut iter = args.into_iter();
                Ok(CalcNode::Mod {
                    dividend: Box::new(iter.next().unwrap()),
                    divisor: Box::new(iter.next().unwrap()),
                })
            }
            "rem" => {
                let args = p.parse_comma_separated(|p| parse_calc_sum(p, unsupported_units))?;
                if args.len() != 2 {
                    return Err(p.new_custom_error(()));
                }
                let mut iter = args.into_iter();
                Ok(CalcNode::Rem {
                    dividend: Box::new(iter.next().unwrap()),
                    divisor: Box::new(iter.next().unwrap()),
                })
            }
            "hypot" => {
                let args = p.parse_comma_separated(|p| parse_calc_sum(p, unsupported_units))?;
                if args.is_empty() {
                    return Err(p.new_custom_error(()));
                }
                Ok(CalcNode::Hypot(args))
            }
            "sin" => {
                let arg = parse_calc_sum(p, unsupported_units)?;
                Ok(CalcNode::Sin(Box::new(arg)))
            }
            "cos" => {
                let arg = parse_calc_sum(p, unsupported_units)?;
                Ok(CalcNode::Cos(Box::new(arg)))
            }
            "tan" => {
                let arg = parse_calc_sum(p, unsupported_units)?;
                Ok(CalcNode::Tan(Box::new(arg)))
            }
            "asin" => {
                let arg = parse_calc_sum(p, unsupported_units)?;
                Ok(CalcNode::Asin(Box::new(arg)))
            }
            "acos" => {
                let arg = parse_calc_sum(p, unsupported_units)?;
                Ok(CalcNode::Acos(Box::new(arg)))
            }
            "atan" => {
                let arg = parse_calc_sum(p, unsupported_units)?;
                Ok(CalcNode::Atan(Box::new(arg)))
            }
            "atan2" => {
                let args = p.parse_comma_separated(|p| parse_calc_sum(p, unsupported_units))?;
                if args.len() != 2 {
                    return Err(p.new_custom_error(()));
                }
                let mut iter = args.into_iter();
                Ok(CalcNode::Atan2 {
                    y: Box::new(iter.next().unwrap()),
                    x: Box::new(iter.next().unwrap()),
                })
            }
            "pow" => {
                let args = p.parse_comma_separated(|p| parse_calc_sum(p, unsupported_units))?;
                if args.len() != 2 {
                    return Err(p.new_custom_error(()));
                }
                let mut iter = args.into_iter();
                Ok(CalcNode::Pow {
                    base: Box::new(iter.next().unwrap()),
                    exponent: Box::new(iter.next().unwrap()),
                })
            }
            "sqrt" => {
                let arg = parse_calc_sum(p, unsupported_units)?;
                Ok(CalcNode::Sqrt(Box::new(arg)))
            }
            "exp" => {
                let arg = parse_calc_sum(p, unsupported_units)?;
                Ok(CalcNode::Exp(Box::new(arg)))
            }
            "log" => {
                let args = p.parse_comma_separated(|p| parse_calc_sum(p, unsupported_units))?;
                if args.len() == 1 {
                    // log(value) - natural log
                    Ok(CalcNode::Log {
                        value: Box::new(args.into_iter().next().unwrap()),
                        base: None,
                    })
                } else if args.len() == 2 {
                    // log(value, base)
                    let mut iter = args.into_iter();
                    Ok(CalcNode::Log {
                        value: Box::new(iter.next().unwrap()),
                        base: Some(Box::new(iter.next().unwrap())),
                    })
                } else {
                    Err(p.new_custom_error(()))
                }
            }
            _ => Err(p.new_custom_error(())),
    }
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

/// Try to parse "from <color>" prefix for relative color syntax
///
/// Returns Some(origin_color) if "from" keyword is found, None otherwise
fn try_parse_origin_color<'i, 't>(
    parser: &mut Parser<'i, 't>,
    unsupported_units: &RefCell<Vec<String>>,
) -> Result<Option<ThemeValue>, ParseError<'i, ()>> {
    // Save state to restore if we don't find "from"
    let state = parser.state();

    // Try to match "from" keyword
    match parser.next() {
        Ok(Token::Ident(ident)) if ident.eq_ignore_ascii_case("from") => {
            // Parse the origin color
            let color = parse_single_value(parser, unsupported_units)?;
            Ok(Some(color))
        }
        _ => {
            // Not "from", restore state
            parser.reset(&state);
            Ok(None)
        }
    }
}

/// Parse a color component (number, percentage, angle, calc, or channel keyword)
fn parse_color_component<'i, 't>(
    parser: &mut Parser<'i, 't>,
    unsupported_units: &RefCell<Vec<String>>,
) -> Result<super::color_component::ColorComponent, ParseError<'i, ()>> {
    parse_color_component_with_space(parser, unsupported_units, None)
}

fn parse_color_component_with_space<'i, 't>(
    parser: &mut Parser<'i, 't>,
    unsupported_units: &RefCell<Vec<String>>,
    color_space: Option<crate::color::types::ColorSpace>,
) -> Result<super::color_component::ColorComponent, ParseError<'i, ()>> {
    use super::calc::ChannelKeyword;
    use super::color_component::ColorComponent;

    // Try to parse a value
    let value = parse_single_value(parser, unsupported_units)?;

    // Convert ThemeValue to ColorComponent
    match &value {
        // Check for channel keywords (bare identifiers)
        ThemeValue::String(s) => {
            if s.eq_ignore_ascii_case("none") {
                return Ok(ColorComponent::None);
            }
            if let Some(keyword) = ChannelKeyword::from_ident_with_color_space(s, color_space) {
                return Ok(ColorComponent::ChannelKeyword(keyword));
            }
            // Fall through to normal conversion
            ColorComponent::from_theme_value(&value)
                .map_err(|_| parser.new_custom_error(()))
        }
        _ => ColorComponent::from_theme_value(&value)
            .map_err(|_| parser.new_custom_error(())),
    }
}

/// Parse oklch() function with optional "from <origin>" syntax
fn parse_oklch_with_origin<'i, 't>(
    parser: &mut Parser<'i, 't>,
    unsupported_units: &RefCell<Vec<String>>,
) -> Result<ThemeValue, ParseError<'i, ()>> {
    use super::color_component::ColorComponent;
    use crate::color::types::ColorSpace;

    parser.parse_nested_block(|p| {
        // Try to parse "from <origin>"
        let origin = try_parse_origin_color(p, unsupported_units)?;

        if let Some(origin_color) = origin {
            // Relative color syntax
            let lightness = parse_color_component(p, unsupported_units)?;
            let chroma = parse_color_component(p, unsupported_units)?;
            let hue = parse_color_component(p, unsupported_units)?;

            // Parse optional alpha
            let alpha = if p.try_parse(|p| p.expect_delim('/')).is_ok() {
                parse_color_component(p, unsupported_units)?
            } else {
                // Default: inherit alpha from origin
                ColorComponent::ChannelKeyword(super::calc::ChannelKeyword::Alpha)
            };

            Ok(ThemeValue::RelativeColor {
                space: ColorSpace::Oklch,
                origin: Box::new(origin_color),
                lightness,
                component1: chroma,
                component2: hue,
                alpha,
            })
        } else {
            // Absolute color syntax - parse as space-separated values
            let mut values = Vec::new();

            // Parse space-separated values
            loop {
                match parse_single_value(p, unsupported_units) {
                    Ok(value) => values.push(value),
                    Err(_) => break,
                }

                // Check for slash (alpha separator)
                let state = p.state();
                match p.next() {
                    Ok(Token::Delim('/')) => {
                        if let Ok(alpha) = parse_single_value(p, unsupported_units) {
                            values.push(alpha);
                        }
                        break;
                    }
                    _ => p.reset(&state),
                }
            }

            // Try to parse as absolute color
            if let Some(color) = lab_color::parse_oklch_function(&values) {
                Ok(ThemeValue::Color(color))
            } else {
                Ok(ThemeValue::Function("oklch".to_string(), values))
            }
        }
    })
}

/// Parse oklab() function with optional "from <origin>" syntax
fn parse_oklab_with_origin<'i, 't>(
    parser: &mut Parser<'i, 't>,
    unsupported_units: &RefCell<Vec<String>>,
) -> Result<ThemeValue, ParseError<'i, ()>> {
    use super::color_component::ColorComponent;
    use crate::color::types::ColorSpace;

    parser.parse_nested_block(|p| {
        let origin = try_parse_origin_color(p, unsupported_units)?;

        if let Some(origin_color) = origin {
            // Relative color syntax: oklab(from <origin> L A B [/ alpha])
            let lightness = parse_color_component_with_space(p, unsupported_units, Some(ColorSpace::Oklab))?;
            let a_component = parse_color_component_with_space(p, unsupported_units, Some(ColorSpace::Oklab))?;
            let b_component = parse_color_component_with_space(p, unsupported_units, Some(ColorSpace::Oklab))?;

            let alpha = if p.try_parse(|p| p.expect_delim('/')).is_ok() {
                parse_color_component_with_space(p, unsupported_units, Some(ColorSpace::Oklab))?
            } else {
                ColorComponent::ChannelKeyword(super::calc::ChannelKeyword::Alpha)
            };

            Ok(ThemeValue::RelativeColor {
                space: ColorSpace::Oklab,
                origin: Box::new(origin_color),
                lightness,
                component1: a_component,
                component2: b_component,
                alpha,
            })
        } else {
            // Absolute color syntax
            let mut values = Vec::new();
            loop {
                match parse_single_value(p, unsupported_units) {
                    Ok(value) => values.push(value),
                    Err(_) => break,
                }
                let state = p.state();
                match p.next() {
                    Ok(Token::Delim('/')) => {
                        if let Ok(alpha) = parse_single_value(p, unsupported_units) {
                            values.push(alpha);
                        }
                        break;
                    }
                    _ => p.reset(&state),
                }
            }

            if let Some(color) = lab_color::parse_oklab_function(&values) {
                Ok(ThemeValue::Color(color))
            } else {
                Ok(ThemeValue::Function("oklab".to_string(), values))
            }
        }
    })
}

/// Parse lch() function with optional "from <origin>" syntax
fn parse_lch_with_origin<'i, 't>(
    parser: &mut Parser<'i, 't>,
    unsupported_units: &RefCell<Vec<String>>,
) -> Result<ThemeValue, ParseError<'i, ()>> {
    use super::color_component::ColorComponent;
    use crate::color::types::ColorSpace;

    parser.parse_nested_block(|p| {
        let origin = try_parse_origin_color(p, unsupported_units)?;

        if let Some(origin_color) = origin {
            // Relative color syntax: lch(from <origin> L C H [/ alpha])
            let lightness = parse_color_component(p, unsupported_units)?;
            let chroma = parse_color_component(p, unsupported_units)?;
            let hue = parse_color_component(p, unsupported_units)?;

            let alpha = if p.try_parse(|p| p.expect_delim('/')).is_ok() {
                parse_color_component(p, unsupported_units)?
            } else {
                ColorComponent::ChannelKeyword(super::calc::ChannelKeyword::Alpha)
            };

            Ok(ThemeValue::RelativeColor {
                space: ColorSpace::Lch,
                origin: Box::new(origin_color),
                lightness,
                component1: chroma,
                component2: hue,
                alpha,
            })
        } else {
            // Absolute color syntax
            let mut values = Vec::new();
            loop {
                match parse_single_value(p, unsupported_units) {
                    Ok(value) => values.push(value),
                    Err(_) => break,
                }
                let state = p.state();
                match p.next() {
                    Ok(Token::Delim('/')) => {
                        if let Ok(alpha) = parse_single_value(p, unsupported_units) {
                            values.push(alpha);
                        }
                        break;
                    }
                    _ => p.reset(&state),
                }
            }

            if let Some(color) = lab_color::parse_lch_function(&values) {
                Ok(ThemeValue::Color(color))
            } else {
                Ok(ThemeValue::Function("lch".to_string(), values))
            }
        }
    })
}

/// Parse lab() function with optional "from <origin>" syntax
fn parse_lab_with_origin<'i, 't>(
    parser: &mut Parser<'i, 't>,
    unsupported_units: &RefCell<Vec<String>>,
) -> Result<ThemeValue, ParseError<'i, ()>> {
    use super::color_component::ColorComponent;
    use crate::color::types::ColorSpace;

    parser.parse_nested_block(|p| {
        let origin = try_parse_origin_color(p, unsupported_units)?;

        if let Some(origin_color) = origin {
            // Relative color syntax: lab(from <origin> L A B [/ alpha])
            let lightness = parse_color_component_with_space(p, unsupported_units, Some(ColorSpace::Lab))?;
            let a_component = parse_color_component_with_space(p, unsupported_units, Some(ColorSpace::Lab))?;
            let b_component = parse_color_component_with_space(p, unsupported_units, Some(ColorSpace::Lab))?;

            let alpha = if p.try_parse(|p| p.expect_delim('/')).is_ok() {
                parse_color_component_with_space(p, unsupported_units, Some(ColorSpace::Lab))?
            } else {
                ColorComponent::ChannelKeyword(super::calc::ChannelKeyword::Alpha)
            };

            Ok(ThemeValue::RelativeColor {
                space: ColorSpace::Lab,
                origin: Box::new(origin_color),
                lightness,
                component1: a_component,
                component2: b_component,
                alpha,
            })
        } else {
            // Absolute color syntax
            let mut values = Vec::new();
            loop {
                match parse_single_value(p, unsupported_units) {
                    Ok(value) => values.push(value),
                    Err(_) => break,
                }
                let state = p.state();
                match p.next() {
                    Ok(Token::Delim('/')) => {
                        if let Ok(alpha) = parse_single_value(p, unsupported_units) {
                            values.push(alpha);
                        }
                        break;
                    }
                    _ => p.reset(&state),
                }
            }

            if let Some(color) = lab_color::parse_lab_function(&values) {
                Ok(ThemeValue::Color(color))
            } else {
                Ok(ThemeValue::Function("lab".to_string(), values))
            }
        }
    })
}

/// Parse hsl() function with optional "from <origin>" syntax
fn parse_hsl_with_origin<'i, 't>(
    parser: &mut Parser<'i, 't>,
    unsupported_units: &RefCell<Vec<String>>,
) -> Result<ThemeValue, ParseError<'i, ()>> {
    use super::color_component::ColorComponent;
    use crate::color::types::ColorSpace;

    parser.parse_nested_block(|p| {
        let origin = try_parse_origin_color(p, unsupported_units)?;

        if let Some(origin_color) = origin {
            // Relative color syntax: hsl(from <origin> H S L [/ alpha])
            let hue = parse_color_component(p, unsupported_units)?;
            let saturation = parse_color_component(p, unsupported_units)?;
            let lightness = parse_color_component(p, unsupported_units)?;

            let alpha = if p.try_parse(|p| p.expect_delim('/')).is_ok() {
                parse_color_component(p, unsupported_units)?
            } else {
                ColorComponent::ChannelKeyword(super::calc::ChannelKeyword::Alpha)
            };

            Ok(ThemeValue::RelativeColor {
                space: ColorSpace::Hsl,
                origin: Box::new(origin_color),
                lightness,
                component1: saturation,
                component2: hue,
                alpha,
            })
        } else {
            // Absolute color syntax - parse comma-separated args
            let args = p.parse_comma_separated(|p| parse_single_value(p, unsupported_units))?;

            if let Some(color) = css_value::parse_hsl_function(&args) {
                Ok(ThemeValue::Color(color))
            } else {
                Ok(ThemeValue::Function("hsl".to_string(), args))
            }
        }
    })
}

/// Parse hwb() function with optional "from <origin>" syntax
fn parse_hwb_with_origin<'i, 't>(
    parser: &mut Parser<'i, 't>,
    unsupported_units: &RefCell<Vec<String>>,
) -> Result<ThemeValue, ParseError<'i, ()>> {
    use super::color_component::ColorComponent;
    use crate::color::types::ColorSpace;

    parser.parse_nested_block(|p| {
        let origin = try_parse_origin_color(p, unsupported_units)?;

        if let Some(origin_color) = origin {
            // Relative color syntax: hwb(from <origin> H W B [/ alpha])
            let hue = parse_color_component_with_space(p, unsupported_units, Some(ColorSpace::Hwb))?;
            let whiteness = parse_color_component_with_space(p, unsupported_units, Some(ColorSpace::Hwb))?;
            let blackness = parse_color_component_with_space(p, unsupported_units, Some(ColorSpace::Hwb))?;

            let alpha = if p.try_parse(|p| p.expect_delim('/')).is_ok() {
                parse_color_component_with_space(p, unsupported_units, Some(ColorSpace::Hwb))?
            } else {
                ColorComponent::ChannelKeyword(super::calc::ChannelKeyword::Alpha)
            };

            Ok(ThemeValue::RelativeColor {
                space: ColorSpace::Hwb,
                origin: Box::new(origin_color),
                lightness: hue,
                component1: whiteness,
                component2: blackness,
                alpha,
            })
        } else {
            // Absolute color syntax - hwb not currently supported as absolute
            let mut values = Vec::new();
            loop {
                match parse_single_value(p, unsupported_units) {
                    Ok(value) => values.push(value),
                    Err(_) => break,
                }
                let state = p.state();
                match p.next() {
                    Ok(Token::Delim('/')) => {
                        if let Ok(alpha) = parse_single_value(p, unsupported_units) {
                            values.push(alpha);
                        }
                        break;
                    }
                    _ => p.reset(&state),
                }
            }
            Ok(ThemeValue::Function("hwb".to_string(), values))
        }
    })
}

/// Parse rgb() function with optional "from <origin>" syntax
fn parse_rgb_with_origin<'i, 't>(
    parser: &mut Parser<'i, 't>,
    unsupported_units: &RefCell<Vec<String>>,
) -> Result<ThemeValue, ParseError<'i, ()>> {
    use super::color_component::ColorComponent;
    use crate::color::types::ColorSpace;

    parser.parse_nested_block(|p| {
        let origin = try_parse_origin_color(p, unsupported_units)?;

        if let Some(origin_color) = origin {
            // Relative color syntax: rgb(from <origin> R G B [/ alpha])
            let red = parse_color_component(p, unsupported_units)?;
            let green = parse_color_component(p, unsupported_units)?;
            let blue = parse_color_component(p, unsupported_units)?;

            let alpha = if p.try_parse(|p| p.expect_delim('/')).is_ok() {
                parse_color_component(p, unsupported_units)?
            } else {
                ColorComponent::ChannelKeyword(super::calc::ChannelKeyword::Alpha)
            };

            Ok(ThemeValue::RelativeColor {
                space: ColorSpace::Srgb,
                origin: Box::new(origin_color),
                lightness: red,
                component1: green,
                component2: blue,
                alpha,
            })
        } else {
            // Absolute color syntax - parse comma-separated args
            let args = p.parse_comma_separated(|p| parse_single_value(p, unsupported_units))?;

            if let Some(color) = css_value::parse_rgb_function(&args) {
                Ok(ThemeValue::Color(color))
            } else {
                Ok(ThemeValue::Function("rgb".to_string(), args))
            }
        }
    })
}
