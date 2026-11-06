//! Selectors: the `selectors` crate's parser with the hooks of the style language, and the check
//! of a parsed selector against the vocabulary of the style tree.

use std::{borrow::Borrow, fmt};

use cssparser::{serialize_identifier, ParseError, Parser as CssParser, ToCss, Token};
use precomputed_hash::PrecomputedHash;
use selectors::parser::{
    Component, NonTSPseudoClass, ParseRelative, Parser, Selector, SelectorImpl, SelectorList,
    SelectorParseErrorKind,
};

use crate::{
    source::Problem,
    vocab::{is_attribute, is_mark_name, Element},
};

/// The `selectors` implementation of the style language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selectors;

/// A name in a selector: an element, an attribute, a class, an id, or an attribute value.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Name(pub String);

impl From<&str> for Name {
    fn from(name: &str) -> Self {
        Name(name.to_string())
    }
}

impl Borrow<str> for Name {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for Name {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl ToCss for Name {
    fn to_css<W: fmt::Write>(&self, dest: &mut W) -> fmt::Result {
        cssparser::serialize_identifier(&self.0, dest)
    }
}

impl PrecomputedHash for Name {
    fn precomputed_hash(&self) -> u32 {
        // FNV-1a.
        let mut hash: u32 = 0x811c_9dc5;
        for byte in self.0.bytes() {
            hash ^= u32::from(byte);
            hash = hash.wrapping_mul(0x0100_0193);
        }
        hash
    }
}

/// The only interaction pseudo-class of the style language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PseudoClass {
    FocusVisible,
}

impl ToCss for PseudoClass {
    fn to_css<W: fmt::Write>(&self, dest: &mut W) -> fmt::Result {
        dest.write_str(":focus-visible")
    }
}

impl NonTSPseudoClass for PseudoClass {
    fn is_active_or_hover(&self) -> bool {
        false
    }

    fn is_user_action_state(&self) -> bool {
        true
    }
}

/// The style language has no pseudo-elements.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PseudoElement {}

impl ToCss for PseudoElement {
    fn to_css<W: fmt::Write>(&self, _dest: &mut W) -> fmt::Result {
        match *self {}
    }
}

impl selectors::parser::PseudoElement for PseudoElement {}

impl SelectorImpl for Selectors {
    type ExtraMatchingData<'a> = ();
    type AttrValue = Name;
    type Identifier = Name;
    type LocalName = Name;
    type NamespaceUrl = Name;
    type NamespacePrefix = Name;
    type BorrowedNamespaceUrl = str;
    type BorrowedLocalName = str;
    type NonTSPseudoClass = PseudoClass;
    type PseudoElement = PseudoElement;
}

/// Why a selector does not parse.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SelectorError {
    Parse(SelectorParseErrorKind),
    /// A pseudo-class other than `:focus-visible`, by its name.
    PseudoClass(String),
    /// A pseudo-element, by its name.
    PseudoElement(String),
}

impl From<SelectorParseErrorKind> for SelectorError {
    fn from(kind: SelectorParseErrorKind) -> Self {
        SelectorError::Parse(kind)
    }
}

/// The parser hooks: `:has()`, `:is()`, `:where()`, `of S`, and `&`, no forgiving selector lists,
/// and `:focus-visible` as the only pseudo-class.
struct Hooks;

impl<'i> Parser<'i> for Hooks {
    type Impl = Selectors;
    type Error = SelectorError;

    fn parse_nth_child_of(&self) -> bool {
        true
    }

    fn parse_is_and_where(&self) -> bool {
        true
    }

    fn parse_has(&self) -> bool {
        true
    }

    fn parse_parent_selector(&self) -> bool {
        true
    }

    fn allow_forgiving_selectors(&self) -> bool {
        false
    }

    fn parse_non_ts_pseudo_class(
        &self,
        name: &str,
    ) -> Result<PseudoClass, ParseError<Self::Error>> {
        match name.eq_ignore_ascii_case("focus-visible") {
            true => Ok(PseudoClass::FocusVisible),
            false => Err(ParseError::custom(SelectorError::PseudoClass(
                name.to_string(),
            ))),
        }
    }

    fn parse_non_ts_functional_pseudo_class(
        &self,
        name: &str,
        _parser: &mut CssParser<'i>,
        _after_part: bool,
    ) -> Result<PseudoClass, ParseError<Self::Error>> {
        Err(ParseError::custom(SelectorError::PseudoClass(
            name.to_string(),
        )))
    }

    fn parse_pseudo_element(&self, name: &str) -> Result<PseudoElement, ParseError<Self::Error>> {
        Err(ParseError::custom(SelectorError::PseudoElement(
            name.to_string(),
        )))
    }

    fn parse_functional_pseudo_element(
        &self,
        name: &str,
        _arguments: &mut CssParser<'i>,
    ) -> Result<PseudoElement, ParseError<Self::Error>> {
        Err(ParseError::custom(SelectorError::PseudoElement(
            name.to_string(),
        )))
    }
}

/// Parses a selector list, relative to an enclosing rule when `nested`.
pub(crate) fn parse_list(
    input: &mut CssParser<'_>,
    nested: bool,
) -> Result<SelectorList<Selectors>, ParseError<SelectorError>> {
    let relative = match nested {
        true => ParseRelative::ForNesting,
        false => ParseRelative::No,
    };
    SelectorList::parse(&Hooks, input, relative)
}

/// The message and the help of a selector that does not parse.
pub(crate) fn parse_error_message(error: &SelectorError) -> (String, Option<String>) {
    match error {
        SelectorError::PseudoClass(name) => pseudo_class_message(name),
        SelectorError::PseudoElement(_) => (
            "pseudo-elements are not supported; each part of a chart is a node".to_string(),
            None,
        ),
        SelectorError::Parse(kind) => (kind_message(kind).to_string(), None),
    }
}

fn kind_message(kind: &SelectorParseErrorKind) -> &'static str {
    use SelectorParseErrorKind::*;
    match kind {
        EmptySelector => "a selector is empty or starts with a token that no selector starts with",
        DanglingCombinator => "a combinator needs a selector after it",
        NonCompoundSelector => "this pseudo-class takes compound selectors, without combinators",
        NoQualifiedNameInAttributeSelector | InvalidQualNameInAttr | ExpectedBarInAttr => {
            "an attribute selector needs an attribute name, as in `[channel=x]`"
        }
        UnexpectedTokenInAttributeSelector | BadValueInAttr => {
            "an attribute selector compares its attribute with `=`, `~=`, `|=`, `^=`, `$=`, or `*=` \
             to a name or a string"
        }
        ExpectedNamespace | ExplicitNamespaceUnexpectedToken => "namespaces are not supported",
        PseudoElementExpectedColon
        | PseudoElementExpectedIdent
        | NonPseudoElementAfterSlotted
        | InvalidPseudoElementAfterSlotted
        | InvalidPseudoElementInsideWhere => {
            "pseudo-elements are not supported; each part of a chart is a node"
        }
        NoIdentForPseudo => "a pseudo-class needs a name after `:`",
        UnsupportedPseudoClassOrElement | InvalidState => "the pseudo-class is not supported",
        UnexpectedIdent => "a name stands where a selector cannot hold one",
        ClassNeedsIdent => {
            "a class selector needs a CSS identifier after `.`, with CSS escapes for a class that \
             is not one"
        }
    }
}

/// The message and the help of the first `#` in the selector text `text`, up to its block, that
/// is not an id selector because the name after it is not a CSS identifier, as in `#1st`.
pub(crate) fn non_identifier_id(text: &str) -> Option<(String, Option<String>)> {
    let mut parser = CssParser::new(text);
    loop {
        match parser.next() {
            Ok(Token::Hash(name)) => {
                let mut escaped = String::new();
                serialize_identifier(name, &mut escaped).ok()?;
                return Some((
                    format!(
                        "`#{name}` is not an id selector, because `{name}` is not a CSS identifier"
                    ),
                    Some(format!("escape it, as in `#{escaped}`")),
                ));
            }
            Ok(Token::CurlyBracketBlock | Token::Semicolon) | Err(_) => return None,
            Ok(_) => {}
        }
    }
}

fn pseudo_class_message(name: &str) -> (String, Option<String>) {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "hover" | "active" => (
            format!(
                "`:{name}` is not supported; the only interaction pseudo-class is :focus-visible"
            ),
            Some("style the marks of a selection with a selection condition".to_string()),
        ),
        "focus" | "focus-within" => (
            format!("`:{name}` is not supported"),
            Some("use :focus-visible".to_string()),
        ),
        _ => (format!("the pseudo-class `:{name}` is not supported"), None),
    }
}

/// Checks each selector of `list` against the vocabulary of the style tree: element names,
/// attribute names, the structural pseudo-classes that the style tree supports, and the rules of
/// `:focus-visible` and of virtual nodes. A compound selector is checked once for each
/// alternative of its `:is()` and `:where()` arguments, with their requirements, so
/// `:is(title):focus-visible` is checked as `title:focus-visible`. Returns whether the selectors
/// match only nodes with keyboard focus, or the first problem. Either every selector of the list,
/// and every alternative of each, ends with `:focus-visible`, or none does.
pub(crate) fn check_list(list: &SelectorList<Selectors>) -> Result<bool, Problem> {
    let mut focus = None;
    for selector in list.slice() {
        for alternative in check_selector(selector, Context::Top, true)? {
            match focus {
                None => focus = Some(alternative.focus_visible),
                Some(focus) if focus != alternative.focus_visible => {
                    return Err(Problem::new(
                        "the selectors of a rule all end with :focus-visible, or none does",
                    )
                    .with_help("write the selectors with :focus-visible as a rule of their own"));
                }
                Some(_) => {}
            }
        }
    }
    Ok(focus.unwrap_or(false))
}

/// Where a selector stands, which decides where `:focus-visible` is valid.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Context {
    /// A selector of a rule, or an argument of `:is()` or `:where()` in its last compound.
    Top,
    /// An argument of a pseudo-class in an earlier compound, of `:not()`, or of `:has()`, or a
    /// selector of `of S`.
    Inner,
}

/// What one alternative of a compound selector requires of the nodes that it matches.
#[derive(Clone, Default)]
struct Facts {
    element: Option<Element>,
    focus_visible: bool,
    attributes: Vec<String>,
    /// Whether it has a class, an id, or a pseudo-class other than `:focus-visible`.
    other: bool,
}

impl Facts {
    /// The requirements of a node that both match.
    fn and(&self, other: &Facts) -> Facts {
        Facts {
            element: self.element.or(other.element),
            focus_visible: self.focus_visible || other.focus_visible,
            attributes: (self.attributes.iter())
                .chain(&other.attributes)
                .cloned()
                .collect(),
            other: self.other || other.other,
        }
    }
}

/// Checks a selector and returns the alternatives of its last compound. `check_last` checks them
/// too, and an argument of `:is()` or `:where()` leaves them to the compound that holds it.
fn check_selector(
    selector: &Selector<Selectors>,
    context: Context,
    check_last: bool,
) -> Result<Vec<Facts>, Problem> {
    // Parse order: the first compound first, with a combinator between compounds.
    let components: Vec<&Component<Selectors>> = selector.iter_raw_parse_order_from(0).collect();
    let compounds: Vec<&[&Component<Selectors>]> = components
        .split(|component| matches!(component, Component::Combinator(_)))
        .collect();
    let last = compounds.len().saturating_sub(1);
    let mut alternatives = Vec::new();
    for (index, compound) in compounds.iter().enumerate() {
        let inner = match (context, index == last) {
            (Context::Top, true) => Context::Top,
            _ => Context::Inner,
        };
        alternatives = compound_facts(compound, inner)?;
        if index != last || check_last {
            for facts in &alternatives {
                check_facts(facts)?;
            }
        }
    }
    Ok(alternatives)
}

/// The alternatives of a compound selector: its own requirements with those of each combination
/// of the selectors of its `:is()` and `:where()` arguments.
fn compound_facts(
    components: &[&Component<Selectors>],
    context: Context,
) -> Result<Vec<Facts>, Problem> {
    let mut own = Facts::default();
    let mut arguments: Vec<Vec<Facts>> = Vec::new();
    for component in components {
        match component {
            Component::LocalName(name) => {
                let element =
                    Element::from_name(&name.name.0).ok_or_else(|| unknown(&name.name.0))?;
                own.element = Some(element);
            }
            Component::ExplicitUniversalType => {}
            Component::ID(_) | Component::Class(_) => own.other = true,
            Component::AttributeInNoNamespaceExists { local_name, .. }
            | Component::AttributeInNoNamespace { local_name, .. } => {
                own.attributes.push(local_name.0.clone());
            }
            Component::AttributeOther(attribute) => {
                own.attributes.push(attribute.local_name.0.clone());
            }
            Component::NonTSPseudoClass(PseudoClass::FocusVisible) => {
                if context == Context::Inner {
                    return Err(Problem::new(
                        ":focus-visible is valid only in the last compound selector, outside :not() and :has()",
                    ));
                }
                own.focus_visible = true;
            }
            Component::Root | Component::Nth(_) => own.other = true,
            Component::NthOf(nth) => {
                own.other = true;
                for selector in nth.selectors() {
                    check_selector(selector, Context::Inner, true)?;
                }
            }
            Component::Negation(list) => {
                own.other = true;
                for selector in list.slice() {
                    check_selector(selector, Context::Inner, true)?;
                }
            }
            Component::Is(list) | Component::Where(list) => {
                let mut alternatives = Vec::new();
                for selector in list.slice() {
                    alternatives.extend(check_selector(selector, context, false)?);
                }
                arguments.push(alternatives);
            }
            Component::Has(relatives) => {
                own.other = true;
                for relative in relatives.iter() {
                    check_selector(&relative.selector, Context::Inner, true)?;
                }
            }
            Component::ParentSelector | Component::RelativeSelectorAnchor => {}
            Component::Empty => {
                return Err(Problem::new("the pseudo-class `:empty` is not supported"))
            }
            Component::Scope | Component::ImplicitScope => {}
            _ => return Err(Problem::new("the selector is not supported")),
        }
    }
    let mut alternatives = vec![own];
    for argument in arguments {
        alternatives = (alternatives.iter())
            .flat_map(|facts| argument.iter().map(|alternative| facts.and(alternative)))
            .collect();
    }
    Ok(alternatives)
}

/// Checks one alternative of a compound selector against the vocabulary of the style tree.
fn check_facts(facts: &Facts) -> Result<(), Problem> {
    let Some(element) = facts.element else {
        if let Some(attribute) = facts.attributes.iter().find(|name| !is_attribute(name)) {
            return Err(Problem::new(format!(
                "no element has the attribute `{attribute}`"
            )));
        }
        return Ok(());
    };
    for attribute in &facts.attributes {
        if element.is_virtual() {
            return Err(Problem::new(format!(
                "the virtual node `{element}` has no attributes"
            )));
        }
        if !element.has_attribute(attribute) {
            return Err(Problem::new(format!(
                "the element `{element}` has no attribute `{attribute}`"
            )));
        }
    }
    if element.is_virtual() {
        if !facts.focus_visible {
            return Err(Problem::new(format!(
                "`{element}` is a virtual node; select it with {element}:focus-visible"
            )));
        }
        if facts.other {
            return Err(Problem::new(format!(
                "the virtual node `{element}` takes no selector other than :focus-visible"
            )));
        }
    }
    if facts.focus_visible && !element.can_focus() {
        return Err(Problem::new(format!(
            "`{element}` cannot have focus; :focus-visible is valid on chart, axis, legend, mark, entry, group, and item"
        )));
    }
    Ok(())
}

fn unknown(name: &str) -> Problem {
    let problem = Problem::new(format!("`{name}` is not an element of the style tree"));
    match is_mark_name(name) {
        true => problem.with_help(format!("select the marks with mark[type={name}]")),
        false => problem,
    }
}
