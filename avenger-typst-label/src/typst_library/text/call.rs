//! Retained static text function schemas and literal option parsing.
//!
//! This module mirrors the small `typst-library` surface Avenger labels keep
//! for static text functions such as underline, overline, strike, sub, super,
//! smallcaps, emph, strong, raw, symbols, and emoji. The evaluator calls into
//! this module instead of owning function defaults itself.

use crate::label::LabelError;
use crate::typst_library::Color;
use crate::typst_library::foundations::{Dict, Scope, Value};
use crate::typst_library::text::content::{
    DecorationDash, DecorationDashLength, DecorationLength, DecorationStroke, TextMarkupKind,
    TextMarkupOptions,
};
use crate::typst_svg::{LineCap, LineJoin};

use crate::typst_syntax::ast::{self as typst_ast, AstNode};
use crate::typst_syntax::{SpanKind, SyntaxNode};

pub(crate) fn text_span_kind(name: &str) -> Option<TextMarkupKind> {
    match name {
        "underline" => Some(TextMarkupKind::Underline),
        "strike" => Some(TextMarkupKind::Strike),
        "overline" => Some(TextMarkupKind::Overline),
        "sub" => Some(TextMarkupKind::Subscript),
        "super" => Some(TextMarkupKind::Superscript),
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

pub(crate) fn named_color(name: &str) -> Option<Color> {
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

pub(crate) fn parse_text_markup_option(
    kind: TextMarkupKind,
    named: typst_ast::Named<'_>,
    params: &Scope,
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
    parse_decoration_stroke_with_params(expr, position, &Scope::default())
}

fn parse_decoration_stroke_with_params(
    expr: typst_ast::Expr<'_>,
    position: usize,
    params: &Scope,
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
    params: &Scope,
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
                        ..DecorationStroke::default()
                    },
                    item_position,
                )?;
            }
            "thickness" => {
                stroke.merge(
                    DecorationStroke {
                        thickness: Some(parse_length(named.expr(), item_position, params)?),
                        ..DecorationStroke::default()
                    },
                    item_position,
                )?;
            }
            "cap" => {
                stroke.merge(
                    DecorationStroke {
                        line_cap: Some(parse_line_cap(named.expr(), item_position, params)?),
                        ..DecorationStroke::default()
                    },
                    item_position,
                )?;
            }
            "join" => {
                stroke.merge(
                    DecorationStroke {
                        line_join: Some(parse_line_join(named.expr(), item_position, params)?),
                        ..DecorationStroke::default()
                    },
                    item_position,
                )?;
            }
            "dash" => {
                stroke.merge(
                    DecorationStroke {
                        dash: Some(parse_dash(named.expr(), item_position, params)?),
                        ..DecorationStroke::default()
                    },
                    item_position,
                )?;
            }
            "miter-limit" => {
                stroke.merge(
                    DecorationStroke {
                        miter_limit: Some(parse_miter_limit(named.expr(), item_position, params)?),
                        ..DecorationStroke::default()
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
    params: &Scope,
) -> Result<DecorationStroke, LabelError> {
    if let Some(value) = param_value_for_ident(expr, params) {
        return param_value_to_stroke_part(value, position);
    }
    if let Ok(paint) = parse_paint(expr, position, params) {
        return Ok(DecorationStroke {
            paint: Some(paint),
            ..DecorationStroke::default()
        });
    }
    if let Ok(thickness) = parse_length(expr, position, params) {
        return Ok(DecorationStroke {
            thickness: Some(thickness),
            ..DecorationStroke::default()
        });
    }
    if is_non_solid_paint_expr(expr) {
        return Err(unsupported(position, "unsupported decoration paint"));
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
        if let Some(paint) = other.paint
            && self.paint.replace(paint).is_some()
        {
            return Err(unsupported(position, "duplicate decoration stroke paint"));
        }
        if let Some(thickness) = other.thickness
            && self.thickness.replace(thickness).is_some()
        {
            return Err(unsupported(
                position,
                "duplicate decoration stroke thickness",
            ));
        }
        if let Some(line_cap) = other.line_cap
            && self.line_cap.replace(line_cap).is_some()
        {
            return Err(unsupported(position, "duplicate decoration stroke cap"));
        }
        if let Some(line_join) = other.line_join
            && self.line_join.replace(line_join).is_some()
        {
            return Err(unsupported(position, "duplicate decoration stroke join"));
        }
        if let Some(dash) = other.dash
            && self.dash.replace(dash).is_some()
        {
            return Err(unsupported(position, "duplicate decoration stroke dash"));
        }
        if let Some(miter_limit) = other.miter_limit
            && self.miter_limit.replace(miter_limit).is_some()
        {
            return Err(unsupported(
                position,
                "duplicate decoration stroke miter limit",
            ));
        }
        Ok(())
    }
}

fn parse_auto_or_length(
    expr: typst_ast::Expr<'_>,
    position: usize,
    params: &Scope,
) -> Result<Option<DecorationLength>, LabelError> {
    if let Some(Value::Str(value)) = param_value_for_ident(expr, params)
        && value.trim() == "auto"
    {
        return Ok(None);
    }
    match expr {
        typst_ast::Expr::Auto(_) => Ok(None),
        _ => parse_length(expr, position, params).map(Some),
    }
}

fn parse_length(
    expr: typst_ast::Expr<'_>,
    position: usize,
    params: &Scope,
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
    params: &Scope,
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
    params: &Scope,
    message: &'static str,
) -> Result<bool, LabelError> {
    if let Some(value) = param_value_for_ident(expr, params) {
        return match value {
            Value::Bool(value) => Ok(*value),
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
    params: &Scope,
    message: &'static str,
) -> Result<i64, LabelError> {
    if let Some(value) = param_value_for_ident(expr, params) {
        return match value {
            Value::Int(value) => Ok(*value),
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
    params: &Scope,
) -> Result<LineCap, LabelError> {
    let value = if let Some(value) = param_value_for_ident(expr, params) {
        param_value_to_string(value, position, "unsupported stroke cap value")?
    } else {
        let typst_ast::Expr::Str(value) = expr else {
            return Err(unsupported(position, "unsupported stroke cap value"));
        };
        value.get().to_string()
    };
    match value.as_str() {
        "butt" => Ok(LineCap::Butt),
        "round" => Ok(LineCap::Round),
        "square" => Ok(LineCap::Square),
        _ => Err(unsupported(position, "unsupported stroke cap value")),
    }
}

fn parse_line_join(
    expr: typst_ast::Expr<'_>,
    position: usize,
    params: &Scope,
) -> Result<LineJoin, LabelError> {
    let value = if let Some(value) = param_value_for_ident(expr, params) {
        param_value_to_string(value, position, "unsupported stroke join value")?
    } else {
        let typst_ast::Expr::Str(value) = expr else {
            return Err(unsupported(position, "unsupported stroke join value"));
        };
        value.get().to_string()
    };
    match value.as_str() {
        "bevel" => Ok(LineJoin::Bevel),
        "miter" => Ok(LineJoin::Miter),
        "round" => Ok(LineJoin::Round),
        _ => Err(unsupported(position, "unsupported stroke join value")),
    }
}

fn parse_dash(
    expr: typst_ast::Expr<'_>,
    position: usize,
    params: &Scope,
) -> Result<DecorationDash, LabelError> {
    if let Some(value) = param_value_for_ident(expr, params) {
        return param_value_to_dash(value, position);
    }
    match expr {
        typst_ast::Expr::Str(value) => named_dash(value.get().as_str())
            .ok_or_else(|| unsupported(position, "unsupported stroke dash value")),
        typst_ast::Expr::Array(array) => Ok(DecorationDash {
            array: parse_dash_array(array, position, params)?,
            phase: DecorationLength::default(),
        }),
        typst_ast::Expr::Dict(dict) => parse_dash_dict(dict, position, params),
        _ => Err(unsupported(position, "unsupported stroke dash value")),
    }
}

fn parse_dash_dict(
    dict: typst_ast::Dict<'_>,
    position: usize,
    params: &Scope,
) -> Result<DecorationDash, LabelError> {
    let mut array = None;
    let mut phase = DecorationLength::default();

    for item in dict.items() {
        let typst_ast::DictItem::Named(named) = item else {
            return Err(unsupported(
                position,
                "unsupported stroke dash dictionary item",
            ));
        };
        let item_position = named.to_untyped().range().start;
        match named.name().as_str() {
            "array" => {
                if array.is_some() {
                    return Err(unsupported(item_position, "duplicate stroke dash array"));
                }
                let typst_ast::Expr::Array(items) = named.expr() else {
                    return Err(unsupported(item_position, "unsupported stroke dash array"));
                };
                array = Some(parse_dash_array(items, item_position, params)?);
            }
            "phase" => {
                phase = parse_length(named.expr(), item_position, params)?;
            }
            _ => {
                return Err(unsupported(
                    item_position,
                    "unsupported stroke dash dictionary field",
                ));
            }
        }
    }

    Ok(DecorationDash {
        array: array.ok_or_else(|| unsupported(position, "missing stroke dash array"))?,
        phase,
    })
}

fn parse_dash_array(
    array: typst_ast::Array<'_>,
    position: usize,
    params: &Scope,
) -> Result<Vec<DecorationDashLength>, LabelError> {
    let mut lengths = Vec::new();
    for item in array.items() {
        let typst_ast::ArrayItem::Pos(expr) = item else {
            return Err(unsupported(position, "unsupported stroke dash array item"));
        };
        lengths.push(parse_dash_length(expr, position, params)?);
    }
    Ok(lengths)
}

fn parse_dash_length(
    expr: typst_ast::Expr<'_>,
    position: usize,
    params: &Scope,
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
    Some(DecorationDash {
        array,
        phase: DecorationLength::default(),
    })
}

fn parse_miter_limit(
    expr: typst_ast::Expr<'_>,
    position: usize,
    params: &Scope,
) -> Result<f32, LabelError> {
    if let Some(value) = param_value_for_ident(expr, params) {
        return param_value_to_miter_limit(value, position);
    }
    parse_unitless_f32_text(&expr.to_untyped().full_text(), position)
}

fn parse_unitless_f32_text(text: &str, position: usize) -> Result<f32, LabelError> {
    if text.chars().any(|ch| ch.is_ascii_alphabetic() || ch == '%') {
        return Err(unsupported(position, "unsupported stroke miter limit"));
    }
    let value = text
        .parse::<f32>()
        .map_err(|_| unsupported(position, "unsupported stroke miter limit"))?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(unsupported(position, "unsupported stroke miter limit"))
    }
}

fn parse_evade(
    expr: typst_ast::Expr<'_>,
    position: usize,
    params: &Scope,
) -> Result<Option<bool>, LabelError> {
    if let Some(value) = param_value_for_ident(expr, params) {
        return match value {
            Value::None => Ok(None),
            Value::Bool(value) => Ok(Some(*value)),
            Value::Str(value) if value == "auto" => Ok(None),
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
    params: &Scope,
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

fn is_non_solid_paint_expr(expr: typst_ast::Expr<'_>) -> bool {
    code_expr_name(expr).is_some_and(|name| {
        name == "gradient"
            || name.starts_with("gradient.")
            || name == "pattern"
            || name.starts_with("pattern.")
    })
}

fn code_expr_name(expr: typst_ast::Expr<'_>) -> Option<String> {
    match expr {
        typst_ast::Expr::Ident(ident) => Some(ident.as_str().to_string()),
        typst_ast::Expr::FieldAccess(access) => code_field_access_name(access),
        typst_ast::Expr::FuncCall(call) => code_expr_name(call.callee()),
        _ => None,
    }
}

fn code_field_access_name(access: typst_ast::FieldAccess<'_>) -> Option<String> {
    let mut name = code_expr_name(access.target())?;
    name.push('.');
    name.push_str(access.field().as_str());
    Some(name)
}

fn param_value_for_ident<'a>(expr: typst_ast::Expr<'_>, params: &'a Scope) -> Option<&'a Value> {
    let typst_ast::Expr::Ident(ident) = expr else {
        return None;
    };
    params.get(ident.as_str())
}

fn param_value_to_stroke(value: &Value, position: usize) -> Result<DecorationStroke, LabelError> {
    match value {
        Value::Str(value) => parse_stroke_literal(value, position),
        Value::Dict(dict) => param_dict_to_stroke(dict, position),
        _ => param_value_to_stroke_part(value, position),
    }
}

fn param_value_to_stroke_part(
    value: &Value,
    position: usize,
) -> Result<DecorationStroke, LabelError> {
    if let Ok(paint) = param_value_to_paint(value, position) {
        return Ok(DecorationStroke {
            paint: Some(paint),
            ..DecorationStroke::default()
        });
    }
    if let Ok(thickness) = param_value_to_length(value, position) {
        return Ok(DecorationStroke {
            thickness: Some(thickness),
            ..DecorationStroke::default()
        });
    }
    if matches!(value, Value::None) {
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
            ..DecorationStroke::default()
        });
    }
    if let Some(thickness) = parse_length_literal(raw, position)? {
        return Ok(DecorationStroke {
            thickness: Some(thickness),
            ..DecorationStroke::default()
        });
    }
    Err(unsupported(
        position,
        "label parameter cannot be cast to stroke",
    ))
}

fn param_dict_to_stroke(dict: &Dict, position: usize) -> Result<DecorationStroke, LabelError> {
    let mut stroke = DecorationStroke::default();
    for (name, value) in dict {
        match name.as_str() {
            "paint" => stroke.merge(
                DecorationStroke {
                    paint: Some(param_value_to_paint(value, position)?),
                    ..DecorationStroke::default()
                },
                position,
            )?,
            "thickness" => stroke.merge(
                DecorationStroke {
                    thickness: Some(param_value_to_length(value, position)?),
                    ..DecorationStroke::default()
                },
                position,
            )?,
            "cap" => stroke.merge(
                DecorationStroke {
                    line_cap: Some(parse_cap_literal(
                        &param_value_to_string(value, position, "unsupported stroke cap value")?,
                        position,
                    )?),
                    ..DecorationStroke::default()
                },
                position,
            )?,
            "join" => stroke.merge(
                DecorationStroke {
                    line_join: Some(parse_join_literal(
                        &param_value_to_string(value, position, "unsupported stroke join value")?,
                        position,
                    )?),
                    ..DecorationStroke::default()
                },
                position,
            )?,
            "dash" => stroke.merge(
                DecorationStroke {
                    dash: Some(param_value_to_dash(value, position)?),
                    ..DecorationStroke::default()
                },
                position,
            )?,
            "miter-limit" => stroke.merge(
                DecorationStroke {
                    miter_limit: Some(param_value_to_miter_limit(value, position)?),
                    ..DecorationStroke::default()
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

fn param_value_to_paint(value: &Value, position: usize) -> Result<Color, LabelError> {
    let Value::Str(value) = value else {
        return Err(unsupported(position, "unsupported decoration paint"));
    };
    named_color(value.trim()).ok_or_else(|| unsupported(position, "unsupported decoration paint"))
}

fn param_value_to_length(value: &Value, position: usize) -> Result<DecorationLength, LabelError> {
    let Value::Str(value) = value else {
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
    value: &Value,
    position: usize,
    message: &'static str,
) -> Result<String, LabelError> {
    match value {
        Value::Str(value) => Ok(value.clone()),
        _ => Err(unsupported(position, message)),
    }
}

fn param_value_to_dash(value: &Value, position: usize) -> Result<DecorationDash, LabelError> {
    match value {
        Value::Str(value) => named_dash(value.trim())
            .ok_or_else(|| unsupported(position, "unsupported stroke dash value")),
        Value::Array(values) => values
            .iter()
            .map(|value| match value {
                Value::Str(value) if value == "dot" => Ok(DecorationDashLength::LineWidth),
                _ => param_value_to_length(value, position).map(DecorationDashLength::Length),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|array| DecorationDash {
                array,
                phase: DecorationLength::default(),
            }),
        Value::Dict(dict) => param_dict_to_dash(dict, position),
        _ => Err(unsupported(position, "unsupported stroke dash value")),
    }
}

fn param_dict_to_dash(dict: &Dict, position: usize) -> Result<DecorationDash, LabelError> {
    let mut array = None;
    let mut phase = DecorationLength::default();
    for (name, value) in dict {
        match name.as_str() {
            "array" => {
                if array.is_some() {
                    return Err(unsupported(position, "duplicate stroke dash array"));
                }
                let Value::Array(values) = value else {
                    return Err(unsupported(position, "unsupported stroke dash array"));
                };
                array = Some(
                    values
                        .iter()
                        .map(|value| match value {
                            Value::Str(value) if value == "dot" => {
                                Ok(DecorationDashLength::LineWidth)
                            }
                            _ => param_value_to_length(value, position)
                                .map(DecorationDashLength::Length),
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                );
            }
            "phase" => phase = param_value_to_length(value, position)?,
            _ => {
                return Err(unsupported(
                    position,
                    "unsupported stroke dash dictionary field",
                ));
            }
        }
    }
    Ok(DecorationDash {
        array: array.ok_or_else(|| unsupported(position, "missing stroke dash array"))?,
        phase,
    })
}

fn param_value_to_miter_limit(value: &Value, position: usize) -> Result<f32, LabelError> {
    let number = match value {
        Value::Int(value) => *value as f32,
        Value::Float(value) => *value as f32,
        Value::Str(value) => value
            .trim()
            .parse::<f32>()
            .map_err(|_| unsupported(position, "unsupported stroke miter limit"))?,
        _ => return Err(unsupported(position, "unsupported stroke miter limit")),
    };
    if number.is_finite() {
        Ok(number)
    } else {
        Err(unsupported(position, "unsupported stroke miter limit"))
    }
}

fn parse_cap_literal(value: &str, position: usize) -> Result<LineCap, LabelError> {
    match value.trim() {
        "butt" => Ok(LineCap::Butt),
        "round" => Ok(LineCap::Round),
        "square" => Ok(LineCap::Square),
        _ => Err(unsupported(position, "unsupported stroke cap value")),
    }
}

fn parse_join_literal(value: &str, position: usize) -> Result<LineJoin, LabelError> {
    match value.trim() {
        "bevel" => Ok(LineJoin::Bevel),
        "miter" => Ok(LineJoin::Miter),
        "round" => Ok(LineJoin::Round),
        _ => Err(unsupported(position, "unsupported stroke join value")),
    }
}

fn unsupported(position: usize, message: &'static str) -> LabelError {
    LabelError::UnsupportedSyntax { position, message }
}

trait SyntaxNodeRange {
    fn range(&self) -> std::ops::Range<usize>;
}

impl SyntaxNodeRange for SyntaxNode {
    fn range(&self) -> std::ops::Range<usize> {
        match self.span().get() {
            SpanKind::Range { range, .. } => range,
            _ => 0..self.len(),
        }
    }
}
