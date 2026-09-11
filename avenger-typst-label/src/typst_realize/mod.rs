//! Content realization for labels.
//!
//! This mirrors the static realization step between upstream `typst-eval` and
//! `typst-layout`: flatten retained text/model/symbol markup and parameter
//! values into renderable single-line text and math nodes.

use crate::label::LabelError;
use crate::typst_library::foundations::{Scope, Value};
use crate::typst_library::text::content::{
    LabelContent, LineNode, MathSpan, PlainTextNode, TextMarkupKind, TextMarkupOptions,
};
use crate::typst_library::text::smartquote::{SmartQuoter, is_default_ignorable};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RenderLine {
    pub(crate) source: String,
    pub(crate) nodes: Vec<RenderNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RenderNode {
    Plain(PlainTextNode),
    DecoratedText(DecoratedText),
    Math(MathSpan),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DecoratedText {
    pub(crate) kind: TextMarkupKind,
    pub(crate) options: TextMarkupOptions,
    pub(crate) nested: Vec<TextMarkupRun>,
    pub(crate) text: String,
    pub(crate) byte_range: std::ops::Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TextMarkupRun {
    pub(crate) kind: TextMarkupKind,
    pub(crate) options: TextMarkupOptions,
}

pub(crate) fn realize_static_markup_line(
    line: &LabelContent,
    params: &Scope,
) -> Result<Option<RenderLine>, LabelError> {
    let mut nodes: Vec<RenderNode> = Vec::new();
    let mut pending_plain = String::new();
    let mut pending_start = None;
    let mut pending_end = 0usize;
    let mut quote_context = QuoteContext::default();

    let flush_plain = |nodes: &mut Vec<RenderNode>,
                       pending_plain: &mut String,
                       pending_start: &mut Option<usize>,
                       pending_end: usize| {
        if let Some(start) = pending_start.take()
            && !pending_plain.is_empty()
        {
            nodes.push(RenderNode::Plain(PlainTextNode {
                text: std::mem::take(pending_plain),
                byte_range: start..pending_end,
            }));
        }
    };

    for node in &line.nodes {
        match node {
            LineNode::Plain(plain) => {
                push_pending_text(
                    &mut pending_plain,
                    &mut pending_start,
                    &mut pending_end,
                    &mut quote_context,
                    &plain.text,
                    plain.byte_range.clone(),
                );
            }
            LineNode::Emoji(alias) => {
                push_pending_text(
                    &mut pending_plain,
                    &mut pending_start,
                    &mut pending_end,
                    &mut quote_context,
                    alias.emoji,
                    alias.byte_range.clone(),
                );
            }
            LineNode::Symbol(alias) => {
                push_pending_text(
                    &mut pending_plain,
                    &mut pending_start,
                    &mut pending_end,
                    &mut quote_context,
                    alias.text,
                    alias.byte_range.clone(),
                );
            }
            LineNode::Param(param) => {
                let text = render_scope_param(param.name.as_str(), params, param.byte_range.start)?;
                push_pending_text(
                    &mut pending_plain,
                    &mut pending_start,
                    &mut pending_end,
                    &mut quote_context,
                    &text,
                    param.byte_range.clone(),
                );
            }
            LineNode::SmartQuote(quote) => {
                let text = quote_context.quote(quote.quote.double);
                push_pending_text(
                    &mut pending_plain,
                    &mut pending_start,
                    &mut pending_end,
                    &mut quote_context,
                    text,
                    quote.byte_range.clone(),
                );
            }
            LineNode::Math(math) => {
                flush_plain(
                    &mut nodes,
                    &mut pending_plain,
                    &mut pending_start,
                    pending_end,
                );
                quote_context.push_text("\u{FFFC}");
                nodes.push(RenderNode::Math(math.clone()));
            }
            LineNode::TextSpan(span) => {
                let Some(kind) = supported_static_markup_kind(span.kind) else {
                    return Ok(None);
                };
                let Some(body) = render_static_body(&span.body, params, &mut quote_context)? else {
                    return Ok(None);
                };
                let text = transform_static_text(kind, &body.text);
                flush_plain(
                    &mut nodes,
                    &mut pending_plain,
                    &mut pending_start,
                    pending_end,
                );
                if !text.is_empty() {
                    nodes.push(RenderNode::DecoratedText(DecoratedText {
                        kind,
                        options: span.options.clone(),
                        nested: body.nested,
                        text,
                        byte_range: span.body_range.clone(),
                    }));
                }
            }
        }
    }

    flush_plain(
        &mut nodes,
        &mut pending_plain,
        &mut pending_start,
        pending_end,
    );

    Ok(Some(RenderLine {
        source: line.source.clone(),
        nodes,
    }))
}

#[derive(Default)]
struct QuoteContext {
    quoter: SmartQuoter,
    text: String,
}

impl QuoteContext {
    fn quote(&mut self, double: bool) -> &'static str {
        let before = self.text.chars().rev().find(|&c| !is_default_ignorable(c));
        self.quoter.quote(before, double)
    }

    fn push_text(&mut self, text: &str) {
        self.text.push_str(text);
    }
}

fn push_pending_text(
    pending_plain: &mut String,
    pending_start: &mut Option<usize>,
    pending_end: &mut usize,
    quote_context: &mut QuoteContext,
    text: &str,
    byte_range: std::ops::Range<usize>,
) {
    if pending_start.is_none() {
        *pending_start = Some(byte_range.start);
    }
    *pending_end = byte_range.end;
    pending_plain.push_str(text);
    quote_context.push_text(text);
}

fn supported_static_markup_kind(kind: TextMarkupKind) -> Option<TextMarkupKind> {
    match kind {
        TextMarkupKind::Underline
        | TextMarkupKind::Strike
        | TextMarkupKind::Overline
        | TextMarkupKind::Subscript
        | TextMarkupKind::Superscript
        | TextMarkupKind::Lower
        | TextMarkupKind::Upper
        | TextMarkupKind::Smallcaps
        | TextMarkupKind::Emph
        | TextMarkupKind::Strong
        | TextMarkupKind::Raw => Some(kind),
    }
}

fn transform_static_text(kind: TextMarkupKind, text: &str) -> String {
    match kind {
        TextMarkupKind::Lower => text.to_lowercase(),
        TextMarkupKind::Upper => text.to_uppercase(),
        TextMarkupKind::Underline
        | TextMarkupKind::Strike
        | TextMarkupKind::Overline
        | TextMarkupKind::Subscript
        | TextMarkupKind::Superscript
        | TextMarkupKind::Smallcaps
        | TextMarkupKind::Emph
        | TextMarkupKind::Strong
        | TextMarkupKind::Raw => text.to_string(),
    }
}

struct RealizedStaticBody {
    text: String,
    nested: Vec<TextMarkupRun>,
}

fn render_static_body(
    nodes: &[LineNode],
    params: &Scope,
    quote_context: &mut QuoteContext,
) -> Result<Option<RealizedStaticBody>, LabelError> {
    let mut text = String::new();
    let mut nested = Vec::new();
    for node in nodes {
        match node {
            LineNode::Plain(plain) => {
                text.push_str(&plain.text);
                quote_context.push_text(&plain.text);
            }
            LineNode::Emoji(alias) => {
                text.push_str(alias.emoji);
                quote_context.push_text(alias.emoji);
            }
            LineNode::Symbol(alias) => {
                text.push_str(alias.text);
                quote_context.push_text(alias.text);
            }
            LineNode::Param(param) => {
                let rendered =
                    render_scope_param(param.name.as_str(), params, param.byte_range.start)?;
                text.push_str(&rendered);
                quote_context.push_text(&rendered);
            }
            LineNode::SmartQuote(quote) => {
                let rendered = quote_context.quote(quote.quote.double);
                text.push_str(rendered);
                quote_context.push_text(rendered);
            }
            LineNode::TextSpan(span) if span.kind.is_line_decoration() => {
                let Some(body) = render_static_body(&span.body, params, quote_context)? else {
                    return Ok(None);
                };
                text.push_str(&body.text);
                nested.push(TextMarkupRun {
                    kind: span.kind,
                    options: span.options.clone(),
                });
                nested.extend(body.nested);
            }
            LineNode::Math(_) | LineNode::TextSpan(_) => return Ok(None),
        }
    }
    Ok(Some(RealizedStaticBody { text, nested }))
}

fn render_scope_param(name: &str, params: &Scope, position: usize) -> Result<String, LabelError> {
    let Some(value) = params.get(name) else {
        return Err(LabelError::UnsupportedSyntax {
            position,
            message: "unknown label parameter",
        });
    };
    scope_value_to_text(value, position)
}

fn scope_value_to_text(value: &Value, position: usize) -> Result<String, LabelError> {
    match value {
        Value::None => Ok(String::new()),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Int(value) => Ok(value.to_string()),
        Value::Float(value) if value.is_finite() => Ok(format_f64(*value)),
        Value::Float(_) => Err(LabelError::UnsupportedSyntax {
            position,
            message: "non-finite label parameter is not supported",
        }),
        Value::Str(value) => Ok(value.clone()),
        Value::Date(value) => Ok(value.to_string()),
        Value::DateTime(value) => Ok(value.to_string()),
        Value::UtcDateTime(value) => Ok(value.to_rfc3339()),
        Value::Array(_) | Value::Dict(_) => Err(LabelError::UnsupportedSyntax {
            position,
            message: "label parameter value cannot be rendered as text",
        }),
    }
}

fn format_f64(value: f64) -> String {
    let mut text = value.to_string();
    if text == "-0" {
        text = "0".to_string();
    }
    text
}
