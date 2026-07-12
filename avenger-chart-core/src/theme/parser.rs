//! CSS stylesheet parser using cssparser's high-level APIs

use std::cell::RefCell;

use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserInput, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser, Token,
    color::{OPAQUE, parse_hash_color},
    parse_important,
};
use indexmap::IndexMap;
use selectors::parser::{ParseRelative, Parser as SelectorParser, SelectorList};
use tracing::warn;

use avenger_color::{ColorChannel, ColorSpace};

use crate::theme::{
    AngleUnit, CssRgba, LengthUnit, ThemeValue,
    calc::{CalcLeaf, CalcNode, RoundingStrategy},
    color_component::ColorComponent,
    css_value, lab_color,
    selector_impl::{ChartPseudoClass, ChartSelectors},
    theme::CompiledRule,
    value::parse_color_string,
};

/// Parse a CSS stylesheet into rules
pub fn parse_stylesheet(css: &str) -> Result<Vec<CompiledRule>, String> {
    let mut input = ParserInput::new(css);
    let mut parser = Parser::new(&mut input);
    let unsupported_units = RefCell::new(Vec::new());
    let mut chart_parser = ChartStyleParser::new(&unsupported_units);
    let mut source_order = 0;

    let mut all_rules = Vec::new();
    let mut parse_errors = Vec::new();

    for result in StyleSheetParser::new(&mut parser, &mut chart_parser) {
        match result {
            Ok(rule_or_rules) => {
                // Handle both single rules and @media blocks (which return Vec<CompiledRule>)
                match rule_or_rules {
                    RuleOrRules::Rule(mut rule) => {
                        rule.source_order = source_order;
                        source_order += 1;
                        all_rules.push(rule);
                    }
                    RuleOrRules::Rules(rules) => {
                        // Rules from @media block
                        for mut rule in rules {
                            rule.source_order = source_order;
                            source_order += 1;
                            all_rules.push(rule);
                        }
                    }
                }
            }
            Err((error, context)) => {
                parse_errors.push(format_parse_error("stylesheet", &error, context));
            }
        }
    }

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

    if !parse_errors.is_empty() {
        return Err(parse_errors.join("; "));
    }

    Ok(all_rules)
}

fn format_parse_error(path: &str, error: &ParseError<'_, ()>, context: &str) -> String {
    let location = error.location;
    let context = context.trim();
    let context = if context.is_empty() {
        String::new()
    } else {
        format!(" near `{context}`")
    };
    format!(
        "CSS parse error at {path} (line {}, column {}){}",
        location.line, location.column, context
    )
}

/// Enum to handle both single rules and rule collections from @media
enum RuleOrRules {
    Rule(CompiledRule),
    Rules(Vec<CompiledRule>),
}

impl From<CompiledRule> for RuleOrRules {
    fn from(rule: CompiledRule) -> Self {
        RuleOrRules::Rule(rule)
    }
}

impl From<Vec<CompiledRule>> for RuleOrRules {
    fn from(rules: Vec<CompiledRule>) -> Self {
        RuleOrRules::Rules(rules)
    }
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
    type QualifiedRule = RuleOrRules;
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
    ) -> Result<RuleOrRules, ParseError<'i, ()>> {
        // Parse selector list
        let selector_list = if selector_str == ":root" {
            // Handle :root specially - create a universal selector
            let mut selector_input = ParserInput::new("*");
            let mut selector_parser = Parser::new(&mut selector_input);
            SelectorList::parse(
                &ChartSelectorParser,
                &mut selector_parser,
                ParseRelative::No,
            )
            .map_err(|_| input.new_custom_error(()))?
        } else {
            // Parse selector using selectors crate
            let mut selector_input = ParserInput::new(&selector_str);
            let mut selector_parser = Parser::new(&mut selector_input);
            SelectorList::parse(
                &ChartSelectorParser,
                &mut selector_parser,
                ParseRelative::No,
            )
            .map_err(|_| input.new_custom_error(()))?
        };

        // Parse declarations using RuleBodyParser
        let mut declaration_parser = DeclarationParserImpl {
            declarations: IndexMap::new(),
            unsupported_units: self.unsupported_units,
        };

        // Collect all declaration parsing results, propagating errors
        for result in RuleBodyParser::new(input, &mut declaration_parser) {
            if let Err((err, context)) = result {
                warn!(
                    selector = selector_str,
                    error = %format_parse_error("declaration", &err, context),
                    "Failed to parse CSS declaration"
                );
                return Err(err);
            }
        }

        // Create one rule per selector in the selector list
        let selectors = selector_list.slice();
        if selectors.is_empty() {
            return Err(input.new_custom_error(()));
        }

        let rules: Vec<CompiledRule> = selectors
            .iter()
            .map(|selector| {
                let specificity = if selector_str == ":root" {
                    0
                } else {
                    selector.specificity()
                };

                CompiledRule {
                    selector: selector.clone(),
                    specificity,
                    source_order: 0, // Will be set later
                    declarations: declaration_parser.declarations.clone(),
                    media_condition: None, // No media query for now (will be added in parse_stylesheet)
                    is_root_rule: selector_str == ":root",
                }
            })
            .collect();

        // Return single rule or multiple rules
        if rules.len() == 1 {
            Ok(RuleOrRules::Rule(rules.into_iter().next().unwrap()))
        } else {
            Ok(RuleOrRules::Rules(rules))
        }
    }
}

/// Internal struct for parsing declarations
struct DeclarationParserImpl<'a> {
    declarations: IndexMap<String, crate::theme::theme::Declaration>,
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

        // Parse values manually, stopping at !important or end of input
        let mut values = Vec::new();
        while let Ok(value) = parse_single_value(input, self.unsupported_units) {
            values.push(value);
            // Check if there's a comma (indicating more values)
            if input.try_parse(|i| i.expect_comma()).is_err() {
                // No comma, we're done parsing values
                break;
            }
            // Comma found, continue to next value
        }

        // Check for !important flag using cssparser's parse_important
        let important = input.try_parse(parse_important).is_ok();

        // Store based on number of values
        let value = match values.len() {
            0 => return Err(input.new_custom_error(())),
            1 => values.into_iter().next().unwrap(),
            _ => ThemeValue::List(values),
        };

        let declaration = crate::theme::theme::Declaration::new(value, important);
        self.declarations.insert(property, declaration);

        Ok(())
    }
}

/// Implementation of AtRuleParser - supports @media rules
impl<'i, 'a> AtRuleParser<'i> for ChartStyleParser<'a> {
    type Prelude = MediaQueryPrelude;
    type AtRule = RuleOrRules;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, ParseError<'i, ()>> {
        // Only handle @media rules
        if name.as_ref() == "media" {
            // Capture the starting position
            let start = input.position();

            // Consume all tokens in the prelude to get the full condition string
            while input.next().is_ok() {}

            // Get the condition string from the input
            let condition_str = input.slice_from(start).trim();

            // Parse the condition
            let condition = parse_media_condition(condition_str).map_err(|e| {
                warn!(
                    condition = condition_str,
                    error = %e,
                    "Failed to parse media condition"
                );
                input.new_custom_error(())
            })?;

            Ok(MediaQueryPrelude { condition })
        } else {
            // Reject other at-rules
            Err(input.new_custom_error(()))
        }
    }

    fn parse_block<'t>(
        &mut self,
        prelude: Self::Prelude,
        _start: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::AtRule, ParseError<'i, ()>> {
        use crate::theme::media_query::MediaCondition;

        // Parse the rules inside the @media block
        let mut nested_rules = Vec::new();

        // Use StyleSheetParser to parse nested qualified rules
        let nested_parser = StyleSheetParser::new(input, self);

        for result in nested_parser {
            match result {
                Ok(rule_or_rules) => {
                    // Handle both single rules and collections
                    match rule_or_rules {
                        RuleOrRules::Rule(mut rule) => {
                            // Add the media condition to the rule
                            rule.media_condition = Some(prelude.condition.clone());
                            nested_rules.push(rule);
                        }
                        RuleOrRules::Rules(mut rules) => {
                            // Nested @media or other at-rules (add media condition to all)
                            for rule in &mut rules {
                                // Combine conditions with AND if rule already has one
                                if let Some(existing) = &rule.media_condition {
                                    rule.media_condition = Some(MediaCondition::And(vec![
                                        prelude.condition.clone(),
                                        existing.clone(),
                                    ]));
                                } else {
                                    rule.media_condition = Some(prelude.condition.clone());
                                }
                            }
                            nested_rules.extend(rules);
                        }
                    }
                }
                Err((err, context)) => {
                    warn!(
                        error = %format_parse_error("@media", &err, context),
                        "Failed to parse nested CSS rule"
                    );
                    return Err(err);
                }
            }
        }

        Ok(RuleOrRules::Rules(nested_rules))
    }

    fn rule_without_block(
        &mut self,
        _prelude: Self::Prelude,
        _start: &ParserState,
    ) -> Result<Self::AtRule, ()> {
        // @media rules must have a block
        Err(())
    }
}

/// Media query prelude containing the parsed condition
#[derive(Clone)]
struct MediaQueryPrelude {
    condition: crate::theme::media_query::MediaCondition,
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
                "min" | "max" | "clamp" | "abs" | "sign" | "round" | "mod" | "rem" | "hypot"
                | "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "atan2" | "pow" | "sqrt"
                | "exp" | "log" => {
                    return parser
                        .parse_nested_block(|p| {
                            parse_calc_math_function_args(p, &name_str, unsupported_units)
                        })
                        .map(|node| ThemeValue::Calc(Box::new(node)));
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
                    // Store as function - will be resolved at runtime with params
                    Ok(ThemeValue::Function(name_str, args))
                }
                "contrast-color" => {
                    // Store as function - will be resolved at runtime with params
                    Ok(ThemeValue::Function(name_str, args))
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
        Token::CurlyBracketBlock => parser
            .parse_nested_block(|p| parse_object_block(p, unsupported_units))
            .map_err(|_| parser.new_custom_error(())),
        Token::SquareBracketBlock => parser
            .parse_nested_block(|p| parse_array_block(p, unsupported_units))
            .map_err(|_| parser.new_custom_error(())),
        _ => {
            token_to_theme_value(token, unsupported_units).map_err(|_| parser.new_custom_error(()))
        }
    }
}

fn parse_object_block<'i, 't>(
    parser: &mut Parser<'i, 't>,
    unsupported_units: &RefCell<Vec<String>>,
) -> Result<ThemeValue, ParseError<'i, ()>> {
    let mut values = IndexMap::new();

    loop {
        parser.skip_whitespace();
        if parser.is_exhausted() {
            break;
        }

        let name = parser.expect_ident()?.to_string();
        parser.expect_colon()?;
        let value = parse_single_value(parser, unsupported_units)?;

        if values.insert(name, value).is_some() {
            return Err(parser.new_custom_error(()));
        }

        parser.skip_whitespace();
        if parser.is_exhausted() {
            break;
        }

        parser.expect_semicolon()?;
    }

    Ok(ThemeValue::Object(values))
}

fn parse_array_block<'i, 't>(
    parser: &mut Parser<'i, 't>,
    unsupported_units: &RefCell<Vec<String>>,
) -> Result<ThemeValue, ParseError<'i, ()>> {
    let mut values = Vec::new();

    loop {
        parser.skip_whitespace();
        if parser.is_exhausted() {
            break;
        }

        values.push(parse_single_value(parser, unsupported_units)?);

        parser.skip_whitespace();
        if parser.is_exhausted() {
            break;
        }

        parser.expect_comma()?;
    }

    Ok(ThemeValue::Array(values))
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

            // Peek at next token to decide how to parse
            let state = p.state();
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
                        Token::Percentage { unit_value, .. } => {
                            values.push(ThemeValue::Percentage(*unit_value as f64 * 100.0));
                        }
                        Token::Function(_name) => {
                            // Nested function - reset and parse as complete value
                            p.reset(&state);
                            let nested_value = parse_single_value(p, unsupported_units)?;
                            values.push(nested_value);
                        }
                        _ => {
                            if let Ok(theme_value) = token_to_theme_value(token, unsupported_units)
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
                if let Ok(token) = parser.next_including_whitespace()
                    && !matches!(token, Token::WhiteSpace(_))
                {
                    parser.reset(&next_state);
                }

                let term = parse_calc_product(parser, unsupported_units)?;
                terms.push(term);
            }
            Ok(Token::Delim('-')) => {
                // Require whitespace before the operator (already consumed by skip_whitespace)
                // Check for whitespace after
                let next_state = parser.state();
                if let Ok(token) = parser.next_including_whitespace()
                    && !matches!(token, Token::WhiteSpace(_))
                {
                    parser.reset(&next_state);
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
        Token::Number { value, .. } => Ok(CalcNode::Leaf(CalcLeaf::Number(*value as f64))),
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
        Token::Percentage { unit_value, .. } => Ok(CalcNode::Leaf(CalcLeaf::Percentage(
            (*unit_value * 100.0) as f64,
        ))),
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
                    if let Some(keyword) = ColorChannel::from_ident(ident.as_ref()) {
                        Ok(CalcNode::Leaf(CalcLeaf::ColorChannel(keyword)))
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
                                    _ => Err(p.new_custom_error(())),
                                }
                            })
                        }
                        // Math functions
                        _ => parser.parse_nested_block(|p| {
                            parse_calc_math_function_args(p, &name_str, unsupported_units)
                        }),
                    }
                }
                _ => Err(parser.new_custom_error(())),
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

    fn parse_part(&self) -> bool {
        true
    }

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
    color_space: Option<ColorSpace>,
) -> Result<ColorComponent, ParseError<'i, ()>> {
    // Try to parse a value
    let value = parse_single_value(parser, unsupported_units)?;

    // Convert ThemeValue to ColorComponent
    match &value {
        // Check for channel keywords (bare identifiers)
        ThemeValue::String(s) => {
            if s.eq_ignore_ascii_case("none") {
                return Ok(ColorComponent::None);
            }
            if let Some(keyword) = ColorChannel::from_ident_with_color_space(s, color_space) {
                return Ok(ColorComponent::ColorChannel(keyword));
            }
            // Fall through to normal conversion
            ColorComponent::from_theme_value(&value).map_err(|_| parser.new_custom_error(()))
        }
        _ => ColorComponent::from_theme_value(&value).map_err(|_| parser.new_custom_error(())),
    }
}

/// Parse oklch() function with optional "from <origin>" syntax
fn parse_oklch_with_origin<'i, 't>(
    parser: &mut Parser<'i, 't>,
    unsupported_units: &RefCell<Vec<String>>,
) -> Result<ThemeValue, ParseError<'i, ()>> {
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
                ColorComponent::ColorChannel(ColorChannel::Alpha)
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
    parser.parse_nested_block(|p| {
        let origin = try_parse_origin_color(p, unsupported_units)?;

        if let Some(origin_color) = origin {
            // Relative color syntax: oklab(from <origin> L A B [/ alpha])
            let lightness =
                parse_color_component_with_space(p, unsupported_units, Some(ColorSpace::Oklab))?;
            let a_component =
                parse_color_component_with_space(p, unsupported_units, Some(ColorSpace::Oklab))?;
            let b_component =
                parse_color_component_with_space(p, unsupported_units, Some(ColorSpace::Oklab))?;

            let alpha = if p.try_parse(|p| p.expect_delim('/')).is_ok() {
                parse_color_component_with_space(p, unsupported_units, Some(ColorSpace::Oklab))?
            } else {
                ColorComponent::ColorChannel(ColorChannel::Alpha)
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
                ColorComponent::ColorChannel(ColorChannel::Alpha)
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
    parser.parse_nested_block(|p| {
        let origin = try_parse_origin_color(p, unsupported_units)?;

        if let Some(origin_color) = origin {
            // Relative color syntax: lab(from <origin> L A B [/ alpha])
            let lightness =
                parse_color_component_with_space(p, unsupported_units, Some(ColorSpace::Lab))?;
            let a_component =
                parse_color_component_with_space(p, unsupported_units, Some(ColorSpace::Lab))?;
            let b_component =
                parse_color_component_with_space(p, unsupported_units, Some(ColorSpace::Lab))?;

            let alpha = if p.try_parse(|p| p.expect_delim('/')).is_ok() {
                parse_color_component_with_space(p, unsupported_units, Some(ColorSpace::Lab))?
            } else {
                ColorComponent::ColorChannel(ColorChannel::Alpha)
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
                ColorComponent::ColorChannel(ColorChannel::Alpha)
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
            // Absolute color syntax: hsl(h s l [/ alpha])
            // Parse space-separated values (modern syntax) or comma-separated (legacy)
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
                    Ok(Token::Comma) => {
                        // Legacy comma-separated syntax - continue parsing
                        continue;
                    }
                    _ => p.reset(&state),
                }
            }

            // Try to parse as absolute hsl() color
            if let Some(color) = css_value::parse_hsl_function(&values) {
                Ok(ThemeValue::Color(color))
            } else {
                Ok(ThemeValue::Function("hsl".to_string(), values))
            }
        }
    })
}

/// Parse hwb() function with optional "from <origin>" syntax
fn parse_hwb_with_origin<'i, 't>(
    parser: &mut Parser<'i, 't>,
    unsupported_units: &RefCell<Vec<String>>,
) -> Result<ThemeValue, ParseError<'i, ()>> {
    parser.parse_nested_block(|p| {
        let origin = try_parse_origin_color(p, unsupported_units)?;

        if let Some(origin_color) = origin {
            // Relative color syntax: hwb(from <origin> H W B [/ alpha])
            let hue =
                parse_color_component_with_space(p, unsupported_units, Some(ColorSpace::Hwb))?;
            let whiteness =
                parse_color_component_with_space(p, unsupported_units, Some(ColorSpace::Hwb))?;
            let blackness =
                parse_color_component_with_space(p, unsupported_units, Some(ColorSpace::Hwb))?;

            let alpha = if p.try_parse(|p| p.expect_delim('/')).is_ok() {
                parse_color_component_with_space(p, unsupported_units, Some(ColorSpace::Hwb))?
            } else {
                ColorComponent::ColorChannel(ColorChannel::Alpha)
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
            // Absolute color syntax: hwb(hue whiteness blackness [/ alpha])
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

            // Try to parse as absolute hwb() color
            if let Some(color) = css_value::parse_hwb_function(&values) {
                Ok(ThemeValue::Color(color))
            } else {
                Ok(ThemeValue::Function("hwb".to_string(), values))
            }
        }
    })
}

/// Parse rgb() function with optional "from <origin>" syntax
fn parse_rgb_with_origin<'i, 't>(
    parser: &mut Parser<'i, 't>,
    unsupported_units: &RefCell<Vec<String>>,
) -> Result<ThemeValue, ParseError<'i, ()>> {
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
                ColorComponent::ColorChannel(ColorChannel::Alpha)
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
            // Absolute color syntax: rgb(r g b [/ alpha])
            // Parse space-separated values (modern syntax) or comma-separated (legacy)
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
                    Ok(Token::Comma) => {
                        // Legacy comma-separated syntax - continue parsing
                        continue;
                    }
                    _ => p.reset(&state),
                }
            }

            // Try to parse as absolute rgb() color
            if let Some(color) = css_value::parse_rgb_function(&values) {
                Ok(ThemeValue::Color(color))
            } else {
                Ok(ThemeValue::Function("rgb".to_string(), values))
            }
        }
    })
}

/// Parse a media query condition string using token-based parsing
///
/// Supports:
/// - Modern range syntax: (width >= 600px), (width > 400px and width < 1200px)
/// - Legacy syntax: (min-width: 600px), (max-width: 1200px)
/// - Logical operators with proper precedence: not > and > or
/// - Parenthesized groups: ((a or b) and c)
///
/// Returns a MediaCondition that can be evaluated against params.
fn parse_media_condition(
    condition_str: &str,
) -> Result<crate::theme::media_query::MediaCondition, String> {
    use cssparser::{Parser, ParserInput};

    let mut input = ParserInput::new(condition_str);
    let mut parser = Parser::new(&mut input);

    parse_media_condition_tokens(&mut parser)
        .map_err(|e| format!("Failed to parse media condition: {:?}", e))
}

/// Parse media condition using cssparser tokens with proper operator precedence
///
/// Precedence (highest to lowest):
/// 1. not
/// 2. and
/// 3. or
fn parse_media_condition_tokens<'i, 't>(
    parser: &mut Parser<'i, 't>,
) -> Result<crate::theme::media_query::MediaCondition, ParseError<'i, ()>> {
    parser.skip_whitespace();

    // Parse "or" level (lowest precedence)
    parse_media_or(parser)
}

/// Parse "or" expressions (lowest precedence)
fn parse_media_or<'i, 't>(
    parser: &mut Parser<'i, 't>,
) -> Result<crate::theme::media_query::MediaCondition, ParseError<'i, ()>> {
    use crate::theme::media_query::MediaCondition;

    let mut left = parse_media_and(parser)?;

    loop {
        parser.skip_whitespace();

        // Try to parse "or"
        if parser.try_parse(|p| p.expect_ident_matching("or")).is_err() {
            break;
        }

        parser.skip_whitespace();
        let right = parse_media_and(parser)?;

        // Combine with existing or chain
        left = match left {
            MediaCondition::Or(mut items) => {
                items.push(right);
                MediaCondition::Or(items)
            }
            _ => MediaCondition::Or(vec![left, right]),
        };
    }

    Ok(left)
}

/// Parse "and" expressions (medium precedence)
fn parse_media_and<'i, 't>(
    parser: &mut Parser<'i, 't>,
) -> Result<crate::theme::media_query::MediaCondition, ParseError<'i, ()>> {
    use crate::theme::media_query::MediaCondition;

    let mut left = parse_media_not(parser)?;

    loop {
        parser.skip_whitespace();

        // Try to parse "and"
        if parser
            .try_parse(|p| p.expect_ident_matching("and"))
            .is_err()
        {
            break;
        }

        parser.skip_whitespace();
        let right = parse_media_not(parser)?;

        // Combine with existing and chain
        left = match left {
            MediaCondition::And(mut items) => {
                items.push(right);
                MediaCondition::And(items)
            }
            _ => MediaCondition::And(vec![left, right]),
        };
    }

    Ok(left)
}

/// Parse "not" expressions (highest precedence)
fn parse_media_not<'i, 't>(
    parser: &mut Parser<'i, 't>,
) -> Result<crate::theme::media_query::MediaCondition, ParseError<'i, ()>> {
    use crate::theme::media_query::MediaCondition;

    parser.skip_whitespace();

    // Check for "not"
    if parser.try_parse(|p| p.expect_ident_matching("not")).is_ok() {
        parser.skip_whitespace();
        let inner = parse_media_primary(parser)?;
        return Ok(MediaCondition::Not(Box::new(inner)));
    }

    parse_media_primary(parser)
}

/// Parse primary expression (feature or parenthesized group)
fn parse_media_primary<'i, 't>(
    parser: &mut Parser<'i, 't>,
) -> Result<crate::theme::media_query::MediaCondition, ParseError<'i, ()>> {
    parser.skip_whitespace();

    // Check for parenthesized group
    if parser.try_parse(|p| p.expect_parenthesis_block()).is_ok() {
        return parser.parse_nested_block(parse_media_condition_tokens);
    }

    // Parse feature expression
    parse_media_feature_tokens(parser)
}

/// Parse a media feature expression using tokens
///
/// Supports:
/// - Modern range syntax: width >= 600px
/// - Multi-range syntax: 600px <= width < 1200px
/// - Legacy syntax: min-width: 600px
fn parse_media_feature_tokens<'i, 't>(
    parser: &mut Parser<'i, 't>,
) -> Result<crate::theme::media_query::MediaCondition, ParseError<'i, ()>> {
    use crate::theme::media_query::{MediaCondition, MediaFeature, MediaOperator};
    use cssparser::Token;

    parser.skip_whitespace();

    // Try to parse as multi-range first: VALUE OP NAME OP VALUE
    // Example: 600px <= width < 1200px
    let location = parser.current_source_location();

    // Try to parse left value + operator (for multi-range)
    if let Ok((left_value, left_op)) = parser.try_parse::<_, _, ParseError<()>>(|p| {
        let val = parse_dimension_value_tokens(p)?;
        p.skip_whitespace();
        let op = parse_operator_tokens(p)?;
        p.skip_whitespace();
        Ok((val, op))
    }) {
        // We have left_value and left_op, now parse the feature name
        let name_token = parser.next()?;
        let name_str = match name_token {
            Token::Ident(name) => name.as_ref().to_string(),
            _ => return Err(location.new_unexpected_token_error(name_token.clone())),
        };

        parser.skip_whitespace();

        // Parse right operator and value
        let right_op = parse_operator_tokens(parser)?;
        parser.skip_whitespace();
        let right_value = parse_dimension_value_tokens(parser)?;

        // Validate operator compatibility
        if !left_op.is_compatible_with(right_op) {
            return Err(location.new_custom_error(()));
        }

        return Ok(MediaCondition::Feature(MediaFeature::Range {
            name: name_str,
            left_value,
            left_op,
            right_op,
            right_value,
        }));
    }

    // Not a multi-range, try single comparison or legacy syntax
    let token = parser.next()?;

    match token {
        Token::Ident(name) => {
            // Clone the name to owned string so we can use parser again
            let name_str = name.as_ref().to_string();
            parser.skip_whitespace();

            // Check for colon (legacy syntax: min-width: 600px)
            if parser.try_parse(|p| p.expect_colon()).is_ok() {
                parser.skip_whitespace();
                let value = parse_dimension_value_tokens(parser)?;

                // Convert legacy min-/max- prefix to modern operators
                if let Some(base_name) = name_str.strip_prefix("min-") {
                    return Ok(MediaCondition::Feature(MediaFeature::Single {
                        name: base_name.to_string(),
                        op: MediaOperator::GreaterEqual,
                        value,
                    }));
                }

                if let Some(base_name) = name_str.strip_prefix("max-") {
                    return Ok(MediaCondition::Feature(MediaFeature::Single {
                        name: base_name.to_string(),
                        op: MediaOperator::LessEqual,
                        value,
                    }));
                }

                // Plain feature (e.g., width: 800px) - treat as equality
                return Ok(MediaCondition::Feature(MediaFeature::Single {
                    name: name_str.clone(),
                    op: MediaOperator::Equal,
                    value,
                }));
            }

            // Modern range syntax: width >= 600px
            // Parse operator
            let op = parse_operator_tokens(parser)?;
            parser.skip_whitespace();

            // Parse value
            let value = parse_dimension_value_tokens(parser)?;

            Ok(MediaCondition::Feature(MediaFeature::Single {
                name: name_str,
                op,
                value,
            }))
        }
        _ => Err(location.new_unexpected_token_error(token.clone())),
    }
}

/// Parse an operator from tokens (>=, <=, >, <, =)
fn parse_operator_tokens<'i, 't>(
    parser: &mut Parser<'i, 't>,
) -> Result<crate::theme::media_query::MediaOperator, ParseError<'i, ()>> {
    use crate::theme::media_query::MediaOperator;
    use cssparser::Token;

    let location = parser.current_source_location();
    let token = parser.next()?;

    match token {
        Token::Delim('=') => Ok(MediaOperator::Equal),
        Token::Delim('>') => {
            // Check for >= (no whitespace allowed between > and =)
            if parser
                .try_parse(|p| match p.next_including_whitespace() {
                    Ok(Token::Delim('=')) => Ok(()),
                    _ => Err(()),
                })
                .is_ok()
            {
                Ok(MediaOperator::GreaterEqual)
            } else {
                Ok(MediaOperator::GreaterThan)
            }
        }
        Token::Delim('<') => {
            // Check for <= (no whitespace allowed between < and =)
            if parser
                .try_parse(|p| match p.next_including_whitespace() {
                    Ok(Token::Delim('=')) => Ok(()),
                    _ => Err(()),
                })
                .is_ok()
            {
                Ok(MediaOperator::LessEqual)
            } else {
                Ok(MediaOperator::LessThan)
            }
        }
        _ => Err(location.new_unexpected_token_error(token.clone())),
    }
}

/// Parse a dimension value from tokens (e.g., 600px, 30rem, 800)
fn parse_dimension_value_tokens<'i, 't>(
    parser: &mut Parser<'i, 't>,
) -> Result<crate::theme::media_query::DimensionValue, ParseError<'i, ()>> {
    use crate::theme::media_query::DimensionValue;
    use cssparser::Token;

    let location = parser.current_source_location();
    let token = parser.next()?;

    match token {
        Token::Number { value, .. } => {
            let num_value = *value;
            // Check if there's a unit following
            let unit_result: Result<String, _> = parser.try_parse(|p| match p.next() {
                Ok(Token::Ident(unit)) => Ok(unit.as_ref().to_string()),
                _ => Err(()),
            });

            if let Ok(unit) = unit_result {
                match unit.as_str() {
                    "px" => Ok(DimensionValue::Pixels(num_value)),
                    "rem" => Ok(DimensionValue::Rem(num_value)),
                    _ => Err(location.new_custom_error(())),
                }
            } else {
                // Plain number treated as pixels
                Ok(DimensionValue::Pixels(num_value))
            }
        }
        Token::Dimension { value, unit, .. } => match unit.as_ref() {
            "px" => Ok(DimensionValue::Pixels(*value)),
            "rem" => Ok(DimensionValue::Rem(*value)),
            _ => Err(location.new_custom_error(())),
        },
        _ => Err(location.new_unexpected_token_error(token.clone())),
    }
}

// Old string-based parser functions removed - now using token-based parser

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::media_query::DimensionValue;

    fn declaration_value<'a>(
        rule: &'a crate::theme::theme::CompiledRule,
        property: &str,
    ) -> &'a ThemeValue {
        &rule
            .declarations
            .get(property)
            .unwrap_or_else(|| panic!("missing declaration {property}"))
            .value
    }

    #[test]
    fn test_parse_curly_object_value_inside_declaration() {
        let css = r#"
            mark {
                fill-pattern-discrete: {
                    anchor: plot;
                    ink: { type: auto-contrast; opacity: 13%; };
                    layers: [
                        { type: stripe; angle: 45deg; spacing: 16px; stroke-width: 1px; }
                    ];
                };
            }
        "#;

        let rules = parse_stylesheet(css).expect("Failed to parse CSS object value");
        assert_eq!(rules.len(), 1);

        let ThemeValue::Object(pattern) = declaration_value(&rules[0], "fill-pattern-discrete")
        else {
            panic!("expected pattern object");
        };
        assert!(
            matches!(pattern.get("anchor"), Some(ThemeValue::String(value)) if value == "plot")
        );
        assert!(matches!(pattern.get("ink"), Some(ThemeValue::Object(_))));
        assert!(matches!(pattern.get("layers"), Some(ThemeValue::Array(_))));
    }

    #[test]
    fn test_parse_square_array_containing_object_values() {
        let css = r#"
            mark {
                pattern-options: [
                    { type: stripe; angle: 0deg; spacing: 12px; stroke-width: 1px; },
                    { type: stripe; angle: 90deg; spacing: 16px; stroke-width: 2px; }
                ];
            }
        "#;

        let rules = parse_stylesheet(css).expect("Failed to parse CSS array value");
        assert_eq!(rules.len(), 1);

        let ThemeValue::Array(values) = declaration_value(&rules[0], "pattern-options") else {
            panic!("expected array value");
        };
        assert_eq!(values.len(), 2);
        assert!(
            values
                .iter()
                .all(|value| matches!(value, ThemeValue::Object(_)))
        );
    }

    #[test]
    fn test_semicolons_inside_object_block_do_not_end_outer_declaration() {
        let css = r#"
            mark {
                fill-pattern-discrete: {
                    ink: { type: solid; color: black; opacity: 0.2; };
                    layers: [
                        { type: stripe; angle: 45deg; spacing: 12px; stroke-width: 1px; }
                    ];
                };
                fill: red;
            }
        "#;

        let rules = parse_stylesheet(css).expect("Failed to parse CSS object value");
        assert_eq!(rules.len(), 1);
        assert!(matches!(
            declaration_value(&rules[0], "fill-pattern-discrete"),
            ThemeValue::Object(_)
        ));
        assert!(matches!(
            declaration_value(&rules[0], "fill"),
            ThemeValue::Color(CssRgba {
                red: 255,
                green: 0,
                blue: 0,
                alpha: 255
            })
        ));
    }

    #[test]
    fn test_existing_fill_discrete_list_parses_as_before() {
        let css = r#"
            mark {
                fill-discrete: #ff0000, #00ff00, #0000ff;
            }
        "#;

        let rules = parse_stylesheet(css).expect("Failed to parse CSS list");
        assert_eq!(rules.len(), 1);

        let ThemeValue::List(values) = declaration_value(&rules[0], "fill-discrete") else {
            panic!("expected fill-discrete list");
        };
        assert_eq!(values.len(), 3);
        assert!(matches!(
            values.as_slice(),
            [
                ThemeValue::Color(CssRgba {
                    red: 255,
                    green: 0,
                    blue: 0,
                    alpha: 255
                }),
                ThemeValue::Color(CssRgba {
                    red: 0,
                    green: 255,
                    blue: 0,
                    alpha: 255
                }),
                ThemeValue::Color(CssRgba {
                    red: 0,
                    green: 0,
                    blue: 255,
                    alpha: 255
                }),
            ]
        ));
    }

    #[test]
    fn test_parse_error_reports_stylesheet_context() {
        let css = r#"
            mark {
                fill-pattern-discrete: {
                    anchor: plot
                    layers: [
                        { type: stripe; angle: 45deg; spacing: 16px; stroke-width: 1px; }
                    ];
                };
            }
        "#;

        let err = parse_stylesheet(css).expect_err("invalid CSS should report an error");

        assert!(
            err.contains("CSS parse error at stylesheet"),
            "unexpected error: {err}"
        );
        assert!(
            err.contains("line") && err.contains("column"),
            "unexpected error: {err}"
        );
        assert!(
            err.contains("fill-pattern-discrete"),
            "unexpected error: {err}"
        );
        assert!(err.contains("anchor"), "unexpected error: {err}");
    }

    #[test]
    fn test_parse_media_query_modern_syntax() {
        let css = r#"
            @media (width >= 600px) {
                mark { fill: red; }
            }
        "#;

        let rules = parse_stylesheet(css).expect("Failed to parse CSS");
        assert_eq!(rules.len(), 1);

        let rule = &rules[0];
        assert!(rule.media_condition.is_some());

        // Check that the condition is correct
        if let Some(crate::theme::media_query::MediaCondition::Feature(
            crate::theme::media_query::MediaFeature::Single { name, op, value },
        )) = &rule.media_condition
        {
            assert_eq!(name, "width");
            assert_eq!(*op, crate::theme::media_query::MediaOperator::GreaterEqual);
            assert_eq!(*value, DimensionValue::Pixels(600.0));
        } else {
            panic!("Expected feature condition");
        }

        // Check that the declaration was parsed
        assert!(rule.declarations.contains_key("fill"));
    }

    #[test]
    fn test_parse_media_query_legacy_syntax() {
        let css = r#"
            @media (min-width: 600px) {
                mark { fill: blue; }
            }
        "#;

        let rules = parse_stylesheet(css).expect("Failed to parse CSS");
        assert_eq!(rules.len(), 1);

        let rule = &rules[0];
        assert!(rule.media_condition.is_some());

        // Check that legacy min-width was converted to width >=
        if let Some(crate::theme::media_query::MediaCondition::Feature(
            crate::theme::media_query::MediaFeature::Single { name, op, value },
        )) = &rule.media_condition
        {
            assert_eq!(name, "width");
            assert_eq!(*op, crate::theme::media_query::MediaOperator::GreaterEqual);
            assert_eq!(*value, DimensionValue::Pixels(600.0));
        } else {
            panic!("Expected feature condition");
        }
    }

    #[test]
    fn test_parse_media_query_and_condition() {
        let css = r#"
            @media (width >= 600px) and (height >= 400px) {
                mark { stroke-width: 2px; }
            }
        "#;

        let rules = parse_stylesheet(css).expect("Failed to parse CSS");
        assert_eq!(rules.len(), 1);

        let rule = &rules[0];
        assert!(rule.media_condition.is_some());

        // Check that it's an AND condition
        if let Some(crate::theme::media_query::MediaCondition::And(conditions)) =
            &rule.media_condition
        {
            assert_eq!(conditions.len(), 2);
        } else {
            panic!("Expected AND condition");
        }
    }

    #[test]
    fn test_parse_multiple_rules_in_media_query() {
        let css = r#"
            @media (width >= 600px) {
                mark { fill: red; }
                guide { background-color: blue; }
            }
        "#;

        let rules = parse_stylesheet(css).expect("Failed to parse CSS");
        assert_eq!(rules.len(), 2);

        // Both rules should have the same media condition
        assert!(rules[0].media_condition.is_some());
        assert!(rules[1].media_condition.is_some());
    }

    #[test]
    fn test_parse_mixed_regular_and_media_rules() {
        let css = r#"
            mark { fill: green; }

            @media (width >= 600px) {
                mark { fill: red; }
            }

            guide { background-color: white; }
        "#;

        let rules = parse_stylesheet(css).expect("Failed to parse CSS");
        assert_eq!(rules.len(), 3);

        // First and third rules should NOT have media conditions
        assert!(rules[0].media_condition.is_none());
        assert!(rules[2].media_condition.is_none());

        // Second rule should have a media condition
        assert!(rules[1].media_condition.is_some());
    }

    #[test]
    fn test_media_query_end_to_end() {
        use crate::theme::{Theme, ThemeContext};
        use datafusion_common::ScalarValue;
        use indexmap::IndexMap;

        let css = r#"
            mark { fill: blue; }

            @media (width >= 600px) {
                mark { fill: red; }
            }
        "#;

        let theme = Theme::from_css(css).expect("Failed to parse CSS");

        // Without width param - should get blue
        let ctx = ThemeContext::new("mark", IndexMap::new());
        let fill = theme.fill_color(&ctx).expect("Should have fill");
        assert_eq!(fill[2], 1.0, "Should be blue without width param");
        assert_eq!(fill[0], 0.0, "Should not be red");

        // With width < 600 - should get blue
        let mut params_small = IndexMap::new();
        params_small.insert("width".to_string(), ScalarValue::Float32(Some(400.0)));
        let ctx_small = ThemeContext::new("mark", params_small);
        let fill_small = theme.fill_color(&ctx_small).expect("Should have fill");
        assert_eq!(fill_small[2], 1.0, "Should be blue with width < 600");

        // With width >= 600 - should get red
        let mut params_large = IndexMap::new();
        params_large.insert("width".to_string(), ScalarValue::Float32(Some(800.0)));
        let ctx_large = ThemeContext::new("mark", params_large);
        let fill_large = theme.fill_color(&ctx_large).expect("Should have fill");
        assert_eq!(fill_large[0], 1.0, "Should be red with width >= 600");
        assert_eq!(fill_large[2], 0.0, "Should not be blue");
    }

    #[test]
    fn test_parse_media_query_with_parenthesized_or() {
        let css = r#"
            @media (width >= 600px) and ((height >= 400px) or (height <= 200px)) {
                guide { background-color: blue; }
            }
        "#;

        let rules = parse_stylesheet(css).expect("Failed to parse CSS with parenthesized or");
        assert_eq!(rules.len(), 1);
        assert!(rules[0].media_condition.is_some());
    }

    #[test]
    fn test_parse_media_query_complex_nested() {
        let css = r#"
            @media ((width >= 600px) and (height >= 400px)) or (width >= 1200px) {
                guide { background-color: green; }
            }
        "#;

        let rules = parse_stylesheet(css).expect("Failed to parse complex nested media query");
        assert_eq!(rules.len(), 1);
        assert!(rules[0].media_condition.is_some());
    }

    #[test]
    fn test_parse_media_query_operator_precedence() {
        use crate::theme::{Theme, ThemeContext};
        use datafusion_common::ScalarValue;
        use indexmap::IndexMap;

        // Test: a or b and c should parse as a or (b and c)
        // (width < 400px) or (width >= 600px) and (height >= 400px)
        // Should match:
        // - width < 400px (regardless of height)
        // - width >= 600px AND height >= 400px
        let css = r#"
            guide { background-color: white; }

            @media (width < 400px) or (width >= 600px) and (height >= 400px) {
                guide { background-color: blue; }
            }
        "#;

        let theme = Theme::from_css(css).expect("Failed to parse CSS");

        // Case 1: width = 300px, height = 100px → should match (width < 400)
        let mut params1 = IndexMap::new();
        params1.insert("width".to_string(), ScalarValue::Float32(Some(300.0)));
        params1.insert("height".to_string(), ScalarValue::Float32(Some(100.0)));
        let ctx1 = ThemeContext::new("guide", params1);
        let color1 = theme.query(&ctx1, "background-color");
        assert!(color1.is_some(), "Should match when width < 400");

        // Case 2: width = 500px, height = 500px → should NOT match
        let mut params2 = IndexMap::new();
        params2.insert("width".to_string(), ScalarValue::Float32(Some(500.0)));
        params2.insert("height".to_string(), ScalarValue::Float32(Some(500.0)));
        let ctx2 = ThemeContext::new("guide", params2);
        let _color2 = theme.query(&ctx2, "background-color");
        // This should NOT match because width is not < 400 and not (>= 600 AND >= 400)
        // With correct precedence, should be white (default)

        // Case 3: width = 700px, height = 500px → should match (width >= 600 AND height >= 400)
        let mut params3 = IndexMap::new();
        params3.insert("width".to_string(), ScalarValue::Float32(Some(700.0)));
        params3.insert("height".to_string(), ScalarValue::Float32(Some(500.0)));
        let ctx3 = ThemeContext::new("guide", params3);
        let color3 = theme.query(&ctx3, "background-color");
        assert!(
            color3.is_some(),
            "Should match when width >= 600 and height >= 400"
        );
    }

    #[test]
    fn test_parse_multi_range_syntax() {
        let css = r#"
            @media (600px <= width < 1200px) {
                guide { background-color: green; }
            }
        "#;

        let rules = parse_stylesheet(css).expect("Failed to parse multi-range syntax");
        assert_eq!(rules.len(), 1);

        let rule = &rules[0];
        assert!(rule.media_condition.is_some());

        // Check that it parsed as a Range variant
        if let Some(crate::theme::media_query::MediaCondition::Feature(
            crate::theme::media_query::MediaFeature::Range {
                name,
                left_value,
                left_op,
                right_op,
                right_value,
            },
        )) = &rule.media_condition
        {
            assert_eq!(name, "width");
            assert_eq!(*left_value, DimensionValue::Pixels(600.0));
            assert_eq!(
                *left_op,
                crate::theme::media_query::MediaOperator::LessEqual
            );
            assert_eq!(
                *right_op,
                crate::theme::media_query::MediaOperator::LessThan
            );
            assert_eq!(*right_value, DimensionValue::Pixels(1200.0));
        } else {
            panic!("Expected Range feature condition");
        }
    }

    #[test]
    fn test_parse_multi_range_with_spaces() {
        let css = r#"
            @media ( 600px  <=  width  <  1200px ) {
                guide { background-color: blue; }
            }
        "#;

        let rules = parse_stylesheet(css).expect("Failed to parse multi-range with spaces");
        assert_eq!(rules.len(), 1);
        assert!(rules[0].media_condition.is_some());
    }

    #[test]
    fn test_multi_range_evaluation() {
        use crate::theme::{Theme, ThemeContext, ThemeValue};
        use datafusion_common::ScalarValue;
        use indexmap::IndexMap;

        let css = r#"
            guide { background-color: white; }

            @media (600px <= width < 1200px) {
                guide { background-color: green; }
            }
        "#;

        let theme = Theme::from_css(css).expect("Failed to parse CSS");

        // Width 800px - should match (in range)
        let mut params_in = IndexMap::new();
        params_in.insert("width".to_string(), ScalarValue::Float32(Some(800.0)));
        let ctx_in = ThemeContext::new("guide", params_in);
        let bg_in = theme.query(&ctx_in, "background-color");
        assert!(bg_in.is_some(), "Should match when in range");

        // Width 400px - should not match (below range)
        let mut params_below = IndexMap::new();
        params_below.insert("width".to_string(), ScalarValue::Float32(Some(400.0)));
        let ctx_below = ThemeContext::new("guide", params_below);
        let bg_below = theme.query(&ctx_below, "background-color");
        // Should get white (default), not green
        if let Some(ThemeValue::Color(color)) = bg_below {
            assert_eq!(color.red, 255, "Should be white (below range)");
            assert_eq!(color.green, 255, "Should be white (below range)");
        }

        // Width 1200px - should not match (at exclusive boundary)
        let mut params_boundary = IndexMap::new();
        params_boundary.insert("width".to_string(), ScalarValue::Float32(Some(1200.0)));
        let ctx_boundary = ThemeContext::new("guide", params_boundary);
        let bg_boundary = theme.query(&ctx_boundary, "background-color");
        // Should get white (default), not green
        if let Some(ThemeValue::Color(color)) = bg_boundary {
            assert_eq!(color.red, 255, "Should be white (at exclusive boundary)");
            assert_eq!(color.green, 255, "Should be white (at exclusive boundary)");
        }
    }
}
