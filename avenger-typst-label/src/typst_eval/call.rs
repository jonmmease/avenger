use std::ops::Range;

use crate::typst_diag::LabelError;
use crate::typst_label::{LabelParamValue, LabelParams};
use crate::typst_library::Color;
use crate::typst_library::math::item as math_item;
use crate::typst_library::math::item::{
    MathAccent, MathArg, MathAttach, MathCall, MathCallOptions, MathCancel, MathCancelAngle,
    MathCancelLength, MathCancelOptions, MathDelimitedSize, MathFraction, MathFractionStyle,
    MathGroup, MathIdentifier, MathNode, MathOperator, MathSpace, MathStretchSize,
};
use crate::typst_library::text::content::{
    DecorationDash, DecorationDashLength, DecorationLength, DecorationStroke, TextMarkupKind,
    TextMarkupOptions,
};
use crate::typst_svg::{StrokeCap, StrokeJoin};

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

pub(crate) fn is_base_math_call_name(name: &str) -> bool {
    matches!(
        name,
        "frac"
            | "sqrt"
            | "root"
            | "binom"
            | "abs"
            | "norm"
            | "floor"
            | "ceil"
            | "round"
            | "lr"
            | "mid"
            | "class"
            | "underline"
            | "overline"
            | "bb"
            | "cal"
            | "frak"
            | "sans"
            | "mono"
            | "serif"
            | "scr"
            | "upright"
            | "italic"
            | "bold"
            | "display"
            | "inline"
            | "script"
            | "sscript"
            | "stretch"
    )
}

pub(crate) fn is_builtin_math_control_name(name: &str) -> bool {
    matches!(name, "op" | "attach" | "cancel" | "scripts" | "limits")
}

pub(crate) fn is_retained_math_name(
    name: &str,
    has_predefined_operator: impl Fn(&str) -> bool,
    has_named_accent: impl Fn(&str) -> bool,
    has_named_symbol: impl Fn(&str) -> bool,
) -> bool {
    is_builtin_math_control_name(name)
        || is_math_call_name(name, has_predefined_operator, has_named_accent)
        || is_unsupported_math_table_call_name(name)
        || has_named_symbol(name)
}

pub(crate) fn is_math_call_name(
    name: &str,
    has_predefined_operator: impl Fn(&str) -> bool,
    has_named_accent: impl Fn(&str) -> bool,
) -> bool {
    is_base_math_call_name(name)
        || has_predefined_operator(name)
        || is_math_accent_call_name(name, has_named_accent)
        || is_math_delimiter_symbol_call_name(name)
}

pub(crate) fn is_unsupported_math_table_call_name(name: &str) -> bool {
    matches!(name, "mat" | "vec" | "cases")
}

pub(crate) fn is_math_size_call_name(name: &str) -> bool {
    matches!(name, "display" | "inline" | "script" | "sscript")
}

pub(crate) fn is_math_delimiter_helper_call_name(name: &str) -> bool {
    matches!(name, "abs" | "norm" | "floor" | "ceil" | "round")
}

pub(crate) fn is_math_delimiter_symbol_call_name(name: &str) -> bool {
    matches!(
        name,
        "ceil.l" | "floor.l" | "paren.l" | "brace.l" | "bracket.l" | "chevron.l" | "bar.double"
    )
}

pub(crate) fn is_math_accent_call_name(name: &str, named_accent: impl Fn(&str) -> bool) -> bool {
    name == "accent" || named_accent(name)
}

pub(crate) fn parse_text_markup_option(
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

pub(crate) trait MathCallLoweringContext<'a> {
    fn source(&self) -> &'a str;
    fn offset(&self) -> usize;
    fn lower_math_expr(&mut self, expr: typst_ast::Expr<'a>) -> Result<Vec<MathNode>, LabelError>;
    fn lower_math_expr_as_single(
        &mut self,
        expr: typst_ast::Expr<'a>,
    ) -> Result<MathNode, LabelError>;
    fn named_math_symbol(&self, name: &str) -> Option<&'static str>;
    fn named_accent_char(&self, name: &str) -> Option<char>;
    fn normalize_accent_text(&self, value: &str) -> Option<char>;
    fn predefined_operator_text(&self, name: &str) -> Option<&'static str>;
}

pub(crate) fn lower_math_call<'a>(
    call: typst_ast::MathCall<'a>,
    ctx: &mut impl MathCallLoweringContext<'a>,
) -> Result<Vec<MathNode>, LabelError> {
    let offset = ctx.offset();
    let name = math_access_name(call.callee());
    let range = offset_range(call.to_untyped().range(), offset);
    if is_unsupported_math_table_call_name(&name) {
        return Err(unsupported(
            range.start,
            "matrix/table math is not supported in Avenger Typst subset",
        ));
    }

    if name == "op" {
        let (name, args) = lower_math_op_call_args(call.args(), range.start, ctx)?;
        return Ok(vec![MathNode::Call(MathCall {
            name,
            args,
            options: MathCallOptions::default(),
            byte_range: range,
        })]);
    }

    if name == "frac" {
        return lower_math_frac_call(call.args(), range, ctx);
    }

    if name == "attach" {
        return lower_math_attach_call(call.args(), range, ctx);
    }

    if name == "cancel" {
        return lower_math_cancel_call(call.args(), range, ctx);
    }

    if is_math_accent_call_name(&name, |name| ctx.named_accent_char(name).is_some()) {
        return lower_math_accent_call(&name, call.args(), range, ctx);
    }

    if name == "scripts" || name == "limits" {
        let (name, args) =
            lower_math_attachment_mode_call_args(&name, call.args(), range.start, ctx)?;
        return Ok(vec![MathNode::Call(MathCall {
            name,
            args,
            options: MathCallOptions::default(),
            byte_range: range,
        })]);
    }

    if is_math_size_call_name(&name) {
        let args = lower_math_size_call_args(call.args(), range.start, ctx)?;
        return Ok(vec![MathNode::Call(MathCall {
            name,
            args,
            options: MathCallOptions::default(),
            byte_range: range,
        })]);
    }

    if retained_math_call_name(&name, ctx) {
        let (args, options) = if name == "lr"
            || is_math_delimiter_helper_call_name(&name)
            || is_math_delimiter_symbol_call_name(&name)
        {
            lower_math_delimited_call_args(&name, call.args(), range.start, ctx)?
        } else if name == "stretch" {
            lower_math_stretch_call_args(call.args(), range.start, ctx)?
        } else {
            (
                lower_math_call_args(call.args(), ctx)?,
                MathCallOptions::default(),
            )
        };
        if name == "class" {
            validate_math_class_call_args(&args, range.start)?;
        } else if name == "binom" {
            validate_math_binom_call_args(&args, range.start)?;
        } else if name == "mid" {
            validate_math_mid_call_args(&args, range.start)?;
        }
        return Ok(vec![MathNode::Call(MathCall {
            name,
            args,
            options,
            byte_range: range,
        })]);
    }

    let mut nodes = vec![MathNode::Identifier(MathIdentifier {
        symbol: ctx.named_math_symbol(&name),
        name,
        byte_range: offset_range(call.callee().to_untyped().range(), offset),
    })];
    let args_range = offset_range(call.args().to_untyped().range(), offset);
    let body = lower_math_args_as_group_body(call.args(), ctx)?;
    nodes.push(MathNode::Group(MathGroup {
        left: '(',
        right: ')',
        body,
        byte_range: args_range,
    }));
    Ok(nodes)
}

fn lower_math_frac_call<'a>(
    args: typst_ast::MathArgs<'a>,
    range: std::ops::Range<usize>,
    ctx: &mut impl MathCallLoweringContext<'a>,
) -> Result<Vec<MathNode>, LabelError> {
    let source = ctx.source();
    let offset = ctx.offset();
    let mut numerator = None;
    let mut denominator = None;
    let mut style = MathFractionStyle::Vertical;
    let mut saw_style = false;

    for item in args.arg_items() {
        if item.ends_in_semicolon {
            return Err(unsupported(
                item.arg.to_untyped().range().end + offset,
                "semicolon math arguments are not supported in Avenger Typst subset",
            ));
        }
        match item.arg {
            typst_ast::Arg::Pos(expr) => {
                let byte_range = offset_range(expr.to_untyped().range(), offset);
                let nodes = ctx.lower_math_expr(expr)?;
                if numerator.is_none() {
                    numerator = Some((nodes, byte_range));
                } else if denominator.is_none() {
                    denominator = Some((nodes, byte_range));
                } else {
                    return Err(unsupported(
                        expr.to_untyped().range().start + offset,
                        "frac math expects numerator and denominator arguments",
                    ));
                }
            }
            typst_ast::Arg::Named(named) => {
                let position = named_argument_position(named, source, offset);
                match named.name().as_str() {
                    "style" => {
                        if saw_style {
                            return Err(unsupported(position, "duplicate frac style option"));
                        }
                        style = parse_math_fraction_style(named.expr(), position)?;
                        saw_style = true;
                    }
                    _ => return Err(unsupported(position, "unsupported frac option")),
                }
            }
            typst_ast::Arg::Spread(spread) => {
                return Err(unsupported(
                    spread.to_untyped().range().start + offset,
                    "spread math arguments are not supported in Avenger Typst subset",
                ));
            }
        }
    }

    let (numerator, numerator_range) = numerator.ok_or_else(|| {
        unsupported(
            range.start,
            "frac math expects numerator and denominator arguments",
        )
    })?;
    let (denominator, denominator_range) = denominator.ok_or_else(|| {
        unsupported(
            range.start,
            "frac math expects numerator and denominator arguments",
        )
    })?;
    if numerator.is_empty() || denominator.is_empty() {
        return Err(unsupported(
            range.start,
            "frac math numerator and denominator must not be empty",
        ));
    }

    Ok(vec![MathNode::Fraction(MathFraction {
        numerator,
        denominator,
        style,
        slash_range: numerator_range.end..denominator_range.start,
        byte_range: range,
    })])
}

fn lower_math_attach_call<'a>(
    args: typst_ast::MathArgs<'a>,
    range: std::ops::Range<usize>,
    ctx: &mut impl MathCallLoweringContext<'a>,
) -> Result<Vec<MathNode>, LabelError> {
    let source = ctx.source();
    let offset = ctx.offset();
    let mut base = None;
    let mut top = None;
    let mut bottom = None;
    let mut top_left = None;
    let mut top_right = None;
    let mut bottom_left = None;
    let mut bottom_right = None;

    for item in args.arg_items() {
        if item.ends_in_semicolon {
            return Err(unsupported(
                item.arg.to_untyped().range().end + offset,
                "semicolon math arguments are not supported in Avenger Typst subset",
            ));
        }
        match item.arg {
            typst_ast::Arg::Pos(expr) => {
                if base.is_some() {
                    return Err(unsupported(
                        expr.to_untyped().range().start + offset,
                        "attach expects one base argument",
                    ));
                }
                base = Some(ctx.lower_math_expr_as_single(expr)?);
            }
            typst_ast::Arg::Named(named) => {
                let name = named.name().as_str();
                let slot = match name {
                    "t" => &mut top,
                    "b" => &mut bottom,
                    "tl" => &mut top_left,
                    "tr" => &mut top_right,
                    "bl" => &mut bottom_left,
                    "br" => &mut bottom_right,
                    _ => {
                        return Err(unsupported(
                            named_argument_position(named, source, offset),
                            "unsupported attach option",
                        ));
                    }
                };
                if slot.is_some() {
                    return Err(unsupported(
                        named_argument_position(named, source, offset),
                        "duplicate attach option",
                    ));
                }
                let expr = named.expr();
                let nodes = ctx.lower_math_expr(expr)?;
                if nodes.is_empty() {
                    return Err(unsupported(
                        expr.to_untyped().range().start + offset,
                        "attach option expects math content",
                    ));
                }
                *slot = Some(nodes);
            }
            typst_ast::Arg::Spread(spread) => {
                return Err(unsupported(
                    spread.to_untyped().range().start + offset,
                    "spread math arguments are not supported in Avenger Typst subset",
                ));
            }
        }
    }

    let Some(base) = base else {
        return Err(unsupported(range.start, "attach expects one base argument"));
    };
    Ok(vec![MathNode::Attach(MathAttach {
        base: Box::new(base),
        top,
        bottom,
        top_left,
        top_right,
        bottom_left,
        bottom_right,
        primes: 0,
        byte_range: range,
    })])
}

fn lower_math_delimited_call_args<'a>(
    name: &str,
    args: typst_ast::MathArgs<'a>,
    position: usize,
    ctx: &mut impl MathCallLoweringContext<'a>,
) -> Result<(Vec<MathArg>, MathCallOptions), LabelError> {
    let source = ctx.source();
    let offset = ctx.offset();
    let mut lowered = Vec::new();
    let mut delimiter_size = None;

    for item in args.arg_items() {
        if item.ends_in_semicolon {
            return Err(unsupported(
                item.arg.to_untyped().range().end + offset,
                "semicolon math arguments are not supported in Avenger Typst subset",
            ));
        }
        match item.arg {
            typst_ast::Arg::Pos(expr) => {
                let byte_range = offset_range(expr.to_untyped().range(), offset);
                lowered.push(MathArg {
                    nodes: ctx.lower_math_expr(expr)?,
                    byte_range,
                });
            }
            typst_ast::Arg::Named(named) => {
                let name_position = named_argument_position(named, source, offset);
                if named.name().as_str() != "size" {
                    let message = if name == "lr" {
                        "unsupported lr option"
                    } else {
                        "unsupported delimiter option"
                    };
                    return Err(unsupported(name_position, message));
                }
                if delimiter_size.is_some() {
                    return Err(unsupported(
                        name_position,
                        "duplicate delimiter size option",
                    ));
                }
                delimiter_size = Some(parse_math_delimited_size(
                    named.expr(),
                    source,
                    name_position,
                    "unsupported delimiter size value",
                )?);
            }
            typst_ast::Arg::Spread(spread) => {
                return Err(unsupported(
                    spread.to_untyped().range().start + offset,
                    "spread math arguments are not supported in Avenger Typst subset",
                ));
            }
        }
    }

    if lowered.is_empty() {
        return Err(unsupported(
            position,
            "delimiter call expects a body argument",
        ));
    }
    if lowered.len() != 1 {
        return Err(unsupported(
            position,
            "delimiter call expects exactly one body argument",
        ));
    }

    Ok((
        lowered,
        MathCallOptions {
            delimiter_size,
            ..MathCallOptions::default()
        },
    ))
}

fn lower_math_stretch_call_args<'a>(
    args: typst_ast::MathArgs<'a>,
    position: usize,
    ctx: &mut impl MathCallLoweringContext<'a>,
) -> Result<(Vec<MathArg>, MathCallOptions), LabelError> {
    let source = ctx.source();
    let offset = ctx.offset();
    let mut lowered = Vec::new();
    let mut stretch_size = None;

    for item in args.arg_items() {
        if item.ends_in_semicolon {
            return Err(unsupported(
                item.arg.to_untyped().range().end + offset,
                "semicolon math arguments are not supported in Avenger Typst subset",
            ));
        }
        match item.arg {
            typst_ast::Arg::Pos(expr) => {
                let byte_range = offset_range(expr.to_untyped().range(), offset);
                lowered.push(MathArg {
                    nodes: ctx.lower_math_expr(expr)?,
                    byte_range,
                });
            }
            typst_ast::Arg::Named(named) => {
                let name_position = named_argument_position(named, source, offset);
                if named.name().as_str() != "size" {
                    return Err(unsupported(name_position, "unsupported stretch option"));
                }
                if stretch_size.is_some() {
                    return Err(unsupported(name_position, "duplicate stretch size option"));
                }
                stretch_size = Some(parse_math_stretch_size(
                    named.expr(),
                    source,
                    name_position,
                    "unsupported stretch size value",
                )?);
            }
            typst_ast::Arg::Spread(spread) => {
                return Err(unsupported(
                    spread.to_untyped().range().start + offset,
                    "spread math arguments are not supported in Avenger Typst subset",
                ));
            }
        }
    }

    if lowered.is_empty() {
        return Err(unsupported(position, "stretch expects a body argument"));
    }
    if lowered.len() != 1 {
        return Err(unsupported(
            position,
            "stretch expects exactly one body argument",
        ));
    }

    Ok((
        lowered,
        MathCallOptions {
            stretch_size,
            ..MathCallOptions::default()
        },
    ))
}

fn lower_math_attachment_mode_call_args<'a>(
    name: &str,
    args: typst_ast::MathArgs<'a>,
    position: usize,
    ctx: &mut impl MathCallLoweringContext<'a>,
) -> Result<(String, Vec<MathArg>), LabelError> {
    let source = ctx.source();
    let offset = ctx.offset();
    let mut body = None;
    let mut limits_inline = true;
    for item in args.arg_items() {
        if item.ends_in_semicolon {
            return Err(unsupported(
                item.arg.to_untyped().range().end + offset,
                "semicolon math arguments are not supported in Avenger Typst subset",
            ));
        }
        match item.arg {
            typst_ast::Arg::Pos(expr) => {
                if body.is_some() {
                    return Err(unsupported(
                        expr.to_untyped().range().start + offset,
                        "math attachment mode call expects one body argument",
                    ));
                }
                let byte_range = offset_range(expr.to_untyped().range(), offset);
                body = Some(MathArg {
                    nodes: ctx.lower_math_expr(expr)?,
                    byte_range,
                });
            }
            typst_ast::Arg::Named(named) => {
                if name != "limits" || named.name().as_str() != "inline" {
                    let message = if name == "limits" {
                        "unsupported limits option"
                    } else {
                        "unsupported scripts option"
                    };
                    return Err(unsupported(
                        named_argument_position(named, source, offset),
                        message,
                    ));
                }
                let item_position = named.to_untyped().range().start + offset;
                limits_inline = parse_math_bool_literal_with_message(
                    named.expr(),
                    item_position,
                    "unsupported limits inline value",
                )?;
            }
            typst_ast::Arg::Spread(spread) => {
                return Err(unsupported(
                    spread.to_untyped().range().start + offset,
                    "spread math arguments are not supported in Avenger Typst subset",
                ));
            }
        }
    }

    let body = body.ok_or_else(|| {
        unsupported(
            position,
            "math attachment mode call expects one body argument",
        )
    })?;
    let lowered_name = if name == "limits" && !limits_inline {
        "limits_display".to_string()
    } else {
        name.to_string()
    };
    Ok((lowered_name, vec![body]))
}

fn lower_math_cancel_call<'a>(
    args: typst_ast::MathArgs<'a>,
    range: std::ops::Range<usize>,
    ctx: &mut impl MathCallLoweringContext<'a>,
) -> Result<Vec<MathNode>, LabelError> {
    let source = ctx.source();
    let offset = ctx.offset();
    let mut body = None;
    let mut options = MathCancelOptions::default();
    for item in args.arg_items() {
        if item.ends_in_semicolon {
            return Err(unsupported(
                item.arg.to_untyped().range().end + offset,
                "semicolon math arguments are not supported in Avenger Typst subset",
            ));
        }
        match item.arg {
            typst_ast::Arg::Pos(expr) => {
                if body.is_some() {
                    return Err(unsupported(
                        expr.to_untyped().range().start + offset,
                        "cancel math expects one body argument",
                    ));
                }
                body = Some(ctx.lower_math_expr(expr)?);
            }
            typst_ast::Arg::Named(named) => {
                let position = named_argument_position(named, source, offset);
                match named.name().as_str() {
                    "length" => {
                        options.length = parse_math_cancel_length(named.expr(), source, position)?;
                    }
                    "inverted" => {
                        options.inverted = parse_math_bool_literal_with_message(
                            named.expr(),
                            position,
                            "unsupported cancel inverted value",
                        )?;
                    }
                    "cross" => {
                        options.cross = parse_math_bool_literal_with_message(
                            named.expr(),
                            position,
                            "unsupported cancel cross value",
                        )?;
                    }
                    "angle" => {
                        options.angle = parse_math_cancel_angle(named.expr(), source, position)?;
                    }
                    "stroke" => {
                        options.stroke = parse_decoration_stroke(named.expr(), position)?;
                    }
                    _ => {
                        return Err(unsupported(position, "unsupported cancel option"));
                    }
                }
            }
            typst_ast::Arg::Spread(spread) => {
                return Err(unsupported(
                    spread.to_untyped().range().start + offset,
                    "spread math arguments are not supported in Avenger Typst subset",
                ));
            }
        }
    }
    let body =
        body.ok_or_else(|| unsupported(range.start, "cancel math expects one body argument"))?;
    if body.is_empty() {
        return Err(unsupported(
            range.start,
            "cancel math body must not be empty",
        ));
    }
    Ok(vec![MathNode::Cancel(MathCancel {
        body,
        options,
        byte_range: range,
    })])
}

fn lower_math_accent_call<'a>(
    name: &str,
    args: typst_ast::MathArgs<'a>,
    range: std::ops::Range<usize>,
    ctx: &mut impl MathCallLoweringContext<'a>,
) -> Result<Vec<MathNode>, LabelError> {
    let offset = ctx.offset();
    let source = ctx.source();
    let expected_positional = if name == "accent" { 2 } else { 1 };
    let mut lowered = Vec::new();
    let mut dotless = true;
    let mut saw_dotless = false;
    for item in args.arg_items() {
        if item.ends_in_semicolon {
            return Err(unsupported(
                item.arg.to_untyped().range().end + offset,
                "semicolon math arguments are not supported in Avenger Typst subset",
            ));
        }
        match item.arg {
            typst_ast::Arg::Pos(expr) => {
                if lowered.len() == expected_positional {
                    return Err(unsupported(
                        expr.to_untyped().range().start + offset,
                        "accent math received too many positional arguments",
                    ));
                }
                let byte_range = offset_range(expr.to_untyped().range(), offset);
                lowered.push(MathArg {
                    nodes: ctx.lower_math_expr(expr)?,
                    byte_range,
                });
            }
            typst_ast::Arg::Named(named) => {
                let position = named_argument_position(named, source, offset);
                match named.name().as_str() {
                    "dotless" => {
                        if saw_dotless {
                            return Err(unsupported(position, "duplicate accent dotless option"));
                        }
                        dotless = parse_math_bool_literal_with_message(
                            named.expr(),
                            position,
                            "unsupported accent dotless value",
                        )?;
                        saw_dotless = true;
                    }
                    "size" => {
                        return Err(unsupported(
                            position,
                            "accent size option is not supported yet",
                        ));
                    }
                    _ => return Err(unsupported(position, "unsupported accent option")),
                }
            }
            typst_ast::Arg::Spread(spread) => {
                return Err(unsupported(
                    spread.to_untyped().range().start + offset,
                    "spread math arguments are not supported in Avenger Typst subset",
                ));
            }
        }
    }
    if lowered.len() != expected_positional {
        let message = if name == "accent" {
            "accent math expects a base and accent"
        } else {
            "accent math expects one body argument"
        };
        return Err(unsupported(range.start, message));
    }
    let accent = if name == "accent" {
        accent_arg_char(&lowered[1], ctx)
            .ok_or_else(|| unsupported(lowered[1].byte_range.start, "unsupported accent value"))?
    } else {
        ctx.named_accent_char(name)
            .expect("accent call names should be prevalidated")
    };
    Ok(vec![MathNode::Accent(MathAccent {
        base: lowered.remove(0).nodes,
        accent,
        dotless,
        byte_range: range,
    })])
}

fn accent_arg_char<'a>(arg: &MathArg, ctx: &impl MathCallLoweringContext<'a>) -> Option<char> {
    let [node] = &arg.nodes[..] else {
        return None;
    };
    let text = match node {
        MathNode::StringLiteral(string) => string.text.as_str(),
        MathNode::Identifier(identifier) => identifier.symbol.unwrap_or(&identifier.name),
        MathNode::Operator(operator) => operator.operator.as_str(),
        MathNode::Shorthand(shorthand) => shorthand.replacement,
        MathNode::Text(text) => text.text.as_str(),
        _ => return None,
    };
    ctx.normalize_accent_text(text)
}

fn lower_math_op_call_args<'a>(
    args: typst_ast::MathArgs<'a>,
    position: usize,
    ctx: &mut impl MathCallLoweringContext<'a>,
) -> Result<(String, Vec<MathArg>), LabelError> {
    let source = ctx.source();
    let offset = ctx.offset();
    let mut body = None;
    let mut limits = false;
    for item in args.arg_items() {
        if item.ends_in_semicolon {
            return Err(unsupported(
                item.arg.to_untyped().range().end + offset,
                "semicolon math arguments are not supported in Avenger Typst subset",
            ));
        }
        match item.arg {
            typst_ast::Arg::Pos(expr) => {
                if body.is_some() {
                    return Err(unsupported(
                        expr.to_untyped().range().start + offset,
                        "op math expects one text argument",
                    ));
                }
                let byte_range = offset_range(expr.to_untyped().range(), offset);
                body = Some(MathArg {
                    nodes: ctx.lower_math_expr(expr)?,
                    byte_range,
                });
            }
            typst_ast::Arg::Named(named) => {
                if named.name().as_str() != "limits" {
                    return Err(unsupported(
                        named_argument_position(named, source, offset),
                        "unsupported op option",
                    ));
                }
                limits = parse_math_bool_literal_with_message(
                    named.expr(),
                    named.to_untyped().range().start + offset,
                    "unsupported op limits value",
                )?;
            }
            typst_ast::Arg::Spread(spread) => {
                return Err(unsupported(
                    spread.to_untyped().range().start + offset,
                    "spread math arguments are not supported in Avenger Typst subset",
                ));
            }
        }
    }

    let body = body.ok_or_else(|| unsupported(position, "op math expects one text argument"))?;
    let name = if limits { "op_limits" } else { "op" }.to_string();
    Ok((name, vec![body]))
}

fn lower_math_call_args<'a>(
    args: typst_ast::MathArgs<'a>,
    ctx: &mut impl MathCallLoweringContext<'a>,
) -> Result<Vec<MathArg>, LabelError> {
    let source = ctx.source();
    let offset = ctx.offset();
    let mut lowered = Vec::new();
    for item in args.arg_items() {
        if item.ends_in_semicolon {
            return Err(unsupported(
                item.arg.to_untyped().range().end + offset,
                "semicolon math arguments are not supported in Avenger Typst subset",
            ));
        }
        match item.arg {
            typst_ast::Arg::Pos(expr) => {
                let byte_range = offset_range(expr.to_untyped().range(), offset);
                lowered.push(MathArg {
                    nodes: ctx.lower_math_expr(expr)?,
                    byte_range,
                });
            }
            typst_ast::Arg::Named(named) => {
                return Err(unsupported(
                    named_argument_position(named, source, offset),
                    "named math arguments are not supported in Avenger Typst subset",
                ));
            }
            typst_ast::Arg::Spread(spread) => {
                return Err(unsupported(
                    spread.to_untyped().range().start + offset,
                    "spread math arguments are not supported in Avenger Typst subset",
                ));
            }
        }
    }
    Ok(lowered)
}

fn lower_math_size_call_args<'a>(
    args: typst_ast::MathArgs<'a>,
    position: usize,
    ctx: &mut impl MathCallLoweringContext<'a>,
) -> Result<Vec<MathArg>, LabelError> {
    let source = ctx.source();
    let offset = ctx.offset();
    let mut body = None;
    for item in args.arg_items() {
        if item.ends_in_semicolon {
            return Err(unsupported(
                item.arg.to_untyped().range().end + offset,
                "semicolon math arguments are not supported in Avenger Typst subset",
            ));
        }
        match item.arg {
            typst_ast::Arg::Pos(expr) => {
                if body.is_some() {
                    return Err(unsupported(
                        expr.to_untyped().range().start + offset,
                        "math size call expects one body argument",
                    ));
                }
                let byte_range = offset_range(expr.to_untyped().range(), offset);
                body = Some(MathArg {
                    nodes: ctx.lower_math_expr(expr)?,
                    byte_range,
                });
            }
            typst_ast::Arg::Named(named) => {
                if named.name().as_str() != "cramped" {
                    return Err(unsupported(
                        named_argument_position(named, source, offset),
                        "unsupported math size option",
                    ));
                }
                let item_position = named.to_untyped().range().start + offset;
                parse_math_bool_literal_with_message(
                    named.expr(),
                    item_position,
                    "unsupported math size cramped value",
                )?;
            }
            typst_ast::Arg::Spread(spread) => {
                return Err(unsupported(
                    spread.to_untyped().range().start + offset,
                    "spread math arguments are not supported in Avenger Typst subset",
                ));
            }
        }
    }
    body.map(|body| vec![body])
        .ok_or_else(|| unsupported(position, "math size call expects one body argument"))
}

fn lower_math_args_as_group_body<'a>(
    args: typst_ast::MathArgs<'a>,
    ctx: &mut impl MathCallLoweringContext<'a>,
) -> Result<Vec<MathNode>, LabelError> {
    let source = ctx.source();
    let offset = ctx.offset();
    let mut body = Vec::new();
    for item in args.content_items() {
        match item {
            typst_ast::MathArgItem::Arg(typst_ast::Arg::Pos(expr)) => {
                body.extend(ctx.lower_math_expr(expr)?);
            }
            typst_ast::MathArgItem::Arg(typst_ast::Arg::Named(named)) => {
                return Err(unsupported(
                    named_argument_position(named, source, offset),
                    "named math arguments are not supported in Avenger Typst subset",
                ));
            }
            typst_ast::MathArgItem::Arg(typst_ast::Arg::Spread(spread)) => {
                return Err(unsupported(
                    spread.to_untyped().range().start + offset,
                    "spread math arguments are not supported in Avenger Typst subset",
                ));
            }
            typst_ast::MathArgItem::Space(space) => {
                body.push(MathNode::Space(MathSpace {
                    byte_range: offset_range(space.to_untyped().range(), offset),
                }));
            }
            typst_ast::MathArgItem::Comma(_, node) => {
                body.push(MathNode::Operator(MathOperator {
                    operator: ",".to_string(),
                    byte_range: offset_range(node.range(), offset),
                }));
            }
            typst_ast::MathArgItem::Semicolon(_, node) => {
                return Err(unsupported(
                    node.range().start + offset,
                    "semicolon math arguments are not supported in Avenger Typst subset",
                ));
            }
            typst_ast::MathArgItem::LeftParen(_, _) | typst_ast::MathArgItem::RightParen(_, _) => {}
        }
    }
    Ok(body)
}

fn retained_math_call_name<'a>(name: &str, ctx: &impl MathCallLoweringContext<'a>) -> bool {
    is_math_call_name(
        name,
        |name| ctx.predefined_operator_text(name).is_some(),
        |name| ctx.named_accent_char(name).is_some(),
    )
}

fn math_access_name(access: typst_ast::MathAccess<'_>) -> String {
    match access {
        typst_ast::MathAccess::MathIdent(ident) => ident.as_str().to_string(),
        typst_ast::MathAccess::MathFieldAccess(access) => {
            let mut name = math_access_name(access.target());
            name.push('.');
            name.push_str(access.field().as_str());
            name
        }
    }
}

fn offset_range(range: Range<usize>, offset: usize) -> Range<usize> {
    offset + range.start..offset + range.end
}

pub(crate) fn named_argument_position(
    named: typst_ast::Named<'_>,
    source: &str,
    offset: usize,
) -> usize {
    let range = named.to_untyped().range();
    source[range.clone()]
        .find(':')
        .map(|idx| offset + range.start + idx)
        .unwrap_or(offset + range.start)
}

pub(crate) fn parse_math_fraction_style(
    expr: typst_ast::Expr<'_>,
    position: usize,
) -> Result<MathFractionStyle, LabelError> {
    let style = match expr {
        typst_ast::Expr::Str(string) => string.get(),
        typst_ast::Expr::CodeBlock(block) => {
            let exprs = block.body().exprs().collect::<Vec<_>>();
            let [typst_ast::Expr::Str(string)] = &exprs[..] else {
                return Err(unsupported(position, "unsupported frac style value"));
            };
            string.get()
        }
        _ => return Err(unsupported(position, "unsupported frac style value")),
    };
    match style.as_str() {
        "vertical" => Ok(MathFractionStyle::Vertical),
        "skewed" => Ok(MathFractionStyle::Skewed),
        "horizontal" => Ok(MathFractionStyle::Horizontal),
        _ => Err(unsupported(position, "unsupported frac style value")),
    }
}

pub(crate) fn parse_math_cancel_length(
    expr: typst_ast::Expr<'_>,
    source: &str,
    position: usize,
) -> Result<MathCancelLength, LabelError> {
    let (value, unit) =
        parse_math_numeric_literal(expr, source, position, "unsupported cancel length value")?;
    let value = value as f32;
    match unit {
        typst_ast::Unit::Percent => Ok(MathCancelLength {
            relative: value / 100.0,
            absolute_em: 0.0,
        }),
        typst_ast::Unit::Em => Ok(MathCancelLength {
            relative: 0.0,
            absolute_em: value,
        }),
        _ => Err(unsupported(position, "unsupported cancel length value")),
    }
}

pub(crate) fn parse_math_cancel_angle(
    expr: typst_ast::Expr<'_>,
    source: &str,
    position: usize,
) -> Result<MathCancelAngle, LabelError> {
    if is_math_auto_literal(expr, source) {
        return Ok(MathCancelAngle::Auto);
    }
    let (value, unit) =
        parse_math_numeric_literal(expr, source, position, "unsupported cancel angle value")?;
    match unit {
        typst_ast::Unit::Deg => Ok(MathCancelAngle::Degrees(value as f32)),
        typst_ast::Unit::Rad => Ok(MathCancelAngle::Degrees((value as f32).to_degrees())),
        _ => Err(unsupported(position, "unsupported cancel angle value")),
    }
}

pub(crate) fn parse_math_delimited_size(
    expr: typst_ast::Expr<'_>,
    source: &str,
    position: usize,
    message: &'static str,
) -> Result<MathDelimitedSize, LabelError> {
    parse_math_relative_size(expr, source, position, message)
}

pub(crate) fn parse_math_stretch_size(
    expr: typst_ast::Expr<'_>,
    source: &str,
    position: usize,
    message: &'static str,
) -> Result<MathStretchSize, LabelError> {
    parse_math_relative_size(expr, source, position, message)
}

fn parse_math_relative_size(
    expr: typst_ast::Expr<'_>,
    source: &str,
    position: usize,
    message: &'static str,
) -> Result<math_item::MathRelativeSize, LabelError> {
    let (value, unit) = parse_math_numeric_literal(expr, source, position, message)?;
    let value = value as f32;
    match unit {
        typst_ast::Unit::Percent => Ok(math_item::MathRelativeSize {
            relative: value / 100.0,
            absolute_em: 0.0,
            absolute_pt: 0.0,
        }),
        typst_ast::Unit::Em => Ok(math_item::MathRelativeSize {
            relative: 0.0,
            absolute_em: value,
            absolute_pt: 0.0,
        }),
        typst_ast::Unit::Pt => Ok(math_item::MathRelativeSize {
            relative: 0.0,
            absolute_em: 0.0,
            absolute_pt: value,
        }),
        _ => Err(unsupported(position, message)),
    }
}

fn parse_math_numeric_literal(
    expr: typst_ast::Expr<'_>,
    source: &str,
    position: usize,
    message: &'static str,
) -> Result<(f64, typst_ast::Unit), LabelError> {
    let raw = source
        .get(expr.to_untyped().range())
        .unwrap_or_default()
        .trim();
    match expr {
        typst_ast::Expr::Numeric(value) => Ok(value.get()),
        typst_ast::Expr::CodeBlock(block) => {
            let exprs = block.body().exprs().collect::<Vec<_>>();
            if let [typst_ast::Expr::Numeric(value)] = &exprs[..] {
                Ok(value.get())
            } else {
                parse_raw_math_numeric_literal(raw).ok_or_else(|| unsupported(position, message))
            }
        }
        _ => parse_raw_math_numeric_literal(raw).ok_or_else(|| unsupported(position, message)),
    }
}

fn is_math_auto_literal(expr: typst_ast::Expr<'_>, source: &str) -> bool {
    let raw = source
        .get(expr.to_untyped().range())
        .unwrap_or_default()
        .trim()
        .trim_start_matches('#');
    match expr {
        typst_ast::Expr::Ident(ident) => ident.as_str() == "auto",
        typst_ast::Expr::CodeBlock(block) => {
            let exprs = block.body().exprs().collect::<Vec<_>>();
            matches!(&exprs[..], [typst_ast::Expr::Ident(ident)] if ident.as_str() == "auto")
                || raw == "auto"
        }
        _ => raw == "auto",
    }
}

fn parse_raw_math_numeric_literal(raw: &str) -> Option<(f64, typst_ast::Unit)> {
    let raw = raw.trim().trim_start_matches('#');
    let unit = ["deg", "rad", "em", "pt", "%"]
        .iter()
        .find(|unit| raw.ends_with(**unit))?;
    let value = raw[..raw.len() - unit.len()].trim().parse().ok()?;
    let unit = match *unit {
        "deg" => typst_ast::Unit::Deg,
        "rad" => typst_ast::Unit::Rad,
        "em" => typst_ast::Unit::Em,
        "pt" => typst_ast::Unit::Pt,
        "%" => typst_ast::Unit::Percent,
        _ => return None,
    };
    Some((value, unit))
}

pub(crate) fn parse_math_bool_literal_with_message(
    expr: typst_ast::Expr<'_>,
    position: usize,
    message: &'static str,
) -> Result<bool, LabelError> {
    match expr {
        typst_ast::Expr::Bool(value) => Ok(value.get()),
        typst_ast::Expr::CodeBlock(block) => {
            let exprs = block.body().exprs().collect::<Vec<_>>();
            let [typst_ast::Expr::Bool(value)] = &exprs[..] else {
                return Err(unsupported(position, message));
            };
            Ok(value.get())
        }
        _ => Err(unsupported(position, message)),
    }
}

pub(crate) fn validate_math_class_call_args(
    args: &[MathArg],
    position: usize,
) -> Result<(), LabelError> {
    let [class_arg, body_arg] = args else {
        return Err(unsupported(
            position,
            "class math expects a class name and body",
        ));
    };
    let [MathNode::StringLiteral(class)] = &class_arg.nodes[..] else {
        return Err(unsupported(
            class_arg.byte_range.start,
            "math class name must be a string literal",
        ));
    };
    if !is_supported_math_class_name(&class.text) {
        return Err(unsupported(
            class.byte_range.start,
            "unsupported math class name",
        ));
    }
    if body_arg.nodes.is_empty() {
        return Err(unsupported(
            body_arg.byte_range.start,
            "class math body must not be empty",
        ));
    }
    Ok(())
}

pub(crate) fn validate_math_binom_call_args(
    args: &[MathArg],
    position: usize,
) -> Result<(), LabelError> {
    if args.len() < 2 {
        return Err(unsupported(
            position,
            "binom math expects upper and at least one lower argument",
        ));
    }
    if args.iter().any(|arg| arg.nodes.is_empty()) {
        return Err(unsupported(
            position,
            "binom math arguments must not be empty",
        ));
    }
    Ok(())
}

pub(crate) fn validate_math_mid_call_args(
    args: &[MathArg],
    position: usize,
) -> Result<(), LabelError> {
    if args.is_empty() {
        return Err(unsupported(position, "mid expects a body argument"));
    }
    if args.len() != 1 {
        return Err(unsupported(
            position,
            "mid expects exactly one body argument",
        ));
    }
    Ok(())
}

fn is_supported_math_class_name(name: &str) -> bool {
    matches!(
        name,
        "normal"
            | "alphabetic"
            | "binary"
            | "unary"
            | "vary"
            | "relation"
            | "opening"
            | "closing"
            | "fence"
            | "punctuation"
            | "large"
    )
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
