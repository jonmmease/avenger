//! Stylesheets: their rules, at-rules, and declarations, with the checks of the style language.

use std::collections::BTreeSet;

use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, Delimiter, ParseError, ParseErrorKind, Parser,
    ParserState, QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser, ToCss,
    Token,
};
use selectors::parser::SelectorList;

use crate::{
    condition::{parse_container, parse_media, Condition, MediaQuery, Query},
    properties::{Property, Shorthand},
    selector::{self, Selectors},
    source::{Diagnostic, Problem, SourceId, Span},
    values::{self, parse_all, Syntax, Value, Wide},
};

/// What a stylesheet may hold, by where it comes from.
#[derive(Clone, Copy, Debug, Default)]
pub struct ParseOptions {
    /// Whether `@import` rules are valid, as in a stylesheet file. A stylesheet that a host embeds
    /// in another document, and whose imports the host loads, does not allow them.
    pub allow_import: bool,
    /// Whether the reserved layer names `avenger` and its sublayers are valid, as in
    /// `std:reset.css`.
    pub allow_reserved_layers: bool,
}

/// The dotted name of a cascade layer, such as `avenger.reset`, as its parts.
pub type LayerName = Vec<String>;

/// A declaration of a cascade layer by an `@layer` rule or an `@import` rule.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerDeclaration {
    pub name: LayerName,
    /// The conditions of the `@media` and `@container` rules around the declaration, every one of
    /// which holds when the declaration counts.
    pub conditions: Vec<Condition>,
}

/// A parsed stylesheet.
#[derive(Clone, Debug, Default)]
pub struct Stylesheet {
    pub source: SourceId,
    /// The style rules in source order, with nested rules flattened into selectors of their own.
    pub rules: Vec<StyleRule>,
    pub palettes: Vec<PaletteRule>,
    pub imports: Vec<Import>,
    /// The declarations of layers in source order, each once for its conditions. A layer ranks
    /// by its first declaration whose conditions hold.
    pub layers: Vec<LayerDeclaration>,
    env_names: BTreeSet<String>,
}

impl Stylesheet {
    /// Parses `text`, the text of the source `source`. The stylesheet holds every rule and
    /// declaration that parsed, and the diagnostics report the others.
    pub fn parse(
        text: &str,
        source: SourceId,
        options: ParseOptions,
    ) -> (Stylesheet, Vec<Diagnostic>) {
        let mut out = Output {
            text,
            options,
            sheet: Stylesheet {
                source,
                ..Default::default()
            },
            diagnostics: Vec::new(),
            rules_seen: false,
            anonymous_layers: 0,
        };
        let mut parser = Parser::new(text);
        let mut rules = RuleParser {
            out: &mut out,
            context: Context::default(),
        };
        let results: Vec<_> = StyleSheetParser::new(&mut parser, &mut rules).collect();
        for result in results {
            if let Err((error, slice, _)) = result {
                out.report_failure(error, slice, "invalid rule");
            }
        }
        (out.sheet, out.diagnostics)
    }

    /// The names that the declarations and the descriptors of the stylesheet read with `env()`,
    /// which are its parameters.
    pub fn env_names(&self) -> impl Iterator<Item = &str> {
        self.env_names.iter().map(String::as_str)
    }
}

/// A style rule: a selector list and its declarations, inside its layer and its conditions.
#[derive(Clone, Debug)]
pub struct StyleRule {
    pub selectors: SelectorList<Selectors>,
    pub declarations: Vec<Declaration>,
    pub layer: Option<LayerName>,
    /// The conditions of the rule's `@media` and `@container` rules, every one of which holds
    /// when the rule applies.
    pub conditions: Vec<Condition>,
    /// The span of the rule's selectors.
    pub span: Span,
}

/// A declaration of a longhand property or of a custom property.
#[derive(Clone, Debug, PartialEq)]
pub struct Declaration {
    pub name: Name,
    pub value: Declared,
    pub important: bool,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Name {
    Property(Property),
    /// A custom property, with its name and its leading `--`.
    Custom(String),
}

/// A declared value.
#[derive(Clone, Debug, PartialEq)]
pub enum Declared {
    Value(Value),
    Wide(Wide),
    /// A value with `var()` or `env()`, whose text the cascade substitutes and parses again: for
    /// the property, or for its shorthand when one declared it.
    Pending {
        text: String,
        shorthand: Option<Shorthand>,
    },
    /// The text of a custom property.
    Tokens(String),
}

/// An `@import` rule, which the caller loads.
#[derive(Clone, Debug, PartialEq)]
pub struct Import {
    pub url: String,
    /// The layer of the imported rules, when the rule names one with `layer(...)`.
    pub layer: Option<LayerName>,
    /// How many of the stylesheet's layer declarations come before the import, with the
    /// import's own layer, so the layers that the imported stylesheet declares come after those
    /// and before the rest.
    pub layers_before: usize,
    pub span: Span,
}

/// An `@palette` rule.
#[derive(Clone, Debug, PartialEq)]
pub struct PaletteRule {
    /// The name, in lowercase, because palette names are case-insensitive.
    pub name: String,
    pub kind: PaletteKind,
    /// The colors: a list of colors, or the text of a value with `env()`.
    pub colors: Declared,
    pub interpolate: Interpolation,
    pub cvd_safe: bool,
    pub layer: Option<LayerName>,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PaletteKind {
    Categorical,
    Sequential,
    Diverging,
}

impl PaletteKind {
    /// The fewest colors that a palette of the kind lists, with the message of a palette that
    /// lists fewer.
    pub fn min_colors(self) -> (usize, &'static str) {
        match self {
            PaletteKind::Categorical => (1, "a categorical palette needs at least one color"),
            PaletteKind::Sequential => (2, "a sequential palette needs at least two colors"),
            PaletteKind::Diverging => (3, "a diverging palette needs at least three colors"),
        }
    }
}

/// The color space in which a palette interpolates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Interpolation {
    Srgb,
    SrgbLinear,
    Lab,
    Oklab,
    Lch,
    Oklch,
    Hsl,
}

/// The state that the rule parsers share: the stylesheet so far and the diagnostics.
struct Output<'t> {
    text: &'t str,
    options: ParseOptions,
    sheet: Stylesheet,
    diagnostics: Vec<Diagnostic>,
    /// Whether a rule other than `@import` or a `@layer` statement came before, after which an
    /// `@import` is invalid.
    rules_seen: bool,
    anonymous_layers: usize,
}

/// An error that a diagnostic already reports.
struct Reported;

type Failure = ParseError<Reported>;

impl Output<'_> {
    fn span(&self, start: usize, end: usize) -> Span {
        Span::new(self.sheet.source, start, end.max(start))
    }

    /// The span of `slice`, a slice of the stylesheet's text.
    fn slice_span(&self, slice: &str) -> Span {
        let start = (slice.as_ptr() as usize).saturating_sub(self.text.as_ptr() as usize);
        self.span(start, start + slice.trim_end().len())
    }

    fn error(&mut self, span: Span, problem: Problem) -> Reported {
        self.diagnostics.push(problem.at(span));
        Reported
    }

    /// Reports a rule or a declaration that failed, unless a diagnostic already reports it.
    fn report_failure(&mut self, error: ParseError<Reported>, slice: &str, what: &str) {
        if !matches!(error.kind, ParseErrorKind::Custom(Reported)) {
            let span = self.slice_span(slice);
            self.diagnostics.push(Diagnostic::error(span, what));
        }
    }

    fn declare_layer(&mut self, name: &LayerName, conditions: &[Condition]) {
        // Declaring `a.b` also declares `a`.
        for length in 1..=name.len() {
            let declaration = LayerDeclaration {
                name: name[..length].to_vec(),
                conditions: conditions.to_vec(),
            };
            if !self.sheet.layers.contains(&declaration) {
                self.sheet.layers.push(declaration);
            }
        }
    }
}

/// Where a rule stands: its parent's selectors for a nested rule, its layer, and its
/// conditions.
#[derive(Clone, Default)]
struct Context {
    parent: Option<SelectorList<Selectors>>,
    layer: Option<LayerName>,
    conditions: Vec<Condition>,
}

impl Context {
    fn layer(&self, name: &LayerName) -> LayerName {
        let mut full = self.layer.clone().unwrap_or_default();
        full.extend(name.iter().cloned());
        full
    }
}

/// Parses the rules of a stylesheet or of the block of a conditional or layer rule.
struct RuleParser<'o, 't> {
    out: &'o mut Output<'t>,
    context: Context,
}

enum AtPrelude {
    Import(Import),
    /// The names of an `@layer` rule: a statement without a block, or a block of one name or of
    /// none for an anonymous layer.
    Layer(Vec<LayerName>),
    Media(Vec<MediaQuery>),
    Container(Query),
    Palette(String, Span),
}

impl<'i> AtRuleParser<'i> for RuleParser<'_, '_> {
    type Prelude = AtPrelude;
    type AtRule = ();
    type Error = Reported;

    fn parse_prelude(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
    ) -> Result<AtPrelude, Failure> {
        // The spans of the prelude's diagnostics start at its text, after the at-rule's name.
        input.skip_whitespace();
        let start = input.position().byte_index();
        let at_top = self.context.layer.is_none() && self.context.conditions.is_empty();
        let name = name.to_ascii_lowercase();
        let prelude = at_prelude(self.out, &self.context, &name, input, start, at_top);
        // An `@layer` rule is a statement or a block, which only its end tells apart.
        if !matches!(prelude, Ok(AtPrelude::Import(_)) | Ok(AtPrelude::Layer(_))) {
            self.out.rules_seen = true;
        }
        prelude.map_err(ParseError::custom)
    }

    fn rule_without_block(&mut self, prelude: AtPrelude, _start: &ParserState) -> Result<(), ()> {
        match prelude {
            AtPrelude::Import(mut import) => {
                import.layers_before = self.out.sheet.layers.len();
                self.out.sheet.imports.push(import);
            }
            AtPrelude::Layer(names) if !names.is_empty() => {
                for name in names {
                    let full = self.context.layer(&name);
                    self.out.declare_layer(&full, &self.context.conditions);
                }
            }
            _ => return Err(()),
        }
        Ok(())
    }

    fn parse_block(
        &mut self,
        prelude: AtPrelude,
        _start: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<(), Failure> {
        let mut context = self.context.clone();
        match prelude {
            AtPrelude::Layer(names) if names.len() <= 1 => {
                self.out.rules_seen = true;
                let name = match names.into_iter().next() {
                    Some(name) => name,
                    None => anonymous_layer(self.out),
                };
                let full = self.context.layer(&name);
                self.out.declare_layer(&full, &self.context.conditions);
                context.layer = Some(full);
            }
            AtPrelude::Media(queries) => context.conditions.push(Condition::Media(queries)),
            AtPrelude::Container(query) => context.conditions.push(Condition::Container(query)),
            AtPrelude::Palette(name, span) => {
                palette(self.out, &context, name, span, input);
                return Ok(());
            }
            AtPrelude::Import(import) => {
                let reported = self
                    .out
                    .error(import.span, Problem::new("an @import rule has no block"));
                return Err(ParseError::custom(reported));
            }
            AtPrelude::Layer(_) => {
                let span = self
                    .out
                    .span(input.position().byte_index(), input.position().byte_index());
                let reported = self
                    .out
                    .error(span, Problem::new("a layer block names one layer"));
                return Err(ParseError::custom(reported));
            }
        }
        rule_list(self.out, context, input);
        Ok(())
    }
}

/// Parses the rules of a block.
fn rule_list(out: &mut Output<'_>, context: Context, input: &mut Parser<'_>) {
    let mut rules = RuleParser { out, context };
    let results: Vec<_> = RuleBodyParser::new(input, &mut rules).collect();
    for result in results {
        if let Err((error, slice, _)) = result {
            rules.out.report_failure(error, slice, "invalid rule");
        }
    }
}

fn at_prelude<'i>(
    out: &mut Output<'_>,
    context: &Context,
    name: &str,
    input: &mut Parser<'i>,
    start: usize,
    at_top: bool,
) -> Result<AtPrelude, Reported> {
    let prelude_span =
        |out: &Output<'_>, input: &Parser<'i>| out.span(start, input.position().byte_index());
    match name {
        "import" => {
            let url = match input.next() {
                Ok(Token::QuotedString(url)) => Some(url.to_string()),
                Ok(Token::UnquotedUrl(url)) => Some(url.to_string()),
                Ok(Token::Function(function)) if function.eq_ignore_ascii_case("url") => input
                    .parse_nested_block(|input| {
                        input
                            .expect_string()
                            .map(|url| url.to_string())
                            .map_err(Into::<ParseError<()>>::into)
                    })
                    .ok(),
                Ok(Token::Function(function)) if function.eq_ignore_ascii_case("env") => {
                    let span = prelude_span(out, input);
                    return Err(
                        out.error(span, Problem::new("env() is not valid in an @import URL"))
                    );
                }
                _ => None,
            };
            let span = prelude_span(out, input);
            let Some(url) = url else {
                return Err(out.error(span, Problem::new("an @import rule names a URL")));
            };
            if !out.options.allow_import {
                return Err(out.error(
                    span,
                    Problem::new("@import is valid only in a stylesheet file")
                        .with_help(format!("load `{url}` through the host")),
                ));
            }
            if out.rules_seen || !at_top {
                return Err(out.error(
                    span,
                    Problem::new(
                        "an @import rule comes before every other rule except @layer statements",
                    ),
                ));
            }
            let layer = match input.try_parse(|input| input.expect_ident_matching("layer")) {
                Ok(()) => Some(anonymous_layer(out)),
                Err(_) => match input.try_parse(|input| input.expect_function_matching("layer")) {
                    Ok(()) => Some(
                        input
                            .parse_nested_block(|input| {
                                layer_name(input).map_err(|()| ParseError::<()>::custom(()))
                            })
                            .map_err(|_| {
                                let span = prelude_span(out, input);
                                out.error(span, Problem::new("invalid layer name"))
                            })?,
                    ),
                    Err(_) => None,
                },
            };
            if !input.is_exhausted() {
                let span = prelude_span(out, input);
                return Err(out.error(
                    span,
                    Problem::new("an @import rule takes a URL and an optional layer"),
                ));
            }
            if let Some(layer) = &layer {
                check_layer(out, layer, span)?;
                let full = context.layer(layer);
                out.declare_layer(&full, &context.conditions);
            }
            Ok(AtPrelude::Import(Import {
                url,
                layer: layer.map(|layer| context.layer(&layer)),
                layers_before: 0,
                span,
            }))
        }
        "layer" => {
            if input.is_exhausted() {
                return Ok(AtPrelude::Layer(Vec::new()));
            }
            let names = input
                .parse_comma_separated(|input| {
                    layer_name(input).map_err(|()| ParseError::<()>::custom(()))
                })
                .map_err(|_| {
                    let span = prelude_span(out, input);
                    out.error(span, Problem::new("invalid layer name"))
                })?;
            let span = prelude_span(out, input);
            for name in &names {
                check_layer(out, name, span)?;
            }
            Ok(AtPrelude::Layer(names))
        }
        "media" => parse_media(input).map(AtPrelude::Media).map_err(|problem| {
            let span = prelude_span(out, input);
            out.error(span, problem)
        }),
        "container" => parse_container(input)
            .map(AtPrelude::Container)
            .map_err(|problem| {
                let span = prelude_span(out, input);
                out.error(span, problem)
            }),
        "palette" => {
            let name = input.expect_ident().map(|name| name.to_ascii_lowercase());
            let span = prelude_span(out, input);
            let name = match (name, input.is_exhausted()) {
                (Ok(name), true) => name,
                _ => return Err(out.error(span, Problem::new("an @palette rule takes one name"))),
            };
            if !context.conditions.is_empty() {
                return Err(out.error(
                    span,
                    Problem::new(
                        "an @palette rule is valid only outside @media and @container rules",
                    ),
                ));
            }
            Ok(AtPrelude::Palette(name, span))
        }
        _ => {
            let span = prelude_span(out, input);
            Err(out.error(
                span,
                Problem::new(format!("the at-rule `@{name}` is not supported")),
            ))
        }
    }
}

fn anonymous_layer(out: &mut Output<'_>) -> LayerName {
    out.anonymous_layers += 1;
    // A name that no stylesheet can write, because it is not an identifier.
    vec![format!(" anonymous {}", out.anonymous_layers)]
}

fn layer_name(input: &mut Parser<'_>) -> Result<LayerName, ()> {
    let mut name = vec![input.expect_ident().map_err(|_| ())?.to_string()];
    loop {
        let state = input.state();
        match input.next_including_whitespace() {
            Ok(Token::Delim('.')) => {
                name.push(input.expect_ident().map_err(|_| ())?.to_string());
            }
            _ => {
                input.reset(&state);
                return Ok(name);
            }
        }
    }
}

fn check_layer(out: &mut Output<'_>, name: &LayerName, span: Span) -> Result<(), Reported> {
    if name.first().is_some_and(|first| first == "avenger") && !out.options.allow_reserved_layers {
        return Err(out.error(span, Problem::new("the layer name `avenger` is reserved")));
    }
    Ok(())
}

/// The selectors of a qualified rule and their span.
struct RulePrelude {
    selectors: SelectorList<Selectors>,
    span: Span,
    /// Whether the selectors match only nodes with keyboard focus, so the rule sets only the
    /// outline properties.
    focus: bool,
}

impl<'i> QualifiedRuleParser<'i> for RuleParser<'_, '_> {
    type Prelude = RulePrelude;
    type QualifiedRule = ();
    type Error = Reported;

    fn parse_prelude(&mut self, input: &mut Parser<'i>) -> Result<RulePrelude, Failure> {
        self.out.rules_seen = true;
        selectors(self.out, self.context.parent.as_ref(), input).map_err(ParseError::custom)
    }

    fn parse_block(
        &mut self,
        prelude: RulePrelude,
        _start: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<(), Failure> {
        style_block(self.out, &self.context, prelude, input);
        Ok(())
    }
}

impl<'i> DeclarationParser<'i> for RuleParser<'_, '_> {
    type Declaration = ();
    type Error = Reported;

    fn parse_value(
        &mut self,
        _name: CowRcStr<'i>,
        input: &mut Parser<'i>,
        start: &ParserState,
    ) -> Result<(), Failure> {
        let span = self
            .out
            .span(start.position().byte_index(), input.position().byte_index());
        let reported = self.out.error(
            span,
            Problem::new("a declaration is valid only inside a style rule"),
        );
        Err(ParseError::custom(reported))
    }
}

impl<'i> RuleBodyItemParser<'i, (), Reported> for RuleParser<'_, '_> {
    fn parse_declarations(&self) -> bool {
        false
    }

    fn parse_qualified(&self) -> bool {
        true
    }
}

/// Parses the selectors of a rule, relative to `parent` when the rule is nested, and checks them.
fn selectors(
    out: &mut Output<'_>,
    parent: Option<&SelectorList<Selectors>>,
    input: &mut Parser<'_>,
) -> Result<RulePrelude, Reported> {
    let start = input.position().byte_index();
    let parsed = selector::parse_list(input, parent.is_some());
    let end = input.position().byte_index();
    let list = match parsed {
        Ok(list) => list,
        Err(error) => {
            let (message, help) =
                match (&error.kind, selector::non_identifier_id(&out.text[start..])) {
                    (_, Some(id)) => id,
                    (ParseErrorKind::Custom(error), None) => selector::parse_error_message(error),
                    _ => ("invalid selector".to_string(), None),
                };
            let mut problem = Problem::new(message);
            problem.help = help;
            let span = out.span(end, end);
            return Err(out.error(span, problem));
        }
    };
    let span = out.span(start, start + out.text[start..end].trim_end().len());
    let list = match parent {
        Some(parent) => list.replace_parent_selector(parent),
        None => {
            if list
                .slice()
                .iter()
                .any(|selector| selector.has_parent_selector())
            {
                return Err(out.error(span, Problem::new("`&` is valid only in a nested rule")));
            }
            list
        }
    };
    // The check reads the selectors with the parent in place of `&`, so a nested rule is checked
    // as the rule that it is.
    let focus = selector::check_list(&list).map_err(|problem| out.error(span, problem))?;
    Ok(RulePrelude {
        selectors: list,
        span,
        focus,
    })
}

/// Parses the block of a style rule: its declarations, and its nested rules and conditional
/// rules, which flatten into rules of their own after the declarations before them.
fn style_block(
    out: &mut Output<'_>,
    context: &Context,
    prelude: RulePrelude,
    input: &mut Parser<'_>,
) {
    let mut body = BodyParser {
        out,
        context: Context {
            parent: Some(prelude.selectors.clone()),
            ..context.clone()
        },
        span: prelude.span,
        run: Vec::new(),
        focus: prelude.focus,
        failed_at: None,
    };
    let results: Vec<_> = RuleBodyParser::new(input, &mut body).collect();
    for result in results {
        if let Err((error, slice, _)) = result {
            body.out.report_failure(error, slice, "invalid declaration");
        }
    }
    body.flush();
}

/// Parses the body of a style rule.
struct BodyParser<'o, 't> {
    out: &'o mut Output<'t>,
    /// The context of nested rules, whose parent is this rule.
    context: Context,
    span: Span,
    /// The declarations since the start of the block or the last nested rule.
    run: Vec<Declaration>,
    /// Whether the rule's selectors end with `:focus-visible`.
    focus: bool,
    /// The offset of the last declaration that failed. The rule body parser then tries the same
    /// text as a nested rule, whose failure the declaration's diagnostic already reports.
    failed_at: Option<usize>,
}

impl BodyParser<'_, '_> {
    /// Adds the declarations of the current run as a rule of the parent's selectors.
    fn flush(&mut self) {
        if self.run.is_empty() {
            return;
        }
        let selectors = self
            .context
            .parent
            .clone()
            .expect("a body has its rule's selectors");
        self.out.sheet.rules.push(StyleRule {
            selectors,
            declarations: std::mem::take(&mut self.run),
            layer: self.context.layer.clone(),
            conditions: self.context.conditions.clone(),
            span: self.span,
        });
    }
}

impl<'i> DeclarationParser<'i> for BodyParser<'_, '_> {
    type Declaration = ();
    type Error = Reported;

    fn parse_value(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
        start: &ParserState,
    ) -> Result<(), Failure> {
        let declarations =
            declaration(self.out, &name, input, start, self.focus).map_err(|reported| {
                self.failed_at = Some(start.position().byte_index());
                ParseError::custom(reported)
            })?;
        self.run.extend(declarations);
        Ok(())
    }
}

impl<'i> QualifiedRuleParser<'i> for BodyParser<'_, '_> {
    type Prelude = RulePrelude;
    type QualifiedRule = ();
    type Error = Reported;

    fn parse_prelude(&mut self, input: &mut Parser<'i>) -> Result<RulePrelude, Failure> {
        if self.failed_at == Some(input.position().byte_index()) {
            return Err(ParseError::custom(Reported));
        }
        selectors(self.out, self.context.parent.as_ref(), input).map_err(ParseError::custom)
    }

    fn parse_block(
        &mut self,
        prelude: RulePrelude,
        _start: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<(), Failure> {
        self.flush();
        let context = Context {
            parent: None,
            ..self.context.clone()
        };
        style_block(self.out, &context, prelude, input);
        Ok(())
    }
}

impl<'i> AtRuleParser<'i> for BodyParser<'_, '_> {
    type Prelude = AtPrelude;
    type AtRule = ();
    type Error = Reported;

    fn parse_prelude(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
    ) -> Result<AtPrelude, Failure> {
        input.skip_whitespace();
        let start = input.position().byte_index();
        let name = name.to_ascii_lowercase();
        match name.as_str() {
            "media" | "container" => {
                at_prelude(self.out, &self.context, &name, input, start, false)
                    .map_err(ParseError::custom)
            }
            _ => {
                let span = self.out.span(start, input.position().byte_index());
                let reported = self.out.error(
                    span,
                    Problem::new(format!(
                        "the at-rule `@{name}` is not valid inside a style rule"
                    )),
                );
                Err(ParseError::custom(reported))
            }
        }
    }

    fn parse_block(
        &mut self,
        prelude: AtPrelude,
        _start: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<(), Failure> {
        self.flush();
        let mut context = self.context.clone();
        match prelude {
            AtPrelude::Media(queries) => context.conditions.push(Condition::Media(queries)),
            AtPrelude::Container(query) => context.conditions.push(Condition::Container(query)),
            _ => return Ok(()),
        }
        // A conditional rule inside a style rule holds declarations of the style rule and nested
        // rules relative to it.
        let parent = context
            .parent
            .clone()
            .expect("a body has its rule's selectors");
        let prelude = RulePrelude {
            selectors: parent,
            span: self.span,
            focus: self.focus,
        };
        let outer = Context {
            parent: None,
            ..context
        };
        style_block(self.out, &outer, prelude, input);
        Ok(())
    }
}

impl<'i> RuleBodyItemParser<'i, (), Reported> for BodyParser<'_, '_> {
    fn parse_declarations(&self) -> bool {
        true
    }

    fn parse_qualified(&self) -> bool {
        true
    }
}

/// Parses one declaration into the declarations of its longhands.
fn declaration<'i>(
    out: &mut Output<'_>,
    name: &str,
    input: &mut Parser<'i>,
    start: &ParserState,
    focus: bool,
) -> Result<Vec<Declaration>, Reported> {
    let value_start = input.position();
    let mut important = false;
    let _ = input.parse_until_before(Delimiter::Bang, |input| {
        while input.next().is_ok() {}
        Ok::<_, ParseError<()>>(())
    });
    let text = input.slice_from(value_start).trim().to_string();
    if input.try_parse(cssparser::parse_important).is_ok() {
        important = true;
    }
    let span = out.span(start.position().byte_index(), input.position().byte_index());
    if !input.is_exhausted() {
        return Err(out.error(span, Problem::new(format!("invalid value of `{name}`"))));
    }
    if text.is_empty() {
        return Err(out.error(span, Problem::new(format!("`{name}` has no value"))));
    }
    let scan = scan(&text).map_err(|problem| out.error(span, problem))?;
    out.sheet.env_names.extend(scan.env_names.iter().cloned());

    if name.starts_with("--") {
        let wide = Wide::parse(&mut Parser::new(&text));
        let value = match wide {
            Some(wide) => Declared::Wide(wide),
            None => Declared::Tokens(text),
        };
        return Ok(vec![Declaration {
            name: Name::Custom(name.to_string()),
            value,
            important,
            span,
        }]);
    }
    // Property names are ASCII case-insensitive, as in CSS, and custom property names are not.
    let name = &name.to_ascii_lowercase();
    if matches!(
        name.as_str(),
        "container-type" | "container-name" | "container"
    ) {
        return Err(out.error(
            span,
            Problem::new(format!(
                "`{name}` is not supported; the host box is the only container"
            )),
        ));
    }
    let (longhands, shorthand) = match (Property::from_name(name), Shorthand::from_name(name)) {
        (Some(property), _) => (vec![property], None),
        (None, Some(shorthand)) => {
            let longhands = match (shorthand, focus) {
                (Shorthand::All, true) => (Property::ALL.iter())
                    .copied()
                    .filter(|property| property.is_outline())
                    .collect(),
                _ => shorthand.longhands(),
            };
            (longhands, Some(shorthand))
        }
        (None, None) => {
            out.diagnostics.push(Diagnostic::warning(
                span,
                format!("unknown property `{name}`"),
            ));
            return Ok(Vec::new());
        }
    };
    let outline = shorthand.map_or_else(|| longhands[0].is_outline(), Shorthand::is_outline);
    if outline && !focus {
        return Err(out.error(
            span,
            Problem::new("outline properties apply only through :focus-visible"),
        ));
    }
    if !outline && focus && shorthand != Some(Shorthand::All) {
        return Err(out.error(
            span,
            Problem::new("only outline properties apply through :focus-visible"),
        ));
    }

    let declare = |property: Property, value: Declared| Declaration {
        name: Name::Property(property),
        value,
        important,
        span,
    };
    if scan.substitutes {
        return Ok(longhands
            .into_iter()
            .map(|property| {
                declare(
                    property,
                    Declared::Pending {
                        text: text.clone(),
                        shorthand,
                    },
                )
            })
            .collect());
    }
    let mut parser = Parser::new(&text);
    if let Some(wide) = Wide::parse(&mut parser) {
        return Ok(longhands
            .into_iter()
            .map(|property| declare(property, Declared::Wide(wide)))
            .collect());
    }
    let values = match shorthand {
        None => {
            let property = longhands[0];
            let value = parse_all(property.syntax(), &mut parser)
                .map_err(|()| out.error(span, invalid_value(name, &text, property.syntax())))?;
            vec![(property, Declared::Value(value))]
        }
        Some(shorthand) => expand(shorthand, &text)
            .ok_or_else(|| out.error(span, invalid_shorthand(shorthand, &text)))?,
    };
    Ok(values
        .into_iter()
        .map(|(property, value)| declare(property, value))
        .collect())
}

/// The longhand values of a shorthand's value, which has no `var()` or `env()` and is not a
/// CSS-wide keyword.
pub(crate) fn expand(shorthand: Shorthand, text: &str) -> Option<Vec<(Property, Declared)>> {
    let mut parser = Parser::new(text);
    let values = |syntax: &Syntax, parser: &mut Parser<'_>| match parse_all(syntax, parser) {
        Ok(Value::Sequence(values)) => Some(values),
        _ => None,
    };
    Some(match shorthand {
        Shorthand::Padding => {
            let sides = values(&Syntax::Repeat(&SIZE_FOR_PADDING, 1, 4), &mut parser)?;
            let [top, right, bottom, left] = match sides.as_slice() {
                [all] => [all, all, all, all],
                [vertical, horizontal] => [vertical, horizontal, vertical, horizontal],
                [top, horizontal, bottom] => [top, horizontal, bottom, horizontal],
                [top, right, bottom, left] => [top, right, bottom, left],
                _ => return None,
            };
            let mut expanded = vec![
                (Property::PaddingTop, Declared::Value(top.clone())),
                (Property::PaddingRight, Declared::Value(right.clone())),
                (Property::PaddingBottom, Declared::Value(bottom.clone())),
                (Property::PaddingLeft, Declared::Value(left.clone())),
            ];
            // One number is also the padding of a scale.
            if let [Value::Numeric(values::Calc::Leaf(number, values::Unit::Number))] =
                sides.as_slice()
            {
                if (0.0..=1.0).contains(number) {
                    expanded.push((Property::PaddingInner, Declared::Value(sides[0].clone())));
                }
                expanded.push((Property::PaddingOuter, Declared::Value(sides[0].clone())));
            }
            expanded
        }
        Shorthand::Gap => {
            let lengths = values(&Syntax::Repeat(&SIZE_FOR_PADDING, 1, 2), &mut parser)?;
            let (row, column) = match lengths.as_slice() {
                [both] => (both.clone(), both.clone()),
                [row, column] => (row.clone(), column.clone()),
                _ => return None,
            };
            vec![
                (Property::RowGap, Declared::Value(row)),
                (Property::ColumnGap, Declared::Value(column)),
            ]
        }
        Shorthand::Border | Shorthand::Outline => {
            let parts = values(&Syntax::Line, &mut parser)?;
            let (width, style, color) = match shorthand {
                Shorthand::Border => (
                    Property::BorderWidth,
                    Property::BorderStyle,
                    Property::BorderColor,
                ),
                _ => (
                    Property::OutlineWidth,
                    Property::OutlineStyle,
                    Property::OutlineColor,
                ),
            };
            let mut expanded = vec![
                (width, Declared::Wide(Wide::Initial)),
                (style, Declared::Wide(Wide::Initial)),
                (color, Declared::Wide(Wide::Initial)),
            ];
            for part in parts {
                let slot = match &part {
                    Value::Numeric(_) => 0,
                    Value::Keyword(_) => 1,
                    Value::Color(_) => 2,
                    _ => return None,
                };
                expanded[slot].1 = Declared::Value(part);
            }
            expanded
        }
        Shorthand::All => return None,
    })
}

const SIZE_FOR_PADDING: Syntax = Syntax::Length { min: Some(0.0) };

fn invalid_value(name: &str, text: &str, syntax: &Syntax) -> Problem {
    let message = format!("`{text}` is not a valid value of `{name}`");
    match syntax {
        Syntax::Keywords(keywords) => Problem::new(format!("{message}; use {}", or_list(keywords))),
        Syntax::Boolean => Problem::new(format!("{message}; use true or false")),
        _ => Problem::new(message),
    }
}

fn invalid_shorthand(shorthand: Shorthand, text: &str) -> Problem {
    let name = shorthand.name();
    let message = format!("`{text}` is not a valid value of `{name}`");
    match shorthand {
        Shorthand::Gap => Problem::new(format!("{message}; use one length or two")),
        Shorthand::Padding => Problem::new(format!("{message}; use one to four lengths")),
        Shorthand::All => Problem::new(format!("`{name}` takes only initial, inherit, or unset")),
        _ => Problem::new(message),
    }
}

fn or_list(words: &[&str]) -> String {
    match words {
        [] => String::new(),
        [one] => one.to_string(),
        [first, second] => format!("{first} or {second}"),
        [rest @ .., last] => format!("{}, or {last}", rest.join(", ")),
    }
}

/// What the tokens of a value hold: whether a `var()` or an `env()` makes it a value to
/// substitute, and the names that its `env()` calls read.
struct Scan {
    substitutes: bool,
    env_names: Vec<String>,
}

fn scan(text: &str) -> Result<Scan, Problem> {
    let mut parser = Parser::new(text);
    let mut scan = Scan {
        substitutes: false,
        env_names: Vec::new(),
    };
    scan_tokens(&mut parser, &mut scan)?;
    Ok(scan)
}

fn scan_tokens(input: &mut Parser<'_>, scan: &mut Scan) -> Result<(), Problem> {
    while let Ok(token) = input.next() {
        let token = token.clone();
        match token {
            Token::Function(name) => {
                let name = name.to_ascii_lowercase();
                let result = input.parse_nested_block(|input| {
                    let found = match name.as_str() {
                        "var" => {
                            scan.substitutes = true;
                            match input.expect_ident() {
                                Ok(name) if name.starts_with("--") => Ok(()),
                                _ => Err(Problem::new(
                                    "var() names a custom property, such as var(--accent)",
                                )),
                            }
                        }
                        "env" => {
                            scan.substitutes = true;
                            match input.expect_ident() {
                                Ok(name) => {
                                    scan.env_names.push(name.to_string());
                                    Ok(())
                                }
                                Err(_) => Err(Problem::new(
                                    "env() names a parameter, such as env(accent)",
                                )),
                            }
                        }
                        _ => Ok(()),
                    };
                    let found = found.and_then(|()| {
                        if matches!(name.as_str(), "var" | "env") {
                            match input.next() {
                                Err(_) | Ok(Token::Comma) => {}
                                Ok(_) => {
                                    return Err(Problem::new(format!(
                                    "{name}() takes a name and an optional fallback after a comma"
                                )))
                                }
                            }
                        }
                        scan_tokens(input, scan)
                    });
                    found.map_err(ParseError::<Problem>::custom)
                });
                if let Err(error) = result {
                    return Err(match error.kind {
                        ParseErrorKind::Custom(problem) => problem,
                        _ => Problem::new("invalid value"),
                    });
                }
            }
            token if token.is_parse_error() => {
                return Err(Problem::new(format!(
                    "unexpected `{}` in a value",
                    token.to_css_string()
                )));
            }
            Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                let result = input.parse_nested_block(|input| {
                    scan_tokens(input, scan).map_err(ParseError::<Problem>::custom)
                });
                if let Err(error) = result {
                    return Err(match error.kind {
                        ParseErrorKind::Custom(problem) => problem,
                        _ => Problem::new("invalid value"),
                    });
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Parses the block of an `@palette` rule.
fn palette(
    out: &mut Output<'_>,
    context: &Context,
    name: String,
    span: Span,
    input: &mut Parser<'_>,
) {
    let mut descriptors = Descriptors {
        out,
        entries: Vec::new(),
    };
    let results: Vec<_> = RuleBodyParser::new(input, &mut descriptors).collect();
    for result in results {
        if let Err((error, slice, _)) = result {
            descriptors
                .out
                .report_failure(error, slice, "invalid descriptor");
        }
    }
    let entries = descriptors.entries;
    let out = descriptors.out;
    let get = |key: &str| entries.iter().rev().find(|(name, ..)| name == key);
    let Some((_, kind_text, kind_span)) = get("kind") else {
        out.error(
            span,
            Problem::new(format!("@palette `{name}` has no `kind`")),
        );
        return;
    };
    let kind = match descriptor_keyword(kind_text).as_deref() {
        Some("categorical") => PaletteKind::Categorical,
        Some("sequential") => PaletteKind::Sequential,
        Some("diverging") => PaletteKind::Diverging,
        _ => {
            out.error(
                *kind_span,
                Problem::new(format!(
                    "`{kind_text}` is not a valid value of `kind`; use categorical, sequential, or diverging"
                )),
            );
            return;
        }
    };
    let Some((_, colors_text, colors_span)) = get("colors") else {
        out.error(
            span,
            Problem::new(format!("@palette `{name}` has no `colors`")),
        );
        return;
    };
    let colors = match scan(colors_text) {
        Err(problem) => {
            out.error(*colors_span, problem);
            return;
        }
        Ok(scan) if scan.substitutes => {
            out.sheet.env_names.extend(scan.env_names);
            Declared::Pending {
                text: colors_text.clone(),
                shorthand: None,
            }
        }
        Ok(_) => {
            let mut parser = Parser::new(colors_text);
            let colors = parser.parse_comma_separated(|input| {
                values::parse_color(input).map_err(|()| ParseError::<()>::custom(()))
            });
            let colors = match colors {
                Ok(colors) if parser.is_exhausted() => colors,
                _ => {
                    out.error(
                        *colors_span,
                        Problem::new("the colors of a palette are colors separated by commas"),
                    );
                    return;
                }
            };
            let (least, message) = kind.min_colors();
            if colors.len() < least {
                out.error(*colors_span, Problem::new(message));
                return;
            }
            Declared::Value(Value::List(colors.into_iter().map(Value::Color).collect()))
        }
    };
    let interpolate = match get("interpolate") {
        None => Interpolation::Oklab,
        Some((_, text, at)) => match descriptor_keyword(text).as_deref() {
            Some("srgb") => Interpolation::Srgb,
            Some("srgb-linear") => Interpolation::SrgbLinear,
            Some("lab") => Interpolation::Lab,
            Some("oklab") => Interpolation::Oklab,
            Some("lch") => Interpolation::Lch,
            Some("oklch") => Interpolation::Oklch,
            Some("hsl") => Interpolation::Hsl,
            _ => {
                out.error(
                    *at,
                    Problem::new(format!(
                        "`{text}` is not a valid value of `interpolate`; use srgb, srgb-linear, lab, oklab, lch, oklch, or hsl"
                    )),
                );
                return;
            }
        },
    };
    let cvd_safe = match get("cvd-safe") {
        None => false,
        Some((_, text, at)) => match descriptor_keyword(text).as_deref() {
            Some("true") => true,
            Some("false") => false,
            _ => {
                out.error(
                    *at,
                    Problem::new(format!(
                        "`{text}` is not a valid value of `cvd-safe`; use true or false"
                    )),
                );
                return;
            }
        },
    };
    out.sheet.palettes.push(PaletteRule {
        name,
        kind,
        colors,
        interpolate,
        cvd_safe,
        layer: context.layer.clone(),
        span,
    });
}

/// The keyword of a descriptor's value, in lowercase, when the value is one identifier, which
/// comments and whitespace can surround.
fn descriptor_keyword(text: &str) -> Option<String> {
    let mut parser = Parser::new(text);
    let keyword = parser.expect_ident().ok()?.to_ascii_lowercase();
    parser.expect_exhausted().ok()?;
    Some(keyword)
}

/// Parses the descriptors of an `@palette` rule as their texts.
struct Descriptors<'o, 't> {
    out: &'o mut Output<'t>,
    entries: Vec<(String, String, Span)>,
}

impl<'i> DeclarationParser<'i> for Descriptors<'_, '_> {
    type Declaration = ();
    type Error = Reported;

    fn parse_value(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
        start: &ParserState,
    ) -> Result<(), Failure> {
        let value_start = input.position();
        while input.next().is_ok() {}
        let text = input.slice_from(value_start).trim().to_string();
        let span = self
            .out
            .span(start.position().byte_index(), input.position().byte_index());
        let name = name.to_ascii_lowercase();
        if !matches!(
            name.as_str(),
            "kind" | "colors" | "interpolate" | "cvd-safe"
        ) {
            let reported = self.out.error(
                span,
                Problem::new(format!("`{name}` is not a descriptor of @palette")),
            );
            return Err(ParseError::custom(reported));
        }
        if name != "colors" {
            if let Ok(scan) = scan(&text) {
                if scan.substitutes {
                    let reported = self.out.error(
                        span,
                        Problem::new(format!("`{name}` of @palette takes no env()")),
                    );
                    return Err(ParseError::custom(reported));
                }
            }
        }
        self.entries.push((name, text, span));
        Ok(())
    }
}

impl<'i> AtRuleParser<'i> for Descriptors<'_, '_> {
    type Prelude = ();
    type AtRule = ();
    type Error = Reported;
}

impl<'i> QualifiedRuleParser<'i> for Descriptors<'_, '_> {
    type Prelude = ();
    type QualifiedRule = ();
    type Error = Reported;
}

impl<'i> RuleBodyItemParser<'i, (), Reported> for Descriptors<'_, '_> {
    fn parse_declarations(&self) -> bool {
        true
    }

    fn parse_qualified(&self) -> bool {
        false
    }
}
