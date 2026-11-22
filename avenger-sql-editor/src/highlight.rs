//! The highlighting of a query: the classes of avenger-sql-language's highlights, which the
//! analysis refines for the editor's text, or the syntax's alone for a text without an analysis,
//! such as the Lifted SQL tab, each with a class that the editor colors. A stylesheet takes the
//! same classes from cssparser's tokens.

use std::ops::Range;

use avenger_sql_language::{Highlight, HighlightRange, TextRange};
use avenger_sql_syntax::{FunctionRole, NameRole};
use cssparser::{ParseError, Parser, Token};

/// The class of a token, which sets its color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// Punctuation, and a name without a class of its own, such as an alias or a field.
    Plain,
    Keyword,
    /// A function of GoogleSQL or a transform.
    Function,
    /// A mark, a composition, or a constructor of avenger-chart.
    Chart,
    /// The name of a parameter, as before `=>`, or a property of a stylesheet.
    Argument,
    /// A column, or an attribute of a stylesheet's selector.
    Column,
    /// A table, or a class or an id of a stylesheet's selector.
    Table,
    /// A type, or an element of a stylesheet's selector.
    Type,
    /// A variable, a query parameter, a range variable, or a custom property of a stylesheet.
    Variable,
    Number,
    String,
    /// The format of files, as `'CSV'`, or a keyword of a stylesheet's value, as `bold`.
    Constant,
    /// A URI of files, as `'data/files/sales.csv'`, or the URL of a stylesheet's import.
    Path,
    Comment,
    /// An operator, such as `|>`, `=>`, or `+`.
    Operator,
}

/// A token of the text: its range in characters and its class.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub range: Range<usize>,
    pub class: Class,
}

/// The class of `highlight`.
fn class(highlight: Highlight) -> Class {
    match highlight {
        Highlight::Keyword => Class::Keyword,
        Highlight::Comment => Class::Comment,
        Highlight::String | Highlight::EnumValue => Class::String,
        Highlight::Format => Class::Constant,
        Highlight::Uri => Class::Path,
        Highlight::Number => Class::Number,
        Highlight::QueryParameter => Class::Variable,
        Highlight::Operator => Class::Operator,
        Highlight::Punctuation => Class::Plain,
        Highlight::Name(role) => match role {
            NameRole::Function(
                FunctionRole::Mark | FunctionRole::Composition | FunctionRole::Constructor,
            ) => Class::Chart,
            NameRole::Function(_) => Class::Function,
            NameRole::Parameter => Class::Argument,
            NameRole::Column => Class::Column,
            NameRole::Table | NameRole::Query | NameRole::Schema => Class::Table,
            NameRole::Type => Class::Type,
            NameRole::Variable | NameRole::RangeVariable => Class::Variable,
            _ => Class::Plain,
        },
    }
}

/// The highlights of the syntax of `text` alone.
pub fn syntax(text: &str) -> Vec<HighlightRange> {
    avenger_sql_syntax::highlight(&avenger_sql_syntax::parse(text).syntax())
}

/// The spans of `highlights` of `text`, in characters, in order.
pub fn spans(text: &str, highlights: &[HighlightRange]) -> Vec<Span> {
    let chars = Chars::new(text);
    (highlights.iter())
        .map(|highlight| Span {
            range: chars.range(highlight.range),
            class: class(highlight.highlight),
        })
        .collect()
}

/// The character index of each byte offset of a text, which converts the byte ranges of the
/// language crate to the character ranges of egui's cursor and back.
pub struct Chars<'t> {
    text: &'t str,
}

impl<'t> Chars<'t> {
    pub fn new(text: &'t str) -> Self {
        Self { text }
    }

    /// The character index of the byte offset `byte`, at most the length of the text.
    pub fn index(&self, byte: usize) -> usize {
        let byte = byte.min(self.text.len());
        self.text[..byte].chars().count()
    }

    /// The characters of the byte range `range`.
    pub fn range(&self, range: TextRange) -> Range<usize> {
        self.index(range.start().into())..self.index(range.end().into())
    }

    /// The byte offset of the character index `index`, at most the length of the text.
    pub fn offset(&self, index: usize) -> usize {
        (self.text.char_indices().nth(index)).map_or(self.text.len(), |(byte, _)| byte)
    }
}

/// The spans of the stylesheet `text`, in characters, in order, by cssparser's tokens and the
/// place of each token: comments, at-rules and `!important` as keywords, the elements of
/// selectors as types, their classes and ids as tables, their attributes as columns, and their
/// pseudo-classes as functions, properties as arguments, custom properties as variables, the
/// keywords of values as constants, strings, the URLs of imports as paths, numbers, dimensions,
/// percentages, and colors as numbers, functions such as `var()` and `env()`, and combinators
/// and the operators of `calc()` as operators. The rest, such as braces, colons, and
/// semicolons, is plain.
pub fn css(text: &str) -> Vec<Span> {
    let mut parser = Parser::new(text);
    let mut classes = Vec::new();
    let nodes = tokens(&mut parser, &mut classes);
    rules(&nodes, true, &mut classes);
    let chars = Chars::new(text);
    let mut spans: Vec<Span> = (classes.into_iter())
        .map(|(range, class)| Span {
            range: chars.index(range.start)..chars.index(range.end),
            class,
        })
        .collect();
    spans.sort_by_key(|span| span.range.start);
    spans
}

/// A token of a stylesheet other than a space or a comment, at a byte range, with the tokens of
/// its block when it opens one, such as `{` or `var(`.
struct Node<'i> {
    range: Range<usize>,
    token: Token<'i>,
    block: Vec<Node<'i>>,
}

/// The tokens of `parser` to the end of its block, with comments in `classes` and spaces left
/// out.
fn tokens<'i>(parser: &mut Parser<'i>, classes: &mut Vec<(Range<usize>, Class)>) -> Vec<Node<'i>> {
    let mut nodes = Vec::new();
    loop {
        let start = parser.position().byte_index();
        let Ok(token) = parser.next_including_whitespace_and_comments() else {
            return nodes;
        };
        let token = token.clone();
        let range = start..parser.position().byte_index();
        match token {
            Token::WhiteSpace(_) => continue,
            Token::Comment(_) => {
                classes.push((range, Class::Comment));
                continue;
            }
            _ => {}
        }
        let opens = matches!(
            token,
            Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock
        );
        let mut block = Vec::new();
        if opens {
            let _ = parser.parse_nested_block(|parser| {
                block = tokens(parser, classes);
                Ok::<(), ParseError<()>>(())
            });
        }
        nodes.push(Node {
            range,
            token,
            block,
        });
    }
}

/// The classes of the statements of a stylesheet or of a block: rules, at-rules, and
/// declarations, which end at a `;` or at the block of a rule. A statement with a block is a rule
/// whose prelude is a selector, and a statement without one is a declaration, or in a stylesheet
/// (`top`), a rule whose block is not written yet.
fn rules(nodes: &[Node], top: bool, classes: &mut Vec<(Range<usize>, Class)>) {
    let mut start = 0;
    for (index, node) in nodes.iter().enumerate() {
        let block = match node.token {
            Token::Semicolon => None,
            Token::CurlyBracketBlock => Some(node),
            _ => continue,
        };
        statement(&nodes[start..index], block, top, classes);
        start = index + 1;
    }
    statement(&nodes[start..], None, top, classes);
}

fn statement(
    prelude: &[Node],
    block: Option<&Node>,
    top: bool,
    classes: &mut Vec<(Range<usize>, Class)>,
) {
    match (prelude.first().map(|node| &node.token), block) {
        (Some(Token::AtKeyword(name)), _) => {
            classes.push((prelude[0].range.clone(), Class::Keyword));
            let import = name.eq_ignore_ascii_case("import");
            for node in &prelude[1..] {
                match &node.token {
                    Token::ParenthesisBlock => condition(&node.block, classes),
                    Token::QuotedString(_) | Token::UnquotedUrl(_) if import => {
                        classes.push((node.range.clone(), Class::Path))
                    }
                    _ => value(std::slice::from_ref(node), import, classes),
                }
            }
        }
        (_, Some(_)) => selector(prelude, classes),
        (_, None) if top => selector(prelude, classes),
        (_, None) => declaration(prelude, classes),
    }
    if let Some(block) = block {
        rules(&block.block, false, classes);
    }
}

/// The classes of a condition of `@media` or `@container` in parentheses, such as
/// `(prefers-color-scheme: dark)`, whose feature reads as a property.
fn condition(nodes: &[Node], classes: &mut Vec<(Range<usize>, Class)>) {
    match nodes.iter().any(|node| node.token == Token::Colon) {
        true => declaration(nodes, classes),
        false => value(nodes, false, classes),
    }
}

/// The classes of a selector: elements, classes, ids, attributes, pseudo-classes, and
/// combinators.
fn selector(nodes: &[Node], classes: &mut Vec<(Range<usize>, Class)>) {
    let mut previous: Option<&Token> = None;
    for node in nodes {
        let class = match &node.token {
            Token::Ident(_) => match previous {
                Some(Token::Delim('.')) => Some(Class::Table),
                Some(Token::Colon) => Some(Class::Function),
                _ => Some(Class::Type),
            },
            Token::Delim('.') | Token::IDHash(_) | Token::Hash(_) => Some(Class::Table),
            Token::Colon => Some(Class::Function),
            Token::Delim('>' | '+' | '~' | '*' | '&') => Some(Class::Operator),
            Token::Function(_) => {
                selector(&node.block, classes);
                Some(Class::Function)
            }
            Token::SquareBracketBlock => {
                attribute(&node.block, classes);
                None
            }
            Token::Number { .. } | Token::Dimension { .. } => Some(Class::Number),
            Token::QuotedString(_) => Some(Class::String),
            Token::CurlyBracketBlock => {
                rules(&node.block, false, classes);
                None
            }
            _ => None,
        };
        if let Some(class) = class {
            classes.push((node.range.clone(), class));
        }
        previous = Some(&node.token);
    }
}

/// The classes of an attribute selector in brackets, such as `[type=bar i]`: its name, its
/// operator, and its value and flag.
fn attribute(nodes: &[Node], classes: &mut Vec<(Range<usize>, Class)>) {
    for (index, node) in nodes.iter().enumerate() {
        let class = match &node.token {
            Token::Ident(_) if index == 0 => Class::Column,
            Token::Ident(_) | Token::QuotedString(_) => Class::String,
            Token::Delim(_)
            | Token::IncludeMatch
            | Token::DashMatch
            | Token::PrefixMatch
            | Token::SuffixMatch
            | Token::SubstringMatch => Class::Operator,
            _ => continue,
        };
        classes.push((node.range.clone(), class));
    }
}

/// The classes of a declaration: its property, before the first `:`, and its value.
fn declaration(nodes: &[Node], classes: &mut Vec<(Range<usize>, Class)>) {
    let colon = (nodes.iter())
        .position(|node| node.token == Token::Colon)
        .unwrap_or(nodes.len());
    for node in &nodes[..colon] {
        if let Token::Ident(name) = &node.token {
            let class = match name.starts_with("--") {
                true => Class::Variable,
                false => Class::Argument,
            };
            classes.push((node.range.clone(), class));
        }
    }
    value(nodes.get(colon + 1..).unwrap_or_default(), false, classes);
}

/// The classes of a value, whose strings are paths in a `url()` or an import (`url`).
fn value(nodes: &[Node], url: bool, classes: &mut Vec<(Range<usize>, Class)>) {
    let mut previous: Option<&Token> = None;
    for node in nodes {
        let class = match &node.token {
            Token::Ident(name) if name.starts_with("--") => Some(Class::Variable),
            Token::Ident(name)
                if previous == Some(&Token::Delim('!'))
                    && name.eq_ignore_ascii_case("important") =>
            {
                Some(Class::Keyword)
            }
            Token::Ident(_) => Some(Class::Constant),
            Token::Delim('!') => Some(Class::Keyword),
            Token::Delim('+' | '-' | '*' | '/') => Some(Class::Operator),
            Token::Number { .. }
            | Token::Percentage { .. }
            | Token::Dimension { .. }
            | Token::Hash(_)
            | Token::IDHash(_) => Some(Class::Number),
            Token::QuotedString(_) | Token::BadString(_) => Some(match url {
                true => Class::Path,
                false => Class::String,
            }),
            Token::UnquotedUrl(_) | Token::BadUrl(_) => Some(Class::Path),
            Token::Function(name) => {
                value(
                    &node.block,
                    url || name.eq_ignore_ascii_case("url"),
                    classes,
                );
                Some(Class::Function)
            }
            Token::ParenthesisBlock | Token::SquareBracketBlock => {
                value(&node.block, url, classes);
                None
            }
            Token::CurlyBracketBlock => {
                rules(&node.block, false, classes);
                None
            }
            _ => None,
        };
        if let Some(class) = class {
            classes.push((node.range.clone(), class));
        }
        previous = Some(&node.token);
    }
}
