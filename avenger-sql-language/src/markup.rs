//! The Typst markup in strings: the string literals that the analysis marks as markup, as
//! `Model::markup` gives them, parsed with typst-syntax, the parser that renders a chart's text,
//! in Typst's markup mode, as `avenger-typst-label` parses a label. A string's Typst classes
//! split its highlight, and its Typst syntax errors are diagnostics at their places in the file.
//!
//! The offsets of a Typst parse are in the string's value, after its escapes, so they map back to
//! the text through the range of each byte of the value, which [`ast::Literal::decoded_string`]
//! gives. A literal whose escapes are wrong, or whose value isn't UTF-8, holds no markup.

use avenger_sql_syntax::{
    ast::{self, AstNode, DecodedString},
    Highlight, HighlightRange, TypstTag,
};
use typst_syntax::{LinkedNode, SyntaxKind as TypstKind, Tag};

use crate::{Diagnostic, Document, Severity, Source, TextRange};

/// A string literal of Typst markup: its value, with the range of each byte in the text, and its
/// Typst syntax tree.
struct Markup {
    /// The range of the literal.
    range: TextRange,
    decoded: DecodedString,
    root: typst_syntax::SyntaxNode,
}

impl Document {
    /// The string literals of Typst markup, parsed.
    fn markups(&self) -> Vec<Markup> {
        let root = self.root();
        (self.model().markup.iter())
            .filter_map(|&range| {
                let element = root.covering_element(range);
                let node = match element.as_node() {
                    Some(node) => node.clone(),
                    None => element.as_token()?.parent()?,
                };
                let literal = (node.ancestors())
                    .filter_map(ast::Literal::cast)
                    .find(|literal| literal.syntax().text_range() == range)?;
                let decoded = literal.decoded_string()?;
                let root = typst_syntax::parse(&decoded.text);
                Some(Markup {
                    range,
                    decoded,
                    root,
                })
            })
            .collect()
    }

    /// `ranges`, which [`avenger_sql_syntax::highlight`] gives and the model refines, with the
    /// string ranges of Typst markup split: each range that Typst gives a class takes it, and
    /// the quotes and the text between those ranges stay strings. The ranges stay in the order of
    /// the text, without overlaps.
    pub(crate) fn split_markup(&self, ranges: Vec<HighlightRange>) -> Vec<HighlightRange> {
        let mut typst: Vec<(TextRange, TypstTag)> = (self.markups().iter())
            .flat_map(|markup| markup.highlights())
            .collect();
        if typst.is_empty() {
            return ranges;
        }
        typst.sort_by_key(|(range, _)| range.start());
        let mut split = Vec::with_capacity(ranges.len() + typst.len() * 2);
        let mut next = 0;
        for range in ranges {
            if range.highlight != Highlight::String {
                split.push(range);
                continue;
            }
            while next < typst.len() && typst[next].0.start() < range.range.start() {
                next += 1;
            }
            let mut at = range.range.start();
            while next < typst.len() && range.range.contains_range(typst[next].0) {
                let (inner, tag) = typst[next];
                if at < inner.start() {
                    split.push(HighlightRange {
                        range: TextRange::new(at, inner.start()),
                        ..range
                    });
                }
                split.push(HighlightRange {
                    range: inner,
                    highlight: Highlight::Typst(tag),
                    definition: false,
                });
                at = inner.end();
                next += 1;
            }
            if at < range.range.end() {
                split.push(HighlightRange {
                    range: TextRange::new(at, range.range.end()),
                    ..range
                });
            }
        }
        split
    }

    /// The Typst syntax errors of the strings of markup, which fail a chart's render.
    pub(crate) fn markup_diagnostics(&self) -> Vec<Diagnostic> {
        (self.markups().iter())
            .flat_map(|markup| markup.diagnostics())
            .collect()
    }
}

impl Markup {
    /// The ranges in the text of the leaves of the Typst tree that Typst's highlighter classes,
    /// each by the class of the leaf or of its nearest classed ancestor, with the leaves of one
    /// class next to each other joined, in the order of the text.
    fn highlights(&self) -> Vec<(TextRange, TypstTag)> {
        let mut classed: Vec<(std::ops::Range<usize>, TypstTag)> = Vec::new();
        let mut leaves = vec![LinkedNode::new(&self.root)];
        let mut ordered = Vec::new();
        // Depth first, in the order of the text.
        while let Some(node) = leaves.pop() {
            match node.get().children().len() {
                0 => ordered.push(node),
                _ => leaves.extend(node.children().rev()),
            }
        }
        for leaf in ordered {
            let range = leaf.range();
            if range.is_empty() {
                continue;
            }
            let tag = std::iter::successors(Some(&leaf), |node| node.parent())
                .find_map(|node| typst_syntax::highlight(node));
            let Some(tag) = tag.map(tag_of) else {
                continue;
            };
            match classed.last_mut() {
                Some((last, class)) if *class == tag && last.end == range.start => {
                    last.end = range.end;
                }
                _ => classed.push((range, tag)),
            }
        }
        (classed.into_iter())
            .flat_map(|(range, tag)| {
                (self.decoded.text_ranges(range).into_iter()).map(move |range| (range, tag))
            })
            .collect()
    }

    /// A diagnostic of each error of the Typst tree, at its range in the text, or at the
    /// character after it, when it is empty, such as the end of an unclosed `$`.
    fn diagnostics(&self) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        let mut nodes = vec![LinkedNode::new(&self.root)];
        while let Some(node) = nodes.pop() {
            nodes.extend(node.children());
            if node.kind() != TypstKind::Error {
                continue;
            }
            let (errors, _) = node.get().errors_and_warnings();
            let Some(error) = errors.into_iter().next() else {
                continue;
            };
            let help = (error.hints.iter())
                .map(|hint| hint.v.to_string())
                .reduce(|hints, hint| format!("{hints}; {hint}"));
            diagnostics.push(Diagnostic {
                range: self.range_of(node.range()),
                message: format!("{} in Typst markup", error.message),
                help,
                severity: Severity::Error,
                source: Source::Typst,
                code: None,
                fixes: Vec::new(),
            });
        }
        diagnostics.sort_by_key(|diagnostic| diagnostic.range.start());
        diagnostics
    }

    /// The range in the text of the bytes `range` of the value, from the start of its first part
    /// to the end of its last. An empty range takes the character after it, or before it at the
    /// end, and an empty value the whole literal.
    fn range_of(&self, range: std::ops::Range<usize>) -> TextRange {
        let text = &self.decoded.text;
        let range = match range.is_empty() {
            false => range,
            true => match text[range.start..].chars().next() {
                Some(c) => range.start..range.start + c.len_utf8(),
                None => match text[..range.start].chars().next_back() {
                    Some(c) => range.start - c.len_utf8()..range.start,
                    None => return self.range,
                },
            },
        };
        let parts = self.decoded.text_ranges(range);
        match (parts.first(), parts.last()) {
            (Some(first), Some(last)) => TextRange::new(first.start(), last.end()),
            _ => self.range,
        }
    }
}

/// The class of Typst's tag `tag`.
fn tag_of(tag: Tag) -> TypstTag {
    match tag {
        Tag::Comment => TypstTag::Comment,
        Tag::Punctuation => TypstTag::Punctuation,
        Tag::Escape => TypstTag::Escape,
        Tag::Strong => TypstTag::Strong,
        Tag::Emph => TypstTag::Emph,
        Tag::Link => TypstTag::Link,
        Tag::Raw => TypstTag::Raw,
        Tag::Label => TypstTag::Label,
        Tag::Ref => TypstTag::Ref,
        Tag::Heading => TypstTag::Heading,
        Tag::ListMarker => TypstTag::ListMarker,
        Tag::ListTerm => TypstTag::ListTerm,
        Tag::MathDelimiter => TypstTag::MathDelimiter,
        Tag::MathOperator => TypstTag::MathOperator,
        Tag::MathGroupingParens => TypstTag::MathGroupingParens,
        Tag::Keyword => TypstTag::Keyword,
        Tag::Operator => TypstTag::Operator,
        Tag::Number => TypstTag::Number,
        Tag::String => TypstTag::String,
        Tag::Function => TypstTag::Function,
        Tag::Interpolated => TypstTag::Interpolated,
        Tag::Error => TypstTag::Error,
    }
}
