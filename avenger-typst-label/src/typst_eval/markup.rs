use std::ops::Range;

use crate::label::{LabelParamValue, LabelParams};
use crate::typst_diag::LabelError;
use crate::typst_eval::delimiter::{MathDelimiterInfo, MathDisplayHint};
use crate::typst_library::Color;
use crate::typst_library::text::content::{
    DecorationDash, DecorationDashLength, DecorationLength, DecorationStroke, EmojiAlias,
    LabelParamRef, LineNode, MathSpan, ParsedLine, PlainTextNode, SymbolAlias, TextMarkupKind,
    TextMarkupOptions, TextMarkupSpan,
};
use crate::typst_svg::{StrokeCap, StrokeJoin};

use super::math::named_math_symbol;

use crate::typst_syntax::ast::{self as typst_ast, AstNode};
use crate::typst_syntax::{
    RangeMapper, RootedPath, SpanKind, SyntaxKind, SyntaxNode, VirtualPath, VirtualRoot,
};

#[cfg(test)]
pub(crate) fn parse_line(source: &str) -> Result<ParsedLine, LabelError> {
    parse_line_with_params(source, &LabelParams::default())
}

pub(crate) fn parse_line_with_params(
    source: &str,
    params: &LabelParams,
) -> Result<ParsedLine, LabelError> {
    let mut root = crate::typst_syntax::parse(source);
    synthesize_ranges(&mut root, source.len())?;
    reject_syntax_errors(&root)?;
    let markup = root
        .cast::<typst_ast::Markup>()
        .ok_or_else(|| LabelError::Engine {
            start: 0,
            end: source.len(),
            message: "Typst parser did not return a markup root".to_string(),
        })?;

    let mut nodes = Vec::new();
    lower_markup(markup, source, params, &mut nodes)?;
    Ok(ParsedLine {
        source: source.to_string(),
        nodes,
    })
}

fn lower_markup(
    markup: typst_ast::Markup<'_>,
    source: &str,
    params: &LabelParams,
    nodes: &mut Vec<LineNode>,
) -> Result<(), LabelError> {
    for expr in markup.exprs() {
        lower_markup_expr(expr, source, params, nodes)?;
    }
    Ok(())
}

fn lower_markup_expr(
    expr: typst_ast::Expr<'_>,
    source: &str,
    params: &LabelParams,
    nodes: &mut Vec<LineNode>,
) -> Result<(), LabelError> {
    match expr {
        typst_ast::Expr::Text(text) => {
            push_plain(nodes, text.get().as_str(), text.to_untyped().range());
        }
        typst_ast::Expr::Space(space) => {
            let node = space.to_untyped();
            push_plain(nodes, node.full_text().as_str(), node.range());
        }
        typst_ast::Expr::Escape(escape) => {
            let text = escape.get().to_string();
            push_plain(nodes, &text, escape.to_untyped().range());
        }
        typst_ast::Expr::Shorthand(shorthand) => {
            let text = shorthand.get().to_string();
            push_plain(nodes, &text, shorthand.to_untyped().range());
        }
        typst_ast::Expr::SmartQuote(quote) => {
            let node = quote.to_untyped();
            push_plain(nodes, node.full_text().as_str(), node.range());
        }
        typst_ast::Expr::Equation(equation) => {
            if equation.block() {
                return Err(unsupported(
                    equation.to_untyped().range().start,
                    "display math is not supported in Avenger text lines",
                ));
            }
            let body = equation.body();
            let source_range = body.to_untyped().range();
            let full_range = equation.to_untyped().range();
            let source_text = source[source_range.clone()].to_string();
            nodes.push(LineNode::Math(MathSpan {
                source: source_text,
                source_range,
                delimiter: math_delimiter_info(full_range),
            }));
        }
        typst_ast::Expr::FuncCall(call) => {
            lower_static_call(call, source, params, nodes)?;
        }
        typst_ast::Expr::Ident(ident) => {
            let range = expand_hash_range(source, ident.to_untyped().range());
            nodes.push(LineNode::Param(LabelParamRef {
                name: ident.as_str().to_string(),
                byte_range: range,
            }));
        }
        typst_ast::Expr::FieldAccess(access) => {
            lower_static_field_access(access, source, nodes)?;
        }
        typst_ast::Expr::Strong(strong) => {
            lower_markup_span(
                TextMarkupKind::Strong,
                TextMarkupOptions::default(),
                strong.body(),
                strong.to_untyped().range(),
                source,
                params,
                nodes,
            )?;
        }
        typst_ast::Expr::Emph(emph) => {
            lower_markup_span(
                TextMarkupKind::Emph,
                TextMarkupOptions::default(),
                emph.body(),
                emph.to_untyped().range(),
                source,
                params,
                nodes,
            )?;
        }
        typst_ast::Expr::Raw(raw) => {
            lower_raw_markup(raw, nodes)?;
        }
        other => {
            return Err(unsupported_markup_expr(source, other));
        }
    }
    Ok(())
}

fn lower_static_call(
    call: typst_ast::FuncCall<'_>,
    source: &str,
    params: &LabelParams,
    nodes: &mut Vec<LineNode>,
) -> Result<(), LabelError> {
    let range = expand_hash_range(source, call.to_untyped().range());
    let Some(name) = code_expr_name(call.callee()) else {
        return Err(unsupported(range.start, "unsupported static text command"));
    };
    let Some(kind) = text_span_kind(&name) else {
        return Err(unsupported(range.start, "unsupported static text command"));
    };

    let mut body = None;
    let mut options = TextMarkupOptions::default();
    for arg in call.args().items() {
        match arg {
            typst_ast::Arg::Pos(typst_ast::Expr::ContentBlock(block)) => {
                if kind == TextMarkupKind::Raw {
                    return Err(unsupported(range.start, "raw expects a string literal"));
                }
                let body_markup = block.body();
                let body_range = body_markup.to_untyped().range();
                let mut body_nodes = Vec::new();
                lower_markup(body_markup, source, params, &mut body_nodes)?;
                if body.replace((body_nodes, body_range)).is_some() {
                    return Err(unsupported(
                        range.start,
                        "static text command expects one bracketed content block",
                    ));
                }
            }
            typst_ast::Arg::Pos(typst_ast::Expr::Str(string))
                if kind.is_case_transform() || kind == TextMarkupKind::Raw =>
            {
                let string_range = string.to_untyped().range();
                let body_nodes = vec![LineNode::Plain(PlainTextNode {
                    text: string.get().to_string(),
                    byte_range: string_range.clone(),
                })];
                if body.replace((body_nodes, string_range)).is_some() {
                    return Err(unsupported(
                        range.start,
                        "static text command expects one bracketed content block",
                    ));
                }
            }
            typst_ast::Arg::Named(named) => {
                parse_text_markup_option(kind, named, params, &mut options)?;
            }
            typst_ast::Arg::Spread(_) => {
                return Err(unsupported(
                    range.start,
                    "static text commands do not support spread arguments",
                ));
            }
            typst_ast::Arg::Pos(_) => {
                return Err(unsupported(
                    range.start,
                    "static text command expects bracketed content",
                ));
            }
        }
    }

    let Some(body) = body else {
        return Err(unsupported(
            range.start,
            "static text command expects bracketed content",
        ));
    };

    let (body_nodes, body_range) = body;
    nodes.push(LineNode::TextSpan(TextMarkupSpan {
        kind,
        options,
        body: body_nodes,
        byte_range: range,
        body_range,
    }));
    Ok(())
}

fn lower_raw_markup(raw: typst_ast::Raw<'_>, nodes: &mut Vec<LineNode>) -> Result<(), LabelError> {
    let range = raw.to_untyped().range();
    if raw.block() {
        return Err(unsupported(
            range.start,
            "raw block labels are not supported",
        ));
    }
    if raw.lang().is_some() {
        return Err(unsupported(
            range.start,
            "raw syntax highlighting is not supported",
        ));
    }

    let lines = raw.lines().collect::<Vec<_>>();
    let body_range = lines
        .first()
        .zip(lines.last())
        .map(|(first, last)| first.to_untyped().range().start..last.to_untyped().range().end)
        .unwrap_or_else(|| range.clone());
    let text = lines
        .iter()
        .map(|line| line.get().as_str())
        .collect::<Vec<_>>()
        .join("\n");
    nodes.push(LineNode::TextSpan(TextMarkupSpan {
        kind: TextMarkupKind::Raw,
        options: TextMarkupOptions::default(),
        body: vec![LineNode::Plain(PlainTextNode {
            text,
            byte_range: body_range.clone(),
        })],
        byte_range: range,
        body_range,
    }));
    Ok(())
}

fn lower_markup_span(
    kind: TextMarkupKind,
    options: TextMarkupOptions,
    body_markup: typst_ast::Markup<'_>,
    byte_range: Range<usize>,
    source: &str,
    params: &LabelParams,
    nodes: &mut Vec<LineNode>,
) -> Result<(), LabelError> {
    let body_range = body_markup.to_untyped().range();
    let mut body = Vec::new();
    lower_markup(body_markup, source, params, &mut body)?;
    nodes.push(LineNode::TextSpan(TextMarkupSpan {
        kind,
        options,
        body,
        byte_range,
        body_range,
    }));
    Ok(())
}

fn lower_static_field_access(
    access: typst_ast::FieldAccess<'_>,
    source: &str,
    nodes: &mut Vec<LineNode>,
) -> Result<(), LabelError> {
    let range = expand_hash_range(source, access.to_untyped().range());
    let Some(name) = code_field_access_name(access) else {
        return Err(unsupported(range.start, "unsupported static text command"));
    };
    if let Some(alias) = name.strip_prefix("emoji.") {
        let Some(emoji) = emoji_alias(alias) else {
            return Err(unsupported(range.start, "unknown emoji alias"));
        };
        nodes.push(LineNode::Emoji(EmojiAlias {
            name: alias.to_string(),
            emoji,
            byte_range: range,
        }));
        return Ok(());
    }

    if let Some(alias) = name.strip_prefix("sym.") {
        let Some(text) = named_math_symbol(alias) else {
            return Err(unsupported(range.start, "unknown symbol alias"));
        };
        nodes.push(LineNode::Symbol(SymbolAlias {
            name: alias.to_string(),
            text,
            byte_range: range,
        }));
        return Ok(());
    }

    Err(unsupported(range.start, "unsupported static text command"))
}

fn code_expr_name(expr: typst_ast::Expr<'_>) -> Option<String> {
    match expr {
        typst_ast::Expr::Ident(ident) => Some(ident.as_str().to_string()),
        typst_ast::Expr::FieldAccess(access) => code_field_access_name(access),
        _ => None,
    }
}

fn code_field_access_name(access: typst_ast::FieldAccess<'_>) -> Option<String> {
    let mut name = code_expr_name(access.target())?;
    name.push('.');
    name.push_str(access.field().as_str());
    Some(name)
}

fn text_span_kind(name: &str) -> Option<TextMarkupKind> {
    match name {
        "underline" => Some(TextMarkupKind::Underline),
        "strike" => Some(TextMarkupKind::Strike),
        "overline" => Some(TextMarkupKind::Overline),
        "sub" => Some(TextMarkupKind::Subscript),
        "super" => Some(TextMarkupKind::Superscript),
        "highlight" => Some(TextMarkupKind::Highlight),
        "lower" => Some(TextMarkupKind::Lower),
        "upper" => Some(TextMarkupKind::Upper),
        "smallcaps" => Some(TextMarkupKind::Smallcaps),
        "emph" => Some(TextMarkupKind::Emph),
        "strong" => Some(TextMarkupKind::Strong),
        "raw" => Some(TextMarkupKind::Raw),
        _ => None,
    }
}

pub(crate) fn is_retained_markup_name(name: &str) -> bool {
    text_span_kind(name).is_some()
        || matches!(name, "auto" | "true" | "false" | "none" | "sym" | "emoji")
        || named_color(name).is_some()
}

fn parse_text_markup_option(
    kind: TextMarkupKind,
    named: typst_ast::Named<'_>,
    params: &LabelParams,
    options: &mut TextMarkupOptions,
) -> Result<(), LabelError> {
    let position = named.to_untyped().range().start;
    if kind == TextMarkupKind::Smallcaps {
        match named.name().as_str() {
            "all" => {
                options.smallcaps.all = parse_bool_with_message(
                    named.expr(),
                    position,
                    params,
                    "unsupported smallcaps boolean value",
                )?;
            }
            _ => return Err(unsupported(position, "unsupported smallcaps option")),
        }
        return Ok(());
    }

    if kind == TextMarkupKind::Strong {
        match named.name().as_str() {
            "delta" => {
                options.strong.delta = parse_i64_with_message(
                    named.expr(),
                    position,
                    params,
                    "unsupported strong delta value",
                )?;
            }
            _ => return Err(unsupported(position, "unsupported strong option")),
        }
        return Ok(());
    }

    if kind.is_script() {
        match named.name().as_str() {
            "typographic" => {
                options.script.typographic = parse_bool_with_message(
                    named.expr(),
                    position,
                    params,
                    "unsupported script typographic value",
                )?;
            }
            "baseline" => {
                options.script.baseline = parse_auto_or_length(named.expr(), position, params)?;
            }
            "size" => {
                options.script.size = parse_auto_or_length(named.expr(), position, params)?;
            }
            _ => return Err(unsupported(position, "unsupported script option")),
        }
        return Ok(());
    }

    if !kind.is_line_decoration() {
        return Err(unsupported(
            position,
            "this static text command does not support options",
        ));
    }

    match named.name().as_str() {
        "stroke" => {
            options.decoration.stroke =
                parse_decoration_stroke_with_params(named.expr(), position, params)?;
        }
        "offset" => {
            options.decoration.offset = parse_auto_or_length(named.expr(), position, params)?;
        }
        "extent" => {
            options.decoration.extent = parse_length(named.expr(), position, params)?;
        }
        "background" => {
            options.decoration.background = parse_bool(named.expr(), position, params)?;
        }
        "evade" => {
            if kind == TextMarkupKind::Strike {
                return Err(unsupported(position, "strike does not support evade"));
            }
            options.decoration.evade = parse_evade(named.expr(), position, params)?;
        }
        _ => return Err(unsupported(position, "unsupported decoration option")),
    }

    Ok(())
}

pub(crate) fn parse_decoration_stroke(
    expr: typst_ast::Expr<'_>,
    position: usize,
) -> Result<DecorationStroke, LabelError> {
    parse_decoration_stroke_with_params(expr, position, &LabelParams::default())
}

fn parse_decoration_stroke_with_params(
    expr: typst_ast::Expr<'_>,
    position: usize,
    params: &LabelParams,
) -> Result<DecorationStroke, LabelError> {
    match expr {
        typst_ast::Expr::Ident(ident) => {
            if let Some(value) = params.get(ident.as_str()) {
                return param_value_to_stroke(value, position);
            }
            parse_stroke_part_with_params(expr, position, params)
        }
        typst_ast::Expr::Auto(_) => Ok(DecorationStroke::default()),
        typst_ast::Expr::CodeBlock(block) => {
            let exprs = block.body().exprs().collect::<Vec<_>>();
            let [expr] = &exprs[..] else {
                return Err(unsupported(position, "unsupported decoration stroke value"));
            };
            parse_decoration_stroke_with_params(*expr, position, params)
        }
        typst_ast::Expr::Dict(dict) => parse_stroke_dict(dict, position, params),
        typst_ast::Expr::Binary(binary) if binary.op() == typst_ast::BinOp::Add => {
            let mut stroke = parse_stroke_part_with_params(binary.lhs(), position, params)?;
            stroke.merge(
                parse_stroke_part_with_params(binary.rhs(), position, params)?,
                position,
            )?;
            Ok(stroke)
        }
        _ => parse_stroke_part_with_params(expr, position, params),
    }
}

fn parse_stroke_dict(
    dict: typst_ast::Dict<'_>,
    position: usize,
    params: &LabelParams,
) -> Result<DecorationStroke, LabelError> {
    let mut stroke = DecorationStroke::default();
    for item in dict.items() {
        let typst_ast::DictItem::Named(named) = item else {
            return Err(unsupported(position, "unsupported stroke dictionary item"));
        };
        let item_position = named.to_untyped().range().start;
        match named.name().as_str() {
            "paint" => {
                stroke.merge(
                    DecorationStroke {
                        paint: Some(parse_paint(named.expr(), item_position, params)?),
                        thickness: None,
                        line_cap: None,
                        line_join: None,
                        dash: None,
                    },
                    item_position,
                )?;
            }
            "thickness" => {
                stroke.merge(
                    DecorationStroke {
                        paint: None,
                        thickness: Some(parse_length(named.expr(), item_position, params)?),
                        line_cap: None,
                        line_join: None,
                        dash: None,
                    },
                    item_position,
                )?;
            }
            "cap" => {
                stroke.merge(
                    DecorationStroke {
                        paint: None,
                        thickness: None,
                        line_cap: Some(parse_line_cap(named.expr(), item_position, params)?),
                        line_join: None,
                        dash: None,
                    },
                    item_position,
                )?;
            }
            "join" => {
                stroke.merge(
                    DecorationStroke {
                        paint: None,
                        thickness: None,
                        line_cap: None,
                        line_join: Some(parse_line_join(named.expr(), item_position, params)?),
                        dash: None,
                    },
                    item_position,
                )?;
            }
            "dash" => {
                stroke.merge(
                    DecorationStroke {
                        paint: None,
                        thickness: None,
                        line_cap: None,
                        line_join: None,
                        dash: Some(parse_dash(named.expr(), item_position, params)?),
                    },
                    item_position,
                )?;
            }
            _ => {
                return Err(unsupported(
                    item_position,
                    "unsupported stroke dictionary field",
                ));
            }
        }
    }
    Ok(stroke)
}

fn parse_stroke_part_with_params(
    expr: typst_ast::Expr<'_>,
    position: usize,
    params: &LabelParams,
) -> Result<DecorationStroke, LabelError> {
    if let Some(value) = param_value_for_ident(expr, params) {
        return param_value_to_stroke_part(value, position);
    }
    if let Ok(paint) = parse_paint(expr, position, params) {
        return Ok(DecorationStroke {
            paint: Some(paint),
            thickness: None,
            line_cap: None,
            line_join: None,
            dash: None,
        });
    }
    if let Ok(thickness) = parse_length(expr, position, params) {
        return Ok(DecorationStroke {
            paint: None,
            thickness: Some(thickness),
            line_cap: None,
            line_join: None,
            dash: None,
        });
    }
    if matches!(expr, typst_ast::Expr::Auto(_)) {
        return Ok(DecorationStroke::default());
    }
    Err(unsupported(position, "unsupported decoration stroke value"))
}

trait MergeDecorationStroke {
    fn merge(&mut self, other: DecorationStroke, position: usize) -> Result<(), LabelError>;
}

impl MergeDecorationStroke for DecorationStroke {
    fn merge(&mut self, other: DecorationStroke, position: usize) -> Result<(), LabelError> {
        if let Some(paint) = other.paint {
            if self.paint.replace(paint).is_some() {
                return Err(unsupported(position, "duplicate decoration stroke paint"));
            }
        }
        if let Some(thickness) = other.thickness {
            if self.thickness.replace(thickness).is_some() {
                return Err(unsupported(
                    position,
                    "duplicate decoration stroke thickness",
                ));
            }
        }
        if let Some(line_cap) = other.line_cap {
            if self.line_cap.replace(line_cap).is_some() {
                return Err(unsupported(position, "duplicate decoration stroke cap"));
            }
        }
        if let Some(line_join) = other.line_join {
            if self.line_join.replace(line_join).is_some() {
                return Err(unsupported(position, "duplicate decoration stroke join"));
            }
        }
        if let Some(dash) = other.dash {
            if self.dash.replace(dash).is_some() {
                return Err(unsupported(position, "duplicate decoration stroke dash"));
            }
        }
        Ok(())
    }
}

fn parse_auto_or_length(
    expr: typst_ast::Expr<'_>,
    position: usize,
    params: &LabelParams,
) -> Result<Option<DecorationLength>, LabelError> {
    if let Some(LabelParamValue::Str(value)) = param_value_for_ident(expr, params) {
        if value.trim() == "auto" {
            return Ok(None);
        }
    }
    match expr {
        typst_ast::Expr::Auto(_) => Ok(None),
        _ => parse_length(expr, position, params).map(Some),
    }
}

fn parse_length(
    expr: typst_ast::Expr<'_>,
    position: usize,
    params: &LabelParams,
) -> Result<DecorationLength, LabelError> {
    if let Some(value) = param_value_for_ident(expr, params) {
        return param_value_to_length(value, position);
    }
    match expr {
        typst_ast::Expr::Numeric(numeric) => {
            let (value, unit) = numeric.get();
            length_from_unit(value as f32, unit, position)
        }
        typst_ast::Expr::Unary(unary) => {
            let sign = match unary.op() {
                typst_ast::UnOp::Pos => 1.0,
                typst_ast::UnOp::Neg => -1.0,
                typst_ast::UnOp::Not => {
                    return Err(unsupported(position, "unsupported decoration length"));
                }
            };
            let length = parse_length(unary.expr(), position, params)?;
            Ok(match length {
                DecorationLength::Pt(value) => DecorationLength::Pt(sign * value),
                DecorationLength::Em(value) => DecorationLength::Em(sign * value),
            })
        }
        _ => Err(unsupported(position, "unsupported decoration length")),
    }
}

fn length_from_unit(
    value: f32,
    unit: typst_ast::Unit,
    position: usize,
) -> Result<DecorationLength, LabelError> {
    match unit {
        typst_ast::Unit::Pt => Ok(DecorationLength::Pt(value)),
        typst_ast::Unit::Mm => Ok(DecorationLength::Pt(value * 72.0 / 25.4)),
        typst_ast::Unit::Cm => Ok(DecorationLength::Pt(value * 72.0 / 2.54)),
        typst_ast::Unit::In => Ok(DecorationLength::Pt(value * 72.0)),
        typst_ast::Unit::Em => Ok(DecorationLength::Em(value)),
        typst_ast::Unit::Rad
        | typst_ast::Unit::Deg
        | typst_ast::Unit::Fr
        | typst_ast::Unit::Percent => {
            Err(unsupported(position, "unsupported decoration length unit"))
        }
    }
}

fn parse_bool(
    expr: typst_ast::Expr<'_>,
    position: usize,
    params: &LabelParams,
) -> Result<bool, LabelError> {
    parse_bool_with_message(
        expr,
        position,
        params,
        "unsupported decoration boolean value",
    )
}

fn parse_bool_with_message(
    expr: typst_ast::Expr<'_>,
    position: usize,
    params: &LabelParams,
    message: &'static str,
) -> Result<bool, LabelError> {
    if let Some(value) = param_value_for_ident(expr, params) {
        return match value {
            LabelParamValue::Bool(value) => Ok(*value),
            _ => Err(unsupported(position, message)),
        };
    }
    match expr {
        typst_ast::Expr::Bool(value) => Ok(value.get()),
        _ => Err(unsupported(position, message)),
    }
}

fn parse_i64_with_message(
    expr: typst_ast::Expr<'_>,
    position: usize,
    params: &LabelParams,
    message: &'static str,
) -> Result<i64, LabelError> {
    if let Some(value) = param_value_for_ident(expr, params) {
        return match value {
            LabelParamValue::Int(value) => Ok(*value),
            _ => Err(unsupported(position, message)),
        };
    }
    match expr {
        typst_ast::Expr::Int(value) => Ok(value.get()),
        typst_ast::Expr::Unary(unary) => {
            let sign = match unary.op() {
                typst_ast::UnOp::Pos => 1,
                typst_ast::UnOp::Neg => -1,
                typst_ast::UnOp::Not => return Err(unsupported(position, message)),
            };
            parse_i64_with_message(unary.expr(), position, params, message)
                .map(|value| sign * value)
        }
        _ => Err(unsupported(position, message)),
    }
}

fn parse_line_cap(
    expr: typst_ast::Expr<'_>,
    position: usize,
    params: &LabelParams,
) -> Result<StrokeCap, LabelError> {
    let value = if let Some(value) = param_value_for_ident(expr, params) {
        param_value_to_string(value, position, "unsupported stroke cap value")?
    } else {
        let typst_ast::Expr::Str(value) = expr else {
            return Err(unsupported(position, "unsupported stroke cap value"));
        };
        value.get().to_string()
    };
    match value.as_str() {
        "butt" => Ok(StrokeCap::Butt),
        "round" => Ok(StrokeCap::Round),
        "square" => Ok(StrokeCap::Square),
        _ => Err(unsupported(position, "unsupported stroke cap value")),
    }
}

fn parse_line_join(
    expr: typst_ast::Expr<'_>,
    position: usize,
    params: &LabelParams,
) -> Result<StrokeJoin, LabelError> {
    let value = if let Some(value) = param_value_for_ident(expr, params) {
        param_value_to_string(value, position, "unsupported stroke join value")?
    } else {
        let typst_ast::Expr::Str(value) = expr else {
            return Err(unsupported(position, "unsupported stroke join value"));
        };
        value.get().to_string()
    };
    match value.as_str() {
        "bevel" => Ok(StrokeJoin::Bevel),
        "miter" => Ok(StrokeJoin::Miter),
        "round" => Ok(StrokeJoin::Round),
        _ => Err(unsupported(position, "unsupported stroke join value")),
    }
}

fn parse_dash(
    expr: typst_ast::Expr<'_>,
    position: usize,
    params: &LabelParams,
) -> Result<DecorationDash, LabelError> {
    if let Some(value) = param_value_for_ident(expr, params) {
        return param_value_to_dash(value, position);
    }
    match expr {
        typst_ast::Expr::Str(value) => named_dash(value.get().as_str())
            .ok_or_else(|| unsupported(position, "unsupported stroke dash value")),
        typst_ast::Expr::Array(array) => {
            let mut lengths = Vec::new();
            for item in array.items() {
                let typst_ast::ArrayItem::Pos(expr) = item else {
                    return Err(unsupported(position, "unsupported stroke dash array item"));
                };
                lengths.push(parse_dash_length(expr, position, params)?);
            }
            Ok(DecorationDash { array: lengths })
        }
        typst_ast::Expr::Dict(_) => Err(unsupported(
            position,
            "stroke dash phase is not supported in Avenger labels",
        )),
        _ => Err(unsupported(position, "unsupported stroke dash value")),
    }
}

fn parse_dash_length(
    expr: typst_ast::Expr<'_>,
    position: usize,
    params: &LabelParams,
) -> Result<DecorationDashLength, LabelError> {
    match expr {
        typst_ast::Expr::Str(value) if value.get().as_str() == "dot" => {
            Ok(DecorationDashLength::LineWidth)
        }
        _ => parse_length(expr, position, params).map(DecorationDashLength::Length),
    }
}

fn named_dash(name: &str) -> Option<DecorationDash> {
    let pt = |value| DecorationDashLength::Length(DecorationLength::Pt(value));
    let dot = DecorationDashLength::LineWidth;
    let array = match name {
        "solid" => Vec::new(),
        "dotted" => vec![dot, pt(2.0)],
        "densely-dotted" => vec![dot, pt(1.0)],
        "loosely-dotted" => vec![dot, pt(4.0)],
        "dashed" => vec![pt(3.0), pt(3.0)],
        "densely-dashed" => vec![pt(3.0), pt(2.0)],
        "loosely-dashed" => vec![pt(3.0), pt(6.0)],
        "dash-dotted" => vec![pt(3.0), pt(2.0), dot, pt(2.0)],
        "densely-dash-dotted" => vec![pt(3.0), pt(1.0), dot, pt(1.0)],
        "loosely-dash-dotted" => vec![pt(3.0), pt(4.0), dot, pt(4.0)],
        _ => return None,
    };
    Some(DecorationDash { array })
}

fn parse_evade(
    expr: typst_ast::Expr<'_>,
    position: usize,
    params: &LabelParams,
) -> Result<Option<bool>, LabelError> {
    if let Some(value) = param_value_for_ident(expr, params) {
        return match value {
            LabelParamValue::None => Ok(None),
            LabelParamValue::Bool(value) => Ok(Some(*value)),
            LabelParamValue::Str(value) if value == "auto" => Ok(None),
            _ => Err(unsupported(position, "unsupported decoration evade value")),
        };
    }
    match expr {
        typst_ast::Expr::Auto(_) => Ok(None),
        typst_ast::Expr::Bool(value) => Ok(Some(value.get())),
        _ => Err(unsupported(position, "unsupported decoration evade value")),
    }
}

fn parse_paint(
    expr: typst_ast::Expr<'_>,
    position: usize,
    params: &LabelParams,
) -> Result<Color, LabelError> {
    if let Some(value) = param_value_for_ident(expr, params) {
        return param_value_to_paint(value, position);
    }
    match expr {
        typst_ast::Expr::Ident(ident) => named_color(ident.as_str())
            .ok_or_else(|| unsupported(position, "unsupported decoration paint")),
        _ => Err(unsupported(position, "unsupported decoration paint")),
    }
}

fn param_value_for_ident<'a>(
    expr: typst_ast::Expr<'_>,
    params: &'a LabelParams,
) -> Option<&'a LabelParamValue> {
    let typst_ast::Expr::Ident(ident) = expr else {
        return None;
    };
    params.get(ident.as_str())
}

fn param_value_to_stroke(
    value: &LabelParamValue,
    position: usize,
) -> Result<DecorationStroke, LabelError> {
    match value {
        LabelParamValue::Str(value) => parse_stroke_literal(value, position),
        LabelParamValue::Dict(dict) => param_dict_to_stroke(dict, position),
        _ => param_value_to_stroke_part(value, position),
    }
}

fn param_value_to_stroke_part(
    value: &LabelParamValue,
    position: usize,
) -> Result<DecorationStroke, LabelError> {
    if let Ok(paint) = param_value_to_paint(value, position) {
        return Ok(DecorationStroke {
            paint: Some(paint),
            thickness: None,
            line_cap: None,
            line_join: None,
            dash: None,
        });
    }
    if let Ok(thickness) = param_value_to_length(value, position) {
        return Ok(DecorationStroke {
            paint: None,
            thickness: Some(thickness),
            line_cap: None,
            line_join: None,
            dash: None,
        });
    }
    if matches!(value, LabelParamValue::None) {
        return Ok(DecorationStroke::default());
    }
    Err(unsupported(
        position,
        "label parameter cannot be cast to stroke",
    ))
}

fn parse_stroke_literal(raw: &str, position: usize) -> Result<DecorationStroke, LabelError> {
    let parts = raw.split('+').map(str::trim).collect::<Vec<_>>();
    match &parts[..] {
        [single] => parse_stroke_literal_part(single, position),
        [left, right] => {
            let mut stroke = parse_stroke_literal_part(left, position)?;
            stroke.merge(parse_stroke_literal_part(right, position)?, position)?;
            Ok(stroke)
        }
        _ => Err(unsupported(
            position,
            "label parameter cannot be cast to stroke",
        )),
    }
}

fn parse_stroke_literal_part(raw: &str, position: usize) -> Result<DecorationStroke, LabelError> {
    if let Some(paint) = named_color(raw) {
        return Ok(DecorationStroke {
            paint: Some(paint),
            thickness: None,
            line_cap: None,
            line_join: None,
            dash: None,
        });
    }
    if let Some(thickness) = parse_length_literal(raw, position)? {
        return Ok(DecorationStroke {
            paint: None,
            thickness: Some(thickness),
            line_cap: None,
            line_join: None,
            dash: None,
        });
    }
    Err(unsupported(
        position,
        "label parameter cannot be cast to stroke",
    ))
}

fn param_dict_to_stroke(
    dict: &indexmap::IndexMap<String, LabelParamValue>,
    position: usize,
) -> Result<DecorationStroke, LabelError> {
    let mut stroke = DecorationStroke::default();
    for (name, value) in dict {
        match name.as_str() {
            "paint" => stroke.merge(
                DecorationStroke {
                    paint: Some(param_value_to_paint(value, position)?),
                    thickness: None,
                    line_cap: None,
                    line_join: None,
                    dash: None,
                },
                position,
            )?,
            "thickness" => stroke.merge(
                DecorationStroke {
                    paint: None,
                    thickness: Some(param_value_to_length(value, position)?),
                    line_cap: None,
                    line_join: None,
                    dash: None,
                },
                position,
            )?,
            "cap" => stroke.merge(
                DecorationStroke {
                    paint: None,
                    thickness: None,
                    line_cap: Some(parse_cap_literal(
                        &param_value_to_string(value, position, "unsupported stroke cap value")?,
                        position,
                    )?),
                    line_join: None,
                    dash: None,
                },
                position,
            )?,
            "join" => stroke.merge(
                DecorationStroke {
                    paint: None,
                    thickness: None,
                    line_cap: None,
                    line_join: Some(parse_join_literal(
                        &param_value_to_string(value, position, "unsupported stroke join value")?,
                        position,
                    )?),
                    dash: None,
                },
                position,
            )?,
            "dash" => stroke.merge(
                DecorationStroke {
                    paint: None,
                    thickness: None,
                    line_cap: None,
                    line_join: None,
                    dash: Some(param_value_to_dash(value, position)?),
                },
                position,
            )?,
            _ => {
                return Err(unsupported(position, "unsupported stroke dictionary field"));
            }
        }
    }
    Ok(stroke)
}

fn param_value_to_paint(value: &LabelParamValue, position: usize) -> Result<Color, LabelError> {
    let LabelParamValue::Str(value) = value else {
        return Err(unsupported(position, "unsupported decoration paint"));
    };
    named_color(value.trim()).ok_or_else(|| unsupported(position, "unsupported decoration paint"))
}

fn param_value_to_length(
    value: &LabelParamValue,
    position: usize,
) -> Result<DecorationLength, LabelError> {
    let LabelParamValue::Str(value) = value else {
        return Err(unsupported(
            position,
            "label parameter cannot be cast to decoration length",
        ));
    };
    parse_length_literal(value, position)?.ok_or_else(|| {
        unsupported(
            position,
            "label parameter cannot be cast to decoration length",
        )
    })
}

fn parse_length_literal(
    raw: &str,
    position: usize,
) -> Result<Option<DecorationLength>, LabelError> {
    let raw = raw.trim();
    let Some((number, unit)) = split_number_unit(raw) else {
        return Ok(None);
    };
    let value = number
        .trim()
        .parse::<f32>()
        .map_err(|_| unsupported(position, "unsupported decoration length"))?;
    let length = match unit {
        "pt" => DecorationLength::Pt(value),
        "mm" => DecorationLength::Pt(value * 72.0 / 25.4),
        "cm" => DecorationLength::Pt(value * 72.0 / 2.54),
        "in" => DecorationLength::Pt(value * 72.0),
        "em" => DecorationLength::Em(value),
        _ => return Ok(None),
    };
    Ok(Some(length))
}

fn split_number_unit(raw: &str) -> Option<(&str, &str)> {
    ["pt", "mm", "cm", "in", "em"]
        .iter()
        .find_map(|unit| raw.strip_suffix(unit).map(|number| (number, *unit)))
}

fn param_value_to_string(
    value: &LabelParamValue,
    position: usize,
    message: &'static str,
) -> Result<String, LabelError> {
    match value {
        LabelParamValue::Str(value) => Ok(value.clone()),
        _ => Err(unsupported(position, message)),
    }
}

fn param_value_to_dash(
    value: &LabelParamValue,
    position: usize,
) -> Result<DecorationDash, LabelError> {
    match value {
        LabelParamValue::Str(value) => named_dash(value.trim())
            .ok_or_else(|| unsupported(position, "unsupported stroke dash value")),
        LabelParamValue::Array(values) => values
            .iter()
            .map(|value| match value {
                LabelParamValue::Str(value) if value == "dot" => {
                    Ok(DecorationDashLength::LineWidth)
                }
                _ => param_value_to_length(value, position).map(DecorationDashLength::Length),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|array| DecorationDash { array }),
        _ => Err(unsupported(position, "unsupported stroke dash value")),
    }
}

fn parse_cap_literal(value: &str, position: usize) -> Result<StrokeCap, LabelError> {
    match value.trim() {
        "butt" => Ok(StrokeCap::Butt),
        "round" => Ok(StrokeCap::Round),
        "square" => Ok(StrokeCap::Square),
        _ => Err(unsupported(position, "unsupported stroke cap value")),
    }
}

fn parse_join_literal(value: &str, position: usize) -> Result<StrokeJoin, LabelError> {
    match value.trim() {
        "bevel" => Ok(StrokeJoin::Bevel),
        "miter" => Ok(StrokeJoin::Miter),
        "round" => Ok(StrokeJoin::Round),
        _ => Err(unsupported(position, "unsupported stroke join value")),
    }
}

fn named_color(name: &str) -> Option<Color> {
    Some(match name {
        "black" => Color::rgba(0.0, 0.0, 0.0, 1.0),
        "white" => Color::rgba(1.0, 1.0, 1.0, 1.0),
        "red" => Color::rgba(1.0, 0.0, 0.0, 1.0),
        "green" => Color::rgba(0.0, 0.5, 0.0, 1.0),
        "blue" => Color::rgba(0.0, 0.0, 1.0, 1.0),
        "yellow" => Color::rgba(1.0, 1.0, 0.0, 1.0),
        "orange" => Color::rgba(1.0, 0.65, 0.0, 1.0),
        "purple" => Color::rgba(0.5, 0.0, 0.5, 1.0),
        "maroon" => Color::rgba(0.5, 0.0, 0.0, 1.0),
        "gray" | "grey" => Color::rgba(0.5, 0.5, 0.5, 1.0),
        "silver" => Color::rgba(0.75, 0.75, 0.75, 1.0),
        "teal" => Color::rgba(0.0, 0.5, 0.5, 1.0),
        "aqua" | "cyan" => Color::rgba(0.0, 1.0, 1.0, 1.0),
        "navy" => Color::rgba(0.0, 0.0, 0.5, 1.0),
        "lime" => Color::rgba(0.0, 1.0, 0.0, 1.0),
        "olive" => Color::rgba(0.5, 0.5, 0.0, 1.0),
        "fuchsia" | "magenta" => Color::rgba(1.0, 0.0, 1.0, 1.0),
        _ => return None,
    })
}

fn emoji_alias(name: &str) -> Option<&'static str> {
    match name {
        "face" => Some("😀"),
        "rocket" => Some("🚀"),
        "chart.up" => Some("📈"),
        _ => None,
    }
}

fn push_plain(nodes: &mut Vec<LineNode>, text: &str, range: Range<usize>) {
    if text.is_empty() {
        return;
    }
    if let Some(LineNode::Plain(previous)) = nodes.last_mut() {
        previous.text.push_str(text);
        previous.byte_range.end = range.end;
        return;
    }
    nodes.push(LineNode::Plain(PlainTextNode {
        text: text.to_string(),
        byte_range: range,
    }));
}

fn math_delimiter_info(full_range: Range<usize>) -> MathDelimiterInfo {
    MathDelimiterInfo {
        opening_range: full_range.start..full_range.start + 1,
        closing_range: full_range.end.saturating_sub(1)..full_range.end,
        full_range,
        display_hint: MathDisplayHint::Inline,
    }
}

fn expand_hash_range(source: &str, range: Range<usize>) -> Range<usize> {
    if range.start > 0 && source.as_bytes().get(range.start - 1) == Some(&b'#') {
        range.start - 1..range.end
    } else {
        range
    }
}

fn reject_syntax_errors(root: &SyntaxNode) -> Result<(), LabelError> {
    if !root.diagnosis().errors {
        return Ok(());
    }
    let (errors, _) = root.errors_and_warnings();
    let message = errors
        .first()
        .map(|error| error.message.to_string())
        .unwrap_or_else(|| "invalid Typst syntax".to_string());
    let position = first_error_range(root)
        .map(|range| range.start)
        .unwrap_or_default();
    Err(LabelError::Syntax { position, message })
}

fn first_error_range(node: &SyntaxNode) -> Option<Range<usize>> {
    if node.kind() == SyntaxKind::Error {
        return Some(node.range());
    }
    node.children().find_map(first_error_range)
}

fn unsupported_markup_expr(source: &str, expr: typst_ast::Expr<'_>) -> LabelError {
    let range = expr.to_untyped().range();
    let expanded = expand_hash_range(source, range.clone());
    if expanded.start != range.start {
        unsupported(expanded.start, "unsupported static text command")
    } else {
        unsupported(
            range.start,
            "this Typst markup construct is not supported in Avenger text lines",
        )
    }
}

fn unsupported(position: usize, message: &'static str) -> LabelError {
    LabelError::UnsupportedSyntax { position, message }
}

trait SyntaxNodeRange {
    fn range(&self) -> Range<usize>;
}

impl SyntaxNodeRange for SyntaxNode {
    fn range(&self) -> Range<usize> {
        match self.span().get() {
            SpanKind::Range { range, .. } => range,
            _ => 0..self.len(),
        }
    }
}

fn synthesize_ranges(root: &mut SyntaxNode, source_len: usize) -> Result<(), LabelError> {
    let mapper = RangeMapper::new([0..source_len]).map_err(|message| LabelError::Engine {
        start: 0,
        end: source_len,
        message: message.to_string(),
    })?;
    root.synthesize_mapped(scratch_file_id(), &mapper)
        .map_err(|message| LabelError::Engine {
            start: 0,
            end: source_len,
            message: message.to_string(),
        })
}

fn scratch_file_id() -> crate::typst_syntax::FileId {
    RootedPath::new(
        VirtualRoot::Project,
        VirtualPath::new("avenger-typst-label-line.typ").expect("static virtual path is valid"),
    )
    .intern()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> ParsedLine {
        parse_line(source).unwrap()
    }

    fn parse_with_params(source: &str, params: &LabelParams) -> ParsedLine {
        parse_line_with_params(source, params).unwrap()
    }

    #[test]
    fn parses_plain_and_math_spans() {
        let line = parse("Price \\$7, score $R^2$ = 0.94");

        assert_eq!(line.nodes.len(), 3);
        assert!(
            matches!(&line.nodes[0], LineNode::Plain(plain) if plain.text == "Price $7, score ")
        );
        assert!(matches!(&line.nodes[1], LineNode::Math(math) if math.source == "R^2"));
        assert!(matches!(&line.nodes[2], LineNode::Plain(plain) if plain.text == " = 0.94"));
    }

    #[test]
    fn canonical_typst_unmatched_dollar_errors() {
        let err = parse_line("cost $5").unwrap_err();
        assert!(matches!(err, LabelError::Syntax { .. }));
    }

    #[test]
    fn parses_static_text_spans() {
        let line = parse("This is #underline[important] and #strike[old]");

        assert_eq!(line.nodes.len(), 4);
        assert!(matches!(
            &line.nodes[1],
            LineNode::TextSpan(span)
                if span.kind == TextMarkupKind::Underline
                    && matches!(&span.body[..], [LineNode::Plain(plain)] if plain.text == "important")
        ));
        assert!(matches!(
            &line.nodes[3],
            LineNode::TextSpan(span)
                if span.kind == TextMarkupKind::Strike
                    && matches!(&span.body[..], [LineNode::Plain(plain)] if plain.text == "old")
        ));
    }

    #[test]
    fn parses_embedded_code_identifier_as_param_ref() {
        let line = parse("Series #series_name");

        assert_eq!(line.nodes.len(), 2);
        assert!(matches!(&line.nodes[0], LineNode::Plain(plain) if plain.text == "Series "));
        assert!(matches!(
            &line.nodes[1],
            LineNode::Param(param)
                if param.name == "series_name"
                    && param.byte_range == (7..19)
        ));
    }

    #[test]
    fn parses_nested_static_text_spans() {
        let line = parse("#underline[important #super[2]]");

        let LineNode::TextSpan(span) = &line.nodes[0] else {
            panic!("expected outer span");
        };
        assert_eq!(span.kind, TextMarkupKind::Underline);
        assert_eq!(span.body.len(), 2);
        assert!(
            matches!(&span.body[1], LineNode::TextSpan(inner) if inner.kind == TextMarkupKind::Superscript)
        );
    }

    #[test]
    fn parses_script_text_options() {
        let line = parse(
            "#super(typographic: false, baseline: -0.25em, size: 0.7em)[N] \
             #sub(typographic: false, baseline: 2pt, size: 8pt)[2]",
        );

        assert_eq!(line.nodes.len(), 3);
        assert!(matches!(&line.nodes[0], LineNode::TextSpan(span)
            if span.kind == TextMarkupKind::Superscript
                && !span.options.script.typographic
                && span.options.script.baseline == Some(DecorationLength::Em(-0.25))
                && span.options.script.size == Some(DecorationLength::Em(0.7))
                && matches!(&span.body[..], [LineNode::Plain(plain)] if plain.text == "N")
        ));
        assert!(matches!(&line.nodes[2], LineNode::TextSpan(span)
            if span.kind == TextMarkupKind::Subscript
                && !span.options.script.typographic
                && span.options.script.baseline == Some(DecorationLength::Pt(2.0))
                && span.options.script.size == Some(DecorationLength::Pt(8.0))
                && matches!(&span.body[..], [LineNode::Plain(plain)] if plain.text == "2")
        ));
    }

    #[test]
    fn rejects_unknown_script_option() {
        let err = parse_line("#super(foo: true)[x]").unwrap_err();
        assert_eq!(
            err,
            LabelError::UnsupportedSyntax {
                position: 7,
                message: "unsupported script option"
            }
        );
    }

    #[test]
    fn parses_case_transform_static_text() {
        let line = parse("#lower[MiXeD #sym.arrow.r] #upper(\"loud\")");

        assert_eq!(line.nodes.len(), 3);
        assert!(matches!(&line.nodes[0], LineNode::TextSpan(span)
            if span.kind == TextMarkupKind::Lower
                && matches!(&span.body[..], [LineNode::Plain(_), LineNode::Symbol(_)])
        ));
        assert!(matches!(&line.nodes[2], LineNode::TextSpan(span)
            if span.kind == TextMarkupKind::Upper
                && matches!(&span.body[..], [LineNode::Plain(plain)] if plain.text == "loud")
        ));
    }

    #[test]
    fn parses_smallcaps_static_text() {
        let line = parse("#smallcaps[Smallcaps] #smallcaps(all: true)[UNICEF]");

        assert_eq!(line.nodes.len(), 3);
        assert!(matches!(&line.nodes[0], LineNode::TextSpan(span)
            if span.kind == TextMarkupKind::Smallcaps
                && !span.options.smallcaps.all
                && matches!(&span.body[..], [LineNode::Plain(plain)] if plain.text == "Smallcaps")
        ));
        assert!(matches!(&line.nodes[2], LineNode::TextSpan(span)
            if span.kind == TextMarkupKind::Smallcaps
                && span.options.smallcaps.all
                && matches!(&span.body[..], [LineNode::Plain(plain)] if plain.text == "UNICEF")
        ));
    }

    #[test]
    fn parses_emph_and_strong_static_text() {
        let line = parse("_Emph_ *Strong* #emph[call] #strong(delta: 150)[mild]");

        assert_eq!(line.nodes.len(), 7);
        assert!(matches!(&line.nodes[0], LineNode::TextSpan(span)
            if span.kind == TextMarkupKind::Emph
                && matches!(&span.body[..], [LineNode::Plain(plain)] if plain.text == "Emph")
        ));
        assert!(matches!(&line.nodes[2], LineNode::TextSpan(span)
            if span.kind == TextMarkupKind::Strong
                && span.options.strong.delta == 300
                && matches!(&span.body[..], [LineNode::Plain(plain)] if plain.text == "Strong")
        ));
        assert!(matches!(&line.nodes[4], LineNode::TextSpan(span)
            if span.kind == TextMarkupKind::Emph
                && matches!(&span.body[..], [LineNode::Plain(plain)] if plain.text == "call")
        ));
        assert!(matches!(&line.nodes[6], LineNode::TextSpan(span)
            if span.kind == TextMarkupKind::Strong
                && span.options.strong.delta == 150
                && matches!(&span.body[..], [LineNode::Plain(plain)] if plain.text == "mild")
        ));
    }

    #[test]
    fn parses_inline_raw_static_text() {
        let line = parse("Use `x # y` and #raw(\"z * w\")");

        assert_eq!(line.nodes.len(), 4);
        let LineNode::TextSpan(span) = &line.nodes[1] else {
            panic!("expected backtick raw span");
        };
        assert_eq!(span.kind, TextMarkupKind::Raw);
        assert_eq!(span.byte_range, 4..11);
        assert_eq!(span.body_range, 5..10);
        assert!(matches!(&span.body[..], [LineNode::Plain(plain)]
            if plain.text == "x # y" && plain.byte_range == (5..10)
        ));
        assert!(matches!(&line.nodes[3], LineNode::TextSpan(span)
            if span.kind == TextMarkupKind::Raw
                && matches!(&span.body[..], [LineNode::Plain(plain)] if plain.text == "z * w")
        ));
    }

    #[test]
    fn rejects_raw_block_and_highlighting() {
        let block = parse_line("```typ\nlet x = 1\n```").unwrap_err();
        assert_eq!(
            block,
            LabelError::UnsupportedSyntax {
                position: 0,
                message: "raw block labels are not supported"
            }
        );

        let highlighted = parse_line("```typ let x = 1```").unwrap_err();
        assert_eq!(
            highlighted,
            LabelError::UnsupportedSyntax {
                position: 0,
                message: "raw syntax highlighting is not supported"
            }
        );
    }

    #[test]
    fn parses_named_emoji_aliases() {
        let line = parse("Revenue #emoji.rocket #emoji.chart.up");

        assert_eq!(line.nodes.len(), 4);
        assert!(
            matches!(&line.nodes[1], LineNode::Emoji(alias) if alias.name == "rocket" && alias.emoji == "🚀")
        );
        assert!(
            matches!(&line.nodes[3], LineNode::Emoji(alias) if alias.name == "chart.up" && alias.emoji == "📈")
        );
    }

    #[test]
    fn parses_named_symbol_aliases() {
        let line = parse("Flow #sym.arrow.r target #sym.gt.eq.not");

        assert_eq!(line.nodes.len(), 4);
        assert!(
            matches!(&line.nodes[1], LineNode::Symbol(alias) if alias.name == "arrow.r" && alias.text == "→")
        );
        assert!(
            matches!(&line.nodes[3], LineNode::Symbol(alias) if alias.name == "gt.eq.not" && alias.text == "≱")
        );
    }

    #[test]
    fn parses_decoration_options() {
        let line = parse(
            "#underline(stroke: 1.5pt + red, offset: 2pt, extent: 3pt, background: true, evade: false)[important]",
        );

        let LineNode::TextSpan(span) = &line.nodes[0] else {
            panic!("expected text span");
        };
        assert_eq!(span.kind, TextMarkupKind::Underline);
        assert_eq!(
            span.options.decoration.stroke.paint,
            Some(Color::rgba(1.0, 0.0, 0.0, 1.0))
        );
        assert_eq!(
            span.options.decoration.stroke.thickness,
            Some(DecorationLength::Pt(1.5))
        );
        assert_eq!(span.options.decoration.stroke.line_cap, None);
        assert_eq!(
            span.options.decoration.offset,
            Some(DecorationLength::Pt(2.0))
        );
        assert_eq!(span.options.decoration.extent, DecorationLength::Pt(3.0));
        assert!(span.options.decoration.background);
        assert_eq!(span.options.decoration.evade, Some(false));
    }

    #[test]
    fn parses_non_stroke_decoration_option_params() {
        let mut params = LabelParams::new();
        params.insert(
            "underline_offset".to_string(),
            LabelParamValue::Str("2pt".to_string()),
        );
        params.insert(
            "underline_extent".to_string(),
            LabelParamValue::Str("-0.5em".to_string()),
        );
        params.insert(
            "underline_background".to_string(),
            LabelParamValue::Bool(true),
        );
        params.insert("underline_evade".to_string(), LabelParamValue::Bool(false));

        let line = parse_with_params(
            "#underline(offset: underline_offset, extent: underline_extent, background: underline_background, evade: underline_evade)[care]",
            &params,
        );

        let LineNode::TextSpan(span) = &line.nodes[0] else {
            panic!("expected text span");
        };
        assert_eq!(span.kind, TextMarkupKind::Underline);
        assert_eq!(
            span.options.decoration.offset,
            Some(DecorationLength::Pt(2.0))
        );
        assert_eq!(span.options.decoration.extent, DecorationLength::Em(-0.5));
        assert!(span.options.decoration.background);
        assert_eq!(span.options.decoration.evade, Some(false));
    }

    #[test]
    fn parses_script_smallcaps_and_strong_option_params() {
        let mut params = LabelParams::new();
        params.insert("use_typographic".to_string(), LabelParamValue::Bool(false));
        params.insert(
            "script_baseline".to_string(),
            LabelParamValue::Str("-0.25em".to_string()),
        );
        params.insert(
            "script_size".to_string(),
            LabelParamValue::Str("8pt".to_string()),
        );
        params.insert("all_caps".to_string(), LabelParamValue::Bool(true));
        params.insert("weight_delta".to_string(), LabelParamValue::Int(150));

        let line = parse_with_params(
            "#super(typographic: use_typographic, baseline: script_baseline, size: script_size)[N] \
             #smallcaps(all: all_caps)[UNICEF] \
             #strong(delta: weight_delta)[bold]",
            &params,
        );

        assert_eq!(line.nodes.len(), 5);
        assert!(matches!(&line.nodes[0], LineNode::TextSpan(span)
            if span.kind == TextMarkupKind::Superscript
                && !span.options.script.typographic
                && span.options.script.baseline == Some(DecorationLength::Em(-0.25))
                && span.options.script.size == Some(DecorationLength::Pt(8.0))
        ));
        assert!(matches!(&line.nodes[2], LineNode::TextSpan(span)
            if span.kind == TextMarkupKind::Smallcaps
                && span.options.smallcaps.all
        ));
        assert!(matches!(&line.nodes[4], LineNode::TextSpan(span)
            if span.kind == TextMarkupKind::Strong
                && span.options.strong.delta == 150
        ));
    }

    #[test]
    fn rejects_invalid_non_stroke_option_param_casts() {
        let mut params = LabelParams::new();
        params.insert("badlength".to_string(), LabelParamValue::Bool(true));

        let err =
            parse_line_with_params("#underline(offset: badlength)[care]", &params).unwrap_err();

        assert_eq!(
            err,
            LabelError::UnsupportedSyntax {
                position: 11,
                message: "label parameter cannot be cast to decoration length"
            }
        );
    }

    #[test]
    fn rejects_unsupported_decoration_options() {
        let err = parse_line("#strike(evade: false)[old]").unwrap_err();

        assert_eq!(
            err,
            LabelError::UnsupportedSyntax {
                position: 8,
                message: "strike does not support evade"
            }
        );

        let err = parse_line("#underline(stroke: (miter-limit: 2))[group]").unwrap_err();

        assert_eq!(
            err,
            LabelError::UnsupportedSyntax {
                position: 20,
                message: "unsupported stroke dictionary field"
            }
        );
    }

    #[test]
    fn parses_stroke_cap_join_and_dash() {
        let line =
            parse("#underline(stroke: (cap: \"round\", join: \"bevel\", dash: \"dotted\"))[x]");

        let LineNode::TextSpan(span) = &line.nodes[0] else {
            panic!("expected text span");
        };
        assert_eq!(
            span.options.decoration.stroke.line_cap,
            Some(StrokeCap::Round)
        );
        assert_eq!(
            span.options.decoration.stroke.line_join,
            Some(StrokeJoin::Bevel)
        );
        assert_eq!(
            span.options
                .decoration
                .stroke
                .dash
                .as_ref()
                .map(|dash| dash.array.len()),
            Some(2)
        );
    }

    #[test]
    fn rejects_unknown_hash_commands() {
        let err = parse_line("#let x = 1").unwrap_err();

        assert!(matches!(
            err,
            LabelError::UnsupportedSyntax {
                position: 0,
                message: "unsupported static text command"
            }
        ));
    }

    #[test]
    fn rejects_unknown_emoji_aliases() {
        let err = parse_line("#emoji.not.real").unwrap_err();

        assert_eq!(
            err,
            LabelError::UnsupportedSyntax {
                position: 0,
                message: "unknown emoji alias"
            }
        );
    }

    #[test]
    fn rejects_unknown_symbol_aliases() {
        let err = parse_line("#sym.not.real").unwrap_err();

        assert_eq!(
            err,
            LabelError::UnsupportedSyntax {
                position: 0,
                message: "unknown symbol alias"
            }
        );
    }
}
