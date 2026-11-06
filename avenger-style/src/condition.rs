//! The conditions of `@media` and `@container` rules, which read only the host inputs.

use cssparser::{ParseError, Parser, Token};

use crate::{
    source::Problem,
    values::{parse_numeric, Calc, Category},
};

/// The condition of a conditional rule. A rule inside several holds when every one holds.
#[derive(Clone, Debug, PartialEq)]
pub enum Condition {
    /// An `@media` rule: its queries, any of which holds.
    Media(Vec<MediaQuery>),
    /// An `@container` rule, which reads the host box.
    Container(Query),
}

/// One query of a media query list.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaQuery {
    pub negated: bool,
    /// The media type, when the query names one.
    pub media_type: Option<MediaType>,
    pub condition: Option<Query>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaType {
    All,
    Screen,
    /// True when the output is for printing.
    Print,
}

/// A condition of features with `not`, `and`, and `or`.
#[derive(Clone, Debug, PartialEq)]
pub enum Query {
    Not(Box<Query>),
    And(Vec<Query>),
    Or(Vec<Query>),
    Feature(Feature),
}

/// A test of one feature.
#[derive(Clone, Debug, PartialEq)]
pub enum Feature {
    /// A preference of the host. A test without a value holds when the preference is not its
    /// default: not `no-preference` and not `none`.
    Preference {
        name: Preference,
        value: Option<&'static str>,
    },
    /// A size feature of the host box, compared with one or two values.
    Size {
        feature: SizeFeature,
        tests: Vec<(Comparison, SizeValue)>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preference {
    ColorScheme,
    Contrast,
    ReducedMotion,
    ForcedColors,
}

impl Preference {
    fn from_name(name: &str) -> Option<Preference> {
        Some(match name {
            "prefers-color-scheme" => Preference::ColorScheme,
            "prefers-contrast" => Preference::Contrast,
            "prefers-reduced-motion" => Preference::ReducedMotion,
            "forced-colors" => Preference::ForcedColors,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Preference::ColorScheme => "prefers-color-scheme",
            Preference::Contrast => "prefers-contrast",
            Preference::ReducedMotion => "prefers-reduced-motion",
            Preference::ForcedColors => "forced-colors",
        }
    }

    /// The values of the preference, with its default first.
    pub fn values(self) -> &'static [&'static str] {
        match self {
            Preference::ColorScheme => &["light", "dark"],
            Preference::Contrast => &["no-preference", "more", "less", "custom"],
            Preference::ReducedMotion => &["no-preference", "reduce"],
            Preference::ForcedColors => &["none", "active"],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeFeature {
    Width,
    Height,
    AspectRatio,
    Orientation,
}

impl SizeFeature {
    fn from_name(name: &str) -> Option<SizeFeature> {
        Some(match name {
            "width" => SizeFeature::Width,
            "height" => SizeFeature::Height,
            "aspect-ratio" => SizeFeature::AspectRatio,
            "orientation" => SizeFeature::Orientation,
            _ => return None,
        })
    }
}

/// How a feature compares with a value: `feature <op> value`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comparison {
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    Equal,
}

impl Comparison {
    /// The comparison with its operands swapped: `value <op> feature` is `feature <flipped> value`.
    fn flipped(self) -> Comparison {
        match self {
            Comparison::Less => Comparison::Greater,
            Comparison::LessOrEqual => Comparison::GreaterOrEqual,
            Comparison::Greater => Comparison::Less,
            Comparison::GreaterOrEqual => Comparison::LessOrEqual,
            Comparison::Equal => Comparison::Equal,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SizeValue {
    /// A length in pixels or in `rem` or `em`, whose units resolve against the text scale.
    Length(Calc),
    Ratio(f32),
    Orientation(&'static str),
}

/// Which rule a condition belongs to, which decides its features.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Rule {
    Media,
    Container,
}

/// Parses the prelude of an `@media` rule.
pub(crate) fn parse_media(input: &mut Parser<'_>) -> Result<Vec<MediaQuery>, Problem> {
    let mut queries = Vec::new();
    loop {
        queries.push(media_query(input)?);
        match input.next() {
            Ok(Token::Comma) => continue,
            Ok(_) => return Err(Problem::new("invalid media query")),
            Err(_) => return Ok(queries),
        }
    }
}

/// Parses the prelude of an `@container` rule, which has no container name.
pub(crate) fn parse_container(input: &mut Parser<'_>) -> Result<Query, Problem> {
    let state = input.state();
    if let Ok(Token::Ident(name)) = input.next() {
        let name = name.to_string();
        if !matches!(name.to_ascii_lowercase().as_str(), "not") {
            return Err(Problem::new(format!(
                "the host box is the only container; remove the container name `{name}`"
            )));
        }
    }
    input.reset(&state);
    let query = condition(input, Rule::Container)?;
    match input.is_exhausted() {
        true => Ok(query),
        false => Err(Problem::new("invalid container condition")),
    }
}

fn media_query(input: &mut Parser<'_>) -> Result<MediaQuery, Problem> {
    let state = input.state();
    let mut negated = false;
    if let Ok(Token::Ident(word)) = input.next() {
        let word = word.to_ascii_lowercase();
        let media_type = match word.as_str() {
            "not" | "only" => {
                // `not` before a parenthesized condition negates the condition.
                let after = input.state();
                match input.next() {
                    Ok(Token::Ident(name)) => {
                        negated = word == "not";
                        Some(media_type(&name.to_ascii_lowercase())?)
                    }
                    _ => {
                        input.reset(&after);
                        input.reset(&state);
                        None
                    }
                }
            }
            name => Some(media_type(name)?),
        };
        if let Some(media_type) = media_type {
            let condition = match input.try_parse(|input| input.expect_ident_matching("and")) {
                Ok(()) => Some(condition(input, Rule::Media)?),
                Err(_) => None,
            };
            return Ok(MediaQuery {
                negated,
                media_type: Some(media_type),
                condition,
            });
        }
    } else {
        input.reset(&state);
    }
    Ok(MediaQuery {
        negated: false,
        media_type: None,
        condition: Some(condition(input, Rule::Media)?),
    })
}

fn media_type(name: &str) -> Result<MediaType, Problem> {
    Ok(match name {
        "all" => MediaType::All,
        "screen" => MediaType::Screen,
        "print" => MediaType::Print,
        _ => {
            return Err(Problem::new(format!(
                "the media type `{name}` is not supported"
            )))
        }
    })
}

fn condition(input: &mut Parser<'_>, rule: Rule) -> Result<Query, Problem> {
    if input
        .try_parse(|input| input.expect_ident_matching("not"))
        .is_ok()
    {
        return Ok(Query::Not(Box::new(in_parens(input, rule)?)));
    }
    let first = in_parens(input, rule)?;
    let mut operator: Option<String> = None;
    let mut operands = vec![first];
    loop {
        let state = input.state();
        match input.next() {
            Ok(Token::Ident(word)) => {
                let word = word.to_ascii_lowercase();
                if word != "and" && word != "or" {
                    return Err(Problem::new(format!("unexpected `{word}` in a condition")));
                }
                if operator.as_ref().is_some_and(|operator| *operator != word) {
                    return Err(Problem::new(
                        "a condition does not mix `and` and `or` without parentheses",
                    ));
                }
                operator = Some(word);
                operands.push(in_parens(input, rule)?);
            }
            _ => {
                input.reset(&state);
                break;
            }
        }
    }
    Ok(match (operator.as_deref(), operands.len()) {
        (_, 1) => operands.pop().expect("one operand"),
        (Some("or"), _) => Query::Or(operands),
        _ => Query::And(operands),
    })
}

fn in_parens(input: &mut Parser<'_>, rule: Rule) -> Result<Query, Problem> {
    match input.next() {
        Ok(Token::ParenthesisBlock) => {}
        Ok(Token::Function(name)) if name.eq_ignore_ascii_case("env") => {
            return Err(env_problem());
        }
        _ => return Err(Problem::new("a condition tests a feature in parentheses")),
    }
    let mut problem = None;
    let result = input.parse_nested_block(|input| {
        let state = input.state();
        if let Ok(query) = input.try_parse(|input| -> Result<Query, cssparser::ParseError<()>> {
            let query = condition(input, rule).map_err(|_| ParseError::custom(()))?;
            input.expect_exhausted()?;
            Ok(query)
        }) {
            return Ok(query);
        }
        input.reset(&state);
        match feature(input, rule) {
            Ok(feature) => Ok(Query::Feature(feature)),
            Err(found) => {
                problem = Some(found);
                Err(ParseError::<()>::custom(()))
            }
        }
    });
    result.map_err(|_| problem.unwrap_or_else(|| Problem::new("invalid condition")))
}

fn env_problem() -> Problem {
    Problem::new(
        "env() is not valid in a condition; conditions read only the host box and the preferences",
    )
}

/// Whether the tokens of `input` hold an `env()` function, at any depth.
fn holds_env(input: &mut Parser<'_>) -> bool {
    let mut found = false;
    while let Ok(token) = input.next() {
        match token.clone() {
            Token::Function(name) if name.eq_ignore_ascii_case("env") => found = true,
            Token::Function(_) | Token::ParenthesisBlock | Token::SquareBracketBlock => {
                found |= input
                    .parse_nested_block(|input| {
                        Ok::<_, cssparser::ParseError<()>>(holds_env(input))
                    })
                    .unwrap_or(false);
            }
            _ => {}
        }
    }
    found
}

fn feature(input: &mut Parser<'_>, rule: Rule) -> Result<Feature, Problem> {
    let state = input.state();
    if holds_env(input) {
        return Err(env_problem());
    }
    input.reset(&state);
    // `name`, `name: value`, or a range with the name first or between two values.
    if let Ok(name) =
        input.try_parse(|input| input.expect_ident().map(|name| name.to_ascii_lowercase()))
    {
        if input.is_exhausted() {
            return boolean(&name, rule);
        }
        if input.try_parse(|input| input.expect_colon()).is_ok() {
            return plain(&name, input, rule);
        }
        let comparison = comparison(input)?;
        let feature = size_feature(&name, rule)?;
        let value = size_value(feature, input)?;
        exhausted(input)?;
        return Ok(Feature::Size {
            feature,
            tests: vec![(comparison, value)],
        });
    }
    // `value <op> name [<op> value]`.
    let (first, after) = value_then_comparison(input)?;
    let name = input
        .expect_ident()
        .map_err(|_| Problem::new("a range names its feature"))?
        .to_ascii_lowercase();
    let feature = size_feature(&name, rule)?;
    let first = sized(feature, first)?;
    let mut tests = vec![(after.flipped(), first)];
    if !input.is_exhausted() {
        let comparison = comparison(input)?;
        tests.push((comparison, size_value(feature, input)?));
    }
    exhausted(input)?;
    Ok(Feature::Size { feature, tests })
}

fn exhausted(input: &mut Parser<'_>) -> Result<(), Problem> {
    input
        .expect_exhausted()
        .map_err(|_| Problem::new("invalid feature test"))
}

fn boolean(name: &str, rule: Rule) -> Result<Feature, Problem> {
    match rule {
        Rule::Media => match Preference::from_name(name) {
            Some(preference) => Ok(Feature::Preference {
                name: preference,
                value: None,
            }),
            None => Err(media_feature_problem(name, None)),
        },
        Rule::Container => Err(Problem::new(format!(
            "the container feature `{name}` needs a value"
        ))),
    }
}

fn plain(name: &str, input: &mut Parser<'_>, rule: Rule) -> Result<Feature, Problem> {
    if rule == Rule::Media {
        if let Some(preference) = Preference::from_name(name) {
            let value = input
                .expect_ident()
                .map_err(|_| Problem::new(format!("`{name}` takes a keyword")))?
                .to_ascii_lowercase();
            let value = (preference.values().iter())
                .find(|known| **known == value)
                .ok_or_else(|| Problem::new(format!("`{value}` is not a value of `{name}`")))?;
            exhausted(input)?;
            return Ok(Feature::Preference {
                name: preference,
                value: Some(value),
            });
        }
        let start = input.position();
        while input.next().is_ok() {}
        let value = input.slice_from(start).trim().to_string();
        return Err(media_feature_problem(name, Some(&value)));
    }
    let (base, comparison) = match name.strip_prefix("min-") {
        Some(base) => (base, Comparison::GreaterOrEqual),
        None => match name.strip_prefix("max-") {
            Some(base) => (base, Comparison::LessOrEqual),
            None => (name, Comparison::Equal),
        },
    };
    let feature = size_feature(base, rule)?;
    if feature == SizeFeature::Orientation && comparison != Comparison::Equal {
        return Err(Problem::new(format!(
            "the container feature `{name}` is not supported"
        )));
    }
    let value = size_value(feature, input)?;
    exhausted(input)?;
    Ok(Feature::Size {
        feature,
        tests: vec![(comparison, value)],
    })
}

fn media_feature_problem(name: &str, value: Option<&str>) -> Problem {
    let base = name
        .strip_prefix("min-")
        .or_else(|| name.strip_prefix("max-"))
        .unwrap_or(name);
    let viewport = matches!(
        base,
        "width"
            | "height"
            | "aspect-ratio"
            | "orientation"
            | "device-width"
            | "device-height"
            | "device-aspect-ratio"
            | "resolution"
    );
    if !viewport {
        return Problem::new(format!("the media feature `{name}` is not supported"));
    }
    let condition = match (name.strip_prefix("min-"), name.strip_prefix("max-"), value) {
        (Some(base), _, Some(value)) => format!(" ({base} >= {value})"),
        (_, Some(base), Some(value)) => format!(" ({base} <= {value})"),
        _ => String::new(),
    };
    Problem::new(format!(
        "a chart has no viewport; the media feature `{name}` is not supported"
    ))
    .with_help(format!("use @container{condition} to test the host box"))
}

fn size_feature(name: &str, rule: Rule) -> Result<SizeFeature, Problem> {
    match (rule, SizeFeature::from_name(name)) {
        (Rule::Container, Some(feature)) => Ok(feature),
        (Rule::Media, _) => Err(media_feature_problem(name, None)),
        (Rule::Container, None) => Err(Problem::new(format!(
            "the container feature `{name}` is not supported"
        ))),
    }
}

fn comparison(input: &mut Parser<'_>) -> Result<Comparison, Problem> {
    let invalid = || Problem::new("a range compares with <, <=, >, >=, or =");
    let comparison = match input.next().map_err(|_| invalid())? {
        Token::Delim('<') => Comparison::Less,
        Token::Delim('>') => Comparison::Greater,
        Token::Delim('=') => return Ok(Comparison::Equal),
        _ => return Err(invalid()),
    };
    let state = input.state();
    match input.next_including_whitespace() {
        Ok(Token::Delim('=')) => Ok(match comparison {
            Comparison::Less => Comparison::LessOrEqual,
            _ => Comparison::GreaterOrEqual,
        }),
        _ => {
            input.reset(&state);
            Ok(comparison)
        }
    }
}

/// A value whose feature is not known yet, from the start of a range.
enum Pending {
    Numeric(Calc),
    Ratio(f32),
}

fn value_then_comparison(input: &mut Parser<'_>) -> Result<(Pending, Comparison), Problem> {
    let calc = parse_numeric(input).map_err(|_| Problem::new("invalid range"))?;
    if input.try_parse(|input| input.expect_delim('/')).is_ok() {
        let denominator = number(input)?;
        let numerator = calc
            .constant()
            .ok_or_else(|| Problem::new("invalid ratio"))?;
        return Ok((Pending::Ratio(numerator / denominator), comparison(input)?));
    }
    Ok((Pending::Numeric(calc), comparison(input)?))
}

fn sized(feature: SizeFeature, value: Pending) -> Result<SizeValue, Problem> {
    match (feature, value) {
        (SizeFeature::Width | SizeFeature::Height, Pending::Numeric(calc)) => length(calc),
        (SizeFeature::AspectRatio, Pending::Ratio(ratio)) => Ok(SizeValue::Ratio(ratio)),
        (SizeFeature::AspectRatio, Pending::Numeric(calc)) => calc
            .constant()
            .map(SizeValue::Ratio)
            .ok_or_else(|| Problem::new("an aspect ratio is a number or a ratio")),
        _ => Err(Problem::new("invalid range")),
    }
}

fn size_value(feature: SizeFeature, input: &mut Parser<'_>) -> Result<SizeValue, Problem> {
    match feature {
        SizeFeature::Orientation => {
            let value = input
                .expect_ident()
                .map_err(|_| Problem::new("`orientation` is portrait or landscape"))?
                .to_ascii_lowercase();
            match value.as_str() {
                "portrait" => Ok(SizeValue::Orientation("portrait")),
                "landscape" => Ok(SizeValue::Orientation("landscape")),
                _ => Err(Problem::new("`orientation` is portrait or landscape")),
            }
        }
        SizeFeature::AspectRatio => {
            let numerator = number(input)?;
            let denominator = match input.try_parse(|input| input.expect_delim('/')) {
                Ok(()) => number(input)?,
                Err(_) => 1.0,
            };
            Ok(SizeValue::Ratio(numerator / denominator))
        }
        SizeFeature::Width | SizeFeature::Height => {
            let calc = parse_numeric(input).map_err(|_| Problem::new("a size is a length"))?;
            length(calc)
        }
    }
}

fn length(calc: Calc) -> Result<SizeValue, Problem> {
    match calc.category() {
        Some(Category::Length | Category::Number) => Ok(SizeValue::Length(calc)),
        _ => Err(Problem::new("a size is a length in px, rem, or em")),
    }
}

fn number(input: &mut Parser<'_>) -> Result<f32, Problem> {
    match input.next() {
        Ok(Token::Number { value, .. }) if *value > 0.0 => Ok(*value),
        _ => Err(Problem::new("a ratio is two positive numbers")),
    }
}
