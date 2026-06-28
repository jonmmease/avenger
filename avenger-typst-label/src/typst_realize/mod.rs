//! Content realization for labels.
//!
//! This mirrors the static realization step between upstream `typst-eval` and
//! `typst-layout`: flatten retained text/model/symbol markup and parameter
//! values into renderable single-line text and math nodes.

use crate::typst_diag::LabelError;
use crate::typst_label::{LabelParamValue, LabelParams};
use crate::typst_library::text::content::{
    LineNode, MathSpan, ParsedLine, PlainTextNode, TextMarkupKind, TextMarkupOptions,
};

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
    line: &ParsedLine,
    params: &LabelParams,
) -> Result<Option<RenderLine>, LabelError> {
    let mut nodes: Vec<RenderNode> = Vec::new();
    let mut pending_plain = String::new();
    let mut pending_start = None;
    let mut pending_end = 0usize;

    let flush_plain = |nodes: &mut Vec<RenderNode>,
                       pending_plain: &mut String,
                       pending_start: &mut Option<usize>,
                       pending_end: usize| {
        if let Some(start) = pending_start.take() {
            if !pending_plain.is_empty() {
                nodes.push(RenderNode::Plain(PlainTextNode {
                    text: std::mem::take(pending_plain),
                    byte_range: start..pending_end,
                }));
            }
        }
    };

    for node in &line.nodes {
        match node {
            LineNode::Plain(plain) => {
                if pending_start.is_none() {
                    pending_start = Some(plain.byte_range.start);
                }
                pending_end = plain.byte_range.end;
                pending_plain.push_str(&plain.text);
            }
            LineNode::Emoji(alias) => {
                if pending_start.is_none() {
                    pending_start = Some(alias.byte_range.start);
                }
                pending_end = alias.byte_range.end;
                pending_plain.push_str(alias.emoji);
            }
            LineNode::Symbol(alias) => {
                if pending_start.is_none() {
                    pending_start = Some(alias.byte_range.start);
                }
                pending_end = alias.byte_range.end;
                pending_plain.push_str(alias.text);
            }
            LineNode::Param(param) => {
                if pending_start.is_none() {
                    pending_start = Some(param.byte_range.start);
                }
                pending_end = param.byte_range.end;
                pending_plain.push_str(&render_label_param(
                    param.name.as_str(),
                    params,
                    param.byte_range.start,
                )?);
            }
            LineNode::Math(math) => {
                flush_plain(
                    &mut nodes,
                    &mut pending_plain,
                    &mut pending_start,
                    pending_end,
                );
                nodes.push(RenderNode::Math(math.clone()));
            }
            LineNode::TextSpan(span) => {
                let Some(kind) = supported_static_markup_kind(span.kind) else {
                    return Ok(None);
                };
                let Some(body) = render_static_body(&span.body, params)? else {
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
        TextMarkupKind::Lower => text.chars().flat_map(char::to_lowercase).collect(),
        TextMarkupKind::Upper => text.chars().flat_map(char::to_uppercase).collect(),
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
    params: &LabelParams,
) -> Result<Option<RealizedStaticBody>, LabelError> {
    let mut text = String::new();
    let mut nested = Vec::new();
    for node in nodes {
        match node {
            LineNode::Plain(plain) => text.push_str(&plain.text),
            LineNode::Emoji(alias) => text.push_str(alias.emoji),
            LineNode::Symbol(alias) => text.push_str(alias.text),
            LineNode::Param(param) => {
                text.push_str(&render_label_param(
                    param.name.as_str(),
                    params,
                    param.byte_range.start,
                )?);
            }
            LineNode::TextSpan(span) if span.kind.is_line_decoration() => {
                let Some(body) = render_static_body(&span.body, params)? else {
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

fn render_label_param(
    name: &str,
    params: &LabelParams,
    position: usize,
) -> Result<String, LabelError> {
    let Some(value) = params.get(name) else {
        return Err(LabelError::UnsupportedSyntax {
            position,
            message: "unknown label parameter",
        });
    };
    label_param_to_text(value, position)
}

fn label_param_to_text(value: &LabelParamValue, position: usize) -> Result<String, LabelError> {
    match value {
        LabelParamValue::None => Ok(String::new()),
        LabelParamValue::Bool(value) => Ok(value.to_string()),
        LabelParamValue::Int(value) => Ok(value.to_string()),
        LabelParamValue::Float(value) if value.is_finite() => Ok(format_f64(*value)),
        LabelParamValue::Float(_) => Err(LabelError::UnsupportedSyntax {
            position,
            message: "non-finite label parameter is not supported",
        }),
        LabelParamValue::Str(value) => Ok(value.clone()),
        LabelParamValue::Array(_) | LabelParamValue::Dict(_) => {
            Err(LabelError::UnsupportedSyntax {
                position,
                message: "label parameter value cannot be rendered as text",
            })
        }
    }
}

fn format_f64(value: f64) -> String {
    let mut text = value.to_string();
    if text == "-0" {
        text = "0".to_string();
    }
    text
}
