use crate::typst_diag::LabelError;
use crate::typst_eval::call::{
    is_math_accent_call_name as is_retained_math_accent_call_name,
    is_math_call_name as is_retained_math_call_name, is_math_delimiter_helper_call_name,
    is_math_delimiter_symbol_call_name, is_math_size_call_name,
    is_retained_math_name as is_retained_math_name_with, is_unsupported_math_table_call_name,
    named_argument_position, parse_math_bool_literal_with_message, parse_math_cancel_angle,
    parse_math_cancel_length, parse_math_delimited_size, parse_math_fraction_style,
    parse_math_stretch_size, validate_math_binom_call_args, validate_math_class_call_args,
    validate_math_mid_call_args,
};
use crate::typst_label::{LabelParamValue, LabelParams};
use crate::typst_library::math::item::{
    MathAccent, MathArg, MathAst, MathAttach, MathCall, MathCallOptions, MathCancel,
    MathCancelOptions, MathFraction, MathFractionStyle, MathGroup, MathIdentifier, MathNode,
    MathOperator, MathShorthand, MathSpace, MathStringLiteral, MathText, MathTextKind,
};

use super::call::parse_decoration_stroke;

use crate::typst_syntax::ast::{self as typst_ast, AstNode};
use crate::typst_syntax::{
    RangeMapper, RootedPath, SpanKind, SyntaxKind, SyntaxNode, VirtualPath, VirtualRoot,
};

#[cfg(test)]
pub(crate) fn parse_math(source: &str, offset: usize) -> Result<MathAst, LabelError> {
    parse_math_with_params(source, offset, &LabelParams::default())
}

pub(crate) fn parse_math_with_params(
    source: &str,
    offset: usize,
    params: &LabelParams,
) -> Result<MathAst, LabelError> {
    if let Some((idx, _)) = source
        .char_indices()
        .find(|(_, ch)| matches!(ch, '\n' | '\r'))
    {
        return Err(unsupported(
            offset + idx,
            "multi-line math is not supported in Avenger Typst subset",
        ));
    }
    let mut root = crate::typst_syntax::parse_math(source);
    synthesize_ranges(&mut root, source.len(), offset)?;
    reject_syntax_errors(&root, offset)?;
    let math = root
        .cast::<typst_ast::Math>()
        .ok_or_else(|| LabelError::Engine {
            start: offset,
            end: offset + source.len(),
            message: "Typst parser did not return a math root".to_string(),
        })?;
    let nodes = lower_math(math, source, offset, params)?;
    Ok(MathAst {
        source: source.to_string(),
        nodes,
    })
}

fn lower_math(
    math: typst_ast::Math<'_>,
    source: &str,
    offset: usize,
    params: &LabelParams,
) -> Result<Vec<MathNode>, LabelError> {
    let mut nodes = Vec::new();
    for expr in math.exprs() {
        nodes.extend(lower_math_expr(expr, source, offset, params)?);
    }
    Ok(nodes)
}

fn lower_math_expr(
    expr: typst_ast::Expr<'_>,
    source: &str,
    offset: usize,
    params: &LabelParams,
) -> Result<Vec<MathNode>, LabelError> {
    let range = expr.to_untyped().range();
    match expr {
        typst_ast::Expr::Math(math) => lower_math(math, source, offset, params),
        typst_ast::Expr::Space(space) => Ok(vec![MathNode::Space(MathSpace {
            byte_range: offset_range(space.to_untyped().range(), offset),
        })]),
        typst_ast::Expr::MathText(text) => {
            let range = offset_range(text.to_untyped().range(), offset);
            match text.get() {
                typst_ast::MathTextKind::Number(value) => Ok(vec![MathNode::Text(MathText {
                    text: value.to_string(),
                    kind: MathTextKind::Number,
                    byte_range: range,
                })]),
                typst_ast::MathTextKind::Grapheme(value) if value == ";" => Err(unsupported(
                    range.start,
                    "semicolon math arguments are not supported in Avenger Typst subset",
                )),
                typst_ast::MathTextKind::Grapheme(value) if is_identifier_text(value) => {
                    Ok(vec![MathNode::Identifier(MathIdentifier {
                        name: value.to_string(),
                        symbol: named_math_symbol(value),
                        byte_range: range,
                    })])
                }
                typst_ast::MathTextKind::Grapheme(value) if is_operator_text(value) => {
                    Ok(vec![MathNode::Operator(MathOperator {
                        operator: value.to_string(),
                        byte_range: range,
                    })])
                }
                typst_ast::MathTextKind::Grapheme(value) => Ok(vec![MathNode::Text(MathText {
                    text: value.to_string(),
                    kind: MathTextKind::Grapheme,
                    byte_range: range,
                })]),
            }
        }
        typst_ast::Expr::MathIdent(ident) => {
            let name = ident.as_str().to_string();
            Ok(vec![MathNode::Identifier(MathIdentifier {
                symbol: named_math_symbol(&name),
                name,
                byte_range: offset_range(ident.to_untyped().range(), offset),
            })])
        }
        typst_ast::Expr::Ident(ident) => lower_math_param_ident(ident, source, offset, params),
        typst_ast::Expr::MathFieldAccess(access) => lower_math_access_as_nodes(
            typst_ast::MathAccess::MathFieldAccess(access),
            source,
            offset,
        ),
        typst_ast::Expr::MathShorthand(shorthand) => {
            let source_text = shorthand.to_untyped().leaf_text().to_string();
            let range = offset_range(shorthand.to_untyped().range(), offset);
            if let Some(replacement) = shorthand_replacement(&source_text) {
                Ok(vec![MathNode::Shorthand(MathShorthand {
                    source: source_text,
                    replacement,
                    byte_range: range,
                })])
            } else {
                let replacement = shorthand.get().to_string();
                Ok(vec![MathNode::Operator(MathOperator {
                    operator: replacement,
                    byte_range: range,
                })])
            }
        }
        typst_ast::Expr::MathDelimited(delimited) => {
            let open = delimited.open();
            let close = delimited.close();
            let open_range = open.to_untyped().range();
            let close_range = close.to_untyped().range();
            let left = delimiter_char(open, source)?;
            let right = delimiter_char(close, source)?;
            let body_range = open_range.end..close_range.start;
            let body = if body_range.start <= body_range.end {
                parse_math_with_params(
                    &source[body_range.clone()],
                    offset + body_range.start,
                    params,
                )?
                .nodes
            } else {
                Vec::new()
            };
            Ok(vec![MathNode::Group(MathGroup {
                left,
                right,
                body,
                byte_range: offset_range(delimited.to_untyped().range(), offset),
            })])
        }
        typst_ast::Expr::MathAttach(attach) => {
            let base = lower_math_expr_as_single(attach.base(), source, offset, params)?;
            let (top, mut continuation) = attach
                .top()
                .map(|expr| lower_script_expr(expr, source, offset, params))
                .transpose()?
                .map(|(script, continuation)| (Some(vec![script]), continuation))
                .unwrap_or((None, Vec::new()));
            let (bottom, bottom_continuation) = attach
                .bottom()
                .map(|expr| lower_script_expr(expr, source, offset, params))
                .transpose()?
                .map(|(script, continuation)| (Some(vec![script]), continuation))
                .unwrap_or((None, Vec::new()));
            continuation.extend(bottom_continuation);
            let primes = attach.primes().map_or(0, |primes| primes.count());
            let mut byte_range = base.byte_range().start..base.byte_range().end;
            if let Some(top) = top.as_deref().and_then(last_node_byte_range) {
                byte_range.end = byte_range.end.max(top.end);
            }
            if let Some(bottom) = bottom.as_deref().and_then(last_node_byte_range) {
                byte_range.end = byte_range.end.max(bottom.end);
            }
            if primes > 0 {
                byte_range.end = offset_range(attach.to_untyped().range(), offset).end;
            }

            let mut nodes = vec![MathNode::Attach(MathAttach {
                base: Box::new(base),
                top,
                bottom,
                top_left: None,
                top_right: None,
                bottom_left: None,
                bottom_right: None,
                primes,
                byte_range,
            })];
            nodes.extend(continuation);
            Ok(nodes)
        }
        typst_ast::Expr::MathPrimes(primes) => Ok(vec![MathNode::Attach(MathAttach {
            base: Box::new(MathNode::Text(MathText {
                text: String::new(),
                kind: MathTextKind::Grapheme,
                byte_range: offset_range(primes.to_untyped().range(), offset),
            })),
            top: None,
            bottom: None,
            top_left: None,
            top_right: None,
            bottom_left: None,
            bottom_right: None,
            primes: primes.count(),
            byte_range: offset_range(primes.to_untyped().range(), offset),
        })]),
        typst_ast::Expr::MathFrac(frac) => {
            let numerator = lower_math_expr_as_single(frac.num(), source, offset, params)?;
            let denominator = lower_math_expr_as_single(frac.denom(), source, offset, params)?;
            let slash_range = slash_range_between(
                source,
                frac.num().to_untyped().range().end,
                frac.denom().to_untyped().range().start,
                offset,
            );
            let byte_range = numerator.byte_range().start..denominator.byte_range().end;
            Ok(vec![MathNode::Fraction(MathFraction {
                numerator: vec![numerator],
                denominator: vec![denominator],
                style: MathFractionStyle::Vertical,
                slash_range,
                byte_range,
            })])
        }
        typst_ast::Expr::MathRoot(root) => lower_math_root(root, source, offset, params),
        typst_ast::Expr::MathCall(call) => lower_math_call(call, source, offset, params),
        typst_ast::Expr::Str(string) => Ok(vec![MathNode::StringLiteral(MathStringLiteral {
            text: string.get().to_string(),
            byte_range: offset_range(string.to_untyped().range(), offset),
        })]),
        typst_ast::Expr::MathAlignPoint(_) => Err(unsupported(
            range.start + offset,
            "math alignment markers are not supported in Avenger Typst subset",
        )),
        other => Err(unsupported_expr(
            other,
            offset,
            "this Typst math construct is not supported in Avenger Typst subset",
        )),
    }
}

fn lower_math_param_ident(
    ident: typst_ast::Ident<'_>,
    source: &str,
    offset: usize,
    params: &LabelParams,
) -> Result<Vec<MathNode>, LabelError> {
    let local_range = expand_hash_range(source, ident.to_untyped().range());
    let range = offset_range(local_range, offset);
    let Some(value) = params.get(ident.as_str()) else {
        return Err(unsupported(range.start, "unknown label parameter"));
    };
    label_param_to_math_nodes(value, range)
}

fn label_param_to_math_nodes(
    value: &LabelParamValue,
    byte_range: std::ops::Range<usize>,
) -> Result<Vec<MathNode>, LabelError> {
    match value {
        LabelParamValue::None => Ok(Vec::new()),
        LabelParamValue::Bool(value) => Ok(vec![math_text(
            value.to_string(),
            MathTextKind::Grapheme,
            byte_range,
        )]),
        LabelParamValue::Int(value) => Ok(vec![math_text(
            value.to_string(),
            MathTextKind::Number,
            byte_range,
        )]),
        LabelParamValue::Float(value) if value.is_finite() => Ok(vec![math_text(
            format_f64(*value),
            MathTextKind::Number,
            byte_range,
        )]),
        LabelParamValue::Float(_) => Err(LabelError::UnsupportedSyntax {
            position: byte_range.start,
            message: "non-finite label parameter is not supported",
        }),
        LabelParamValue::Str(value) => Ok(vec![math_text(
            value.clone(),
            if is_plain_numeric_text(value) {
                MathTextKind::Number
            } else {
                MathTextKind::Grapheme
            },
            byte_range,
        )]),
        LabelParamValue::Array(_) | LabelParamValue::Dict(_) => {
            Err(LabelError::UnsupportedSyntax {
                position: byte_range.start,
                message: "label parameter value cannot be rendered as math",
            })
        }
    }
}

fn is_plain_numeric_text(value: &str) -> bool {
    value
        .trim()
        .parse::<f64>()
        .is_ok_and(|value| value.is_finite())
}

fn math_text(text: String, kind: MathTextKind, byte_range: std::ops::Range<usize>) -> MathNode {
    MathNode::Text(MathText {
        text,
        kind,
        byte_range,
    })
}

fn format_f64(value: f64) -> String {
    let mut text = value.to_string();
    if text == "-0" {
        text = "0".to_string();
    }
    text
}

fn lower_math_expr_as_single(
    expr: typst_ast::Expr<'_>,
    source: &str,
    offset: usize,
    params: &LabelParams,
) -> Result<MathNode, LabelError> {
    let range = expr.to_untyped().range();
    let mut nodes = lower_math_expr(expr, source, offset, params)?;
    if nodes.len() == 1 {
        Ok(nodes.remove(0))
    } else if let Some(index) = single_non_space_node_index(&nodes) {
        Ok(nodes.remove(index))
    } else if let Some((left, right)) = surrounding_delimiters(&source[range.clone()]) {
        Ok(MathNode::Group(MathGroup {
            left,
            right,
            body: nodes,
            byte_range: offset_range(range, offset),
        }))
    } else {
        Err(unsupported(
            range.start + offset,
            "math attachment expects a single atom",
        ))
    }
}

fn lower_script_expr(
    expr: typst_ast::Expr<'_>,
    source: &str,
    offset: usize,
    params: &LabelParams,
) -> Result<(MathNode, Vec<MathNode>), LabelError> {
    let range = expr.to_untyped().range();
    let mut nodes = lower_math_expr(expr, source, offset, params)?;
    nodes.retain(|node| !matches!(node, MathNode::Space(_)));
    if nodes.is_empty() {
        return Err(unsupported(
            range.start + offset,
            "math script expects an expression",
        ));
    }
    let script = nodes.remove(0);
    Ok((script, nodes))
}

fn last_node_byte_range(nodes: &[MathNode]) -> Option<std::ops::Range<usize>> {
    nodes.last().map(MathNode::byte_range)
}

fn lower_math_root(
    root: typst_ast::MathRoot<'_>,
    source: &str,
    offset: usize,
    params: &LabelParams,
) -> Result<Vec<MathNode>, LabelError> {
    let radicand = lower_math_expr_as_single(root.radicand(), source, offset, params)?;
    let radicand_range = radicand.byte_range();
    let mut args = Vec::new();
    let name = if let Some(index) = root.index() {
        let root_range = offset_range(root.to_untyped().range(), offset);
        args.push(MathArg {
            nodes: vec![MathNode::Text(MathText {
                text: index.to_string(),
                kind: MathTextKind::Number,
                byte_range: root_range.start..root_range.start + 1,
            })],
            byte_range: root_range.start..root_range.start + 1,
        });
        "root"
    } else {
        "sqrt"
    };
    args.push(MathArg {
        nodes: vec![radicand],
        byte_range: radicand_range,
    });
    Ok(vec![MathNode::Call(MathCall {
        name: name.to_string(),
        args,
        options: MathCallOptions::default(),
        byte_range: offset_range(root.to_untyped().range(), offset),
    })])
}

fn lower_math_call(
    call: typst_ast::MathCall<'_>,
    source: &str,
    offset: usize,
    params: &LabelParams,
) -> Result<Vec<MathNode>, LabelError> {
    let name = math_access_name(call.callee());
    let range = offset_range(call.to_untyped().range(), offset);
    if is_unsupported_math_table_call_name(&name) {
        return Err(unsupported(
            range.start,
            "matrix/table math is not supported in Avenger Typst subset",
        ));
    }

    if name == "op" {
        let (name, args) =
            lower_math_op_call_args(call.args(), source, offset, range.start, params)?;
        return Ok(vec![MathNode::Call(MathCall {
            name,
            args,
            options: MathCallOptions::default(),
            byte_range: range,
        })]);
    }

    if name == "frac" {
        return lower_math_frac_call(call.args(), source, offset, range, params);
    }

    if name == "attach" {
        return lower_math_attach_call(call.args(), source, offset, range, params);
    }

    if name == "cancel" {
        return lower_math_cancel_call(call.args(), source, offset, range, params);
    }

    if is_retained_math_accent_call_name(&name, |name| named_accent_char(name).is_some()) {
        return lower_math_accent_call(&name, call.args(), source, offset, range, params);
    }

    if name == "scripts" || name == "limits" {
        let (name, args) = lower_math_attachment_mode_call_args(
            &name,
            call.args(),
            source,
            offset,
            range.start,
            params,
        )?;
        return Ok(vec![MathNode::Call(MathCall {
            name,
            args,
            options: MathCallOptions::default(),
            byte_range: range,
        })]);
    }

    if is_math_size_call_name(&name) {
        let args = lower_math_size_call_args(call.args(), source, offset, range.start, params)?;
        return Ok(vec![MathNode::Call(MathCall {
            name,
            args,
            options: MathCallOptions::default(),
            byte_range: range,
        })]);
    }

    if math_call_name(&name) {
        let (args, options) = if name == "lr"
            || is_math_delimiter_helper_call_name(&name)
            || is_math_delimiter_symbol_call_name(&name)
        {
            lower_math_delimited_call_args(&name, call.args(), source, offset, range.start, params)?
        } else if name == "stretch" {
            lower_math_stretch_call_args(call.args(), source, offset, range.start, params)?
        } else {
            (
                lower_math_call_args(call.args(), source, offset, params)?,
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
        symbol: named_math_symbol(&name),
        name,
        byte_range: offset_range(call.callee().to_untyped().range(), offset),
    })];
    let args_range = offset_range(call.args().to_untyped().range(), offset);
    let body = lower_math_args_as_group_body(call.args(), source, offset, params)?;
    nodes.push(MathNode::Group(MathGroup {
        left: '(',
        right: ')',
        body,
        byte_range: args_range,
    }));
    Ok(nodes)
}

fn lower_math_frac_call(
    args: typst_ast::MathArgs<'_>,
    source: &str,
    offset: usize,
    range: std::ops::Range<usize>,
    params: &LabelParams,
) -> Result<Vec<MathNode>, LabelError> {
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
                let nodes = lower_math_expr(expr, source, offset, params)?;
                let byte_range = offset_range(expr.to_untyped().range(), offset);
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

fn lower_math_attach_call(
    args: typst_ast::MathArgs<'_>,
    source: &str,
    offset: usize,
    range: std::ops::Range<usize>,
    params: &LabelParams,
) -> Result<Vec<MathNode>, LabelError> {
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
                base = Some(lower_math_expr_as_single(expr, source, offset, params)?);
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
                let nodes = lower_math_expr(expr, source, offset, params)?;
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

fn lower_math_delimited_call_args(
    name: &str,
    args: typst_ast::MathArgs<'_>,
    source: &str,
    offset: usize,
    position: usize,
    params: &LabelParams,
) -> Result<(Vec<MathArg>, MathCallOptions), LabelError> {
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
                    nodes: lower_math_expr(expr, source, offset, params)?,
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

fn lower_math_stretch_call_args(
    args: typst_ast::MathArgs<'_>,
    source: &str,
    offset: usize,
    position: usize,
    params: &LabelParams,
) -> Result<(Vec<MathArg>, MathCallOptions), LabelError> {
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
                    nodes: lower_math_expr(expr, source, offset, params)?,
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

fn lower_math_attachment_mode_call_args(
    name: &str,
    args: typst_ast::MathArgs<'_>,
    source: &str,
    offset: usize,
    position: usize,
    params: &LabelParams,
) -> Result<(String, Vec<MathArg>), LabelError> {
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
                    nodes: lower_math_expr(expr, source, offset, params)?,
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

fn lower_math_cancel_call(
    args: typst_ast::MathArgs<'_>,
    source: &str,
    offset: usize,
    range: std::ops::Range<usize>,
    params: &LabelParams,
) -> Result<Vec<MathNode>, LabelError> {
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
                body = Some(lower_math_expr(expr, source, offset, params)?);
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

fn lower_math_accent_call(
    name: &str,
    args: typst_ast::MathArgs<'_>,
    source: &str,
    offset: usize,
    range: std::ops::Range<usize>,
    params: &LabelParams,
) -> Result<Vec<MathNode>, LabelError> {
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
                    nodes: lower_math_expr(expr, source, offset, params)?,
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
        accent_arg_char(&lowered[1])
            .ok_or_else(|| unsupported(lowered[1].byte_range.start, "unsupported accent value"))?
    } else {
        named_accent_char(name).expect("accent call names should be prevalidated")
    };
    Ok(vec![MathNode::Accent(MathAccent {
        base: lowered.remove(0).nodes,
        accent,
        dotless,
        byte_range: range,
    })])
}

fn accent_arg_char(arg: &MathArg) -> Option<char> {
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
    normalize_accent_text(text)
}

fn lower_math_op_call_args(
    args: typst_ast::MathArgs<'_>,
    source: &str,
    offset: usize,
    position: usize,
    params: &LabelParams,
) -> Result<(String, Vec<MathArg>), LabelError> {
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
                    nodes: lower_math_expr(expr, source, offset, params)?,
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

fn lower_math_call_args(
    args: typst_ast::MathArgs<'_>,
    source: &str,
    offset: usize,
    params: &LabelParams,
) -> Result<Vec<MathArg>, LabelError> {
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
                    nodes: lower_math_expr(expr, source, offset, params)?,
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

fn lower_math_size_call_args(
    args: typst_ast::MathArgs<'_>,
    source: &str,
    offset: usize,
    position: usize,
    params: &LabelParams,
) -> Result<Vec<MathArg>, LabelError> {
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
                    nodes: lower_math_expr(expr, source, offset, params)?,
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

fn lower_math_args_as_group_body(
    args: typst_ast::MathArgs<'_>,
    source: &str,
    offset: usize,
    params: &LabelParams,
) -> Result<Vec<MathNode>, LabelError> {
    let mut body = Vec::new();
    for item in args.content_items() {
        match item {
            typst_ast::MathArgItem::Arg(typst_ast::Arg::Pos(expr)) => {
                body.extend(lower_math_expr(expr, source, offset, params)?);
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

fn math_access_name(access: typst_ast::MathAccess<'_>) -> String {
    match access {
        typst_ast::MathAccess::MathIdent(ident) => ident.as_str().to_string(),
        typst_ast::MathAccess::MathFieldAccess(access) => math_field_access_name(access),
    }
}

fn math_field_access_name(access: typst_ast::MathFieldAccess<'_>) -> String {
    let mut name = math_access_name(access.target());
    name.push('.');
    name.push_str(access.field().as_str());
    name
}

fn lower_math_access_as_nodes(
    access: typst_ast::MathAccess<'_>,
    source: &str,
    offset: usize,
) -> Result<Vec<MathNode>, LabelError> {
    match access {
        typst_ast::MathAccess::MathIdent(ident) => {
            let name = ident.as_str().to_string();
            Ok(vec![MathNode::Identifier(MathIdentifier {
                symbol: named_math_symbol(&name),
                name,
                byte_range: offset_range(ident.to_untyped().range(), offset),
            })])
        }
        typst_ast::MathAccess::MathFieldAccess(access) => {
            let full_name = math_field_access_name(access);
            if named_math_symbol(&full_name).is_some() {
                return Ok(vec![MathNode::Identifier(MathIdentifier {
                    symbol: named_math_symbol(&full_name),
                    name: full_name,
                    byte_range: offset_range(access.to_untyped().range(), offset),
                })]);
            }

            let mut nodes = lower_math_access_as_nodes(access.target(), source, offset)?;
            let target_range = access.target().to_untyped().range();
            let field = access.field();
            let field_range = field.to_untyped().range();
            let dot_start = source[target_range.end..field_range.start]
                .find('.')
                .map(|idx| target_range.end + idx)
                .unwrap_or(target_range.end);
            nodes.push(MathNode::Operator(MathOperator {
                operator: ".".to_string(),
                byte_range: offset + dot_start..offset + dot_start + 1,
            }));
            let field_name = field.as_str().to_string();
            nodes.push(MathNode::Identifier(MathIdentifier {
                symbol: named_math_symbol(&field_name),
                name: field_name,
                byte_range: offset_range(field_range, offset),
            }));
            Ok(nodes)
        }
    }
}

fn delimiter_char(expr: typst_ast::Expr<'_>, source: &str) -> Result<char, LabelError> {
    let range = expr.to_untyped().range();
    source[range.clone()]
        .chars()
        .next()
        .ok_or_else(|| unsupported(range.start, "empty math delimiter"))
}

fn offset_range(range: std::ops::Range<usize>, offset: usize) -> std::ops::Range<usize> {
    offset + range.start..offset + range.end
}

fn expand_hash_range(source: &str, range: std::ops::Range<usize>) -> std::ops::Range<usize> {
    if range.start > 0 && source.as_bytes().get(range.start - 1) == Some(&b'#') {
        range.start - 1..range.end
    } else {
        range
    }
}

fn single_non_space_node_index(nodes: &[MathNode]) -> Option<usize> {
    let mut matches = nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| !matches!(node, MathNode::Space(_)));
    let (index, _) = matches.next()?;
    matches.next().is_none().then_some(index)
}

fn surrounding_delimiters(source: &str) -> Option<(char, char)> {
    let mut chars = source.chars();
    let left = chars.next()?;
    let right = source.chars().next_back()?;
    matches!(
        (left, right),
        ('(', ')') | ('[', ']') | ('{', '}') | ('|', '|')
    )
    .then_some((left, right))
}

fn slash_range_between(
    source: &str,
    start: usize,
    end: usize,
    offset: usize,
) -> std::ops::Range<usize> {
    let slash = source[start..end]
        .find('/')
        .map(|idx| start + idx)
        .unwrap_or(start);
    offset + slash..offset + slash + 1
}

fn shorthand_replacement(source: &str) -> Option<&'static str> {
    SHORTHANDS
        .iter()
        .find(|(candidate, _)| *candidate == source)
        .map(|(_, replacement)| *replacement)
}

fn is_operator_text(text: &str) -> bool {
    matches!(
        text,
        "+" | "-"
            | "−"
            | "*"
            | "∗"
            | "="
            | "<"
            | ">"
            | "!"
            | ":"
            | ","
            | "."
            | "|"
            | "&"
            | "≤"
            | "≥"
            | "≠"
            | "→"
            | "←"
            | "⇒"
            | "↔"
            | "≔"
    )
}

fn is_identifier_text(text: &str) -> bool {
    !text.is_empty() && text.chars().all(char::is_alphabetic)
}

fn reject_syntax_errors(root: &SyntaxNode, offset: usize) -> Result<(), LabelError> {
    if !root.diagnosis().errors {
        return Ok(());
    }
    let (errors, _) = root.errors_and_warnings();
    let message = errors
        .first()
        .map(|error| error.message.to_string())
        .unwrap_or_else(|| "invalid Typst math syntax".to_string());
    let position = first_error_range(root)
        .map(|range| offset + range.start)
        .unwrap_or(offset);
    Err(LabelError::Syntax { position, message })
}

fn first_error_range(node: &SyntaxNode) -> Option<std::ops::Range<usize>> {
    if node.kind() == SyntaxKind::Error {
        return Some(node.range());
    }
    node.children().find_map(first_error_range)
}

fn unsupported_expr(expr: typst_ast::Expr<'_>, offset: usize, message: &'static str) -> LabelError {
    unsupported(expr.to_untyped().range().start + offset, message)
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

fn synthesize_ranges(
    root: &mut SyntaxNode,
    source_len: usize,
    offset: usize,
) -> Result<(), LabelError> {
    let mapper = RangeMapper::new([0..source_len]).map_err(|message| LabelError::Engine {
        start: offset,
        end: offset + source_len,
        message: message.to_string(),
    })?;
    root.synthesize_mapped(scratch_file_id(), &mapper)
        .map_err(|message| LabelError::Engine {
            start: offset,
            end: offset + source_len,
            message: message.to_string(),
        })
}

fn scratch_file_id() -> crate::typst_syntax::FileId {
    RootedPath::new(
        VirtualRoot::Project,
        VirtualPath::new("avenger-typst-label-math.typ").expect("static virtual path is valid"),
    )
    .intern()
}

const SHORTHANDS: &[(&str, &str)] = &[
    ("...", "…"),
    ("<=", "≤"),
    (">=", "≥"),
    ("!=", "≠"),
    ("=>", "⇒"),
    ("->", "→"),
    ("<-", "←"),
    (":=", "≔"),
];

fn math_call_name(name: &str) -> bool {
    is_retained_math_call_name(
        name,
        |name| predefined_operator_text(name).is_some(),
        |name| named_accent_char(name).is_some(),
    )
}

pub(crate) fn is_retained_math_name(name: &str) -> bool {
    is_retained_math_name_with(
        name,
        |name| predefined_operator_text(name).is_some(),
        |name| named_accent_char(name).is_some(),
        |name| named_math_symbol(name).is_some(),
    )
}

pub(crate) fn named_accent_char(name: &str) -> Option<char> {
    match name {
        "grave" => Some('\u{0300}'),
        "acute" => Some('\u{0301}'),
        "hat" => Some('\u{0302}'),
        "tilde" => Some('\u{0303}'),
        "macron" | "bar" => Some('\u{0304}'),
        "dash" => Some('\u{0305}'),
        "breve" => Some('\u{0306}'),
        "dot" => Some('\u{0307}'),
        "dot.double" | "ddot" | "diaer" => Some('\u{0308}'),
        "dot.triple" => Some('\u{20db}'),
        "dot.quad" => Some('\u{20dc}'),
        "circle" => Some('\u{030a}'),
        "acute.double" => Some('\u{030b}'),
        "caron" => Some('\u{030c}'),
        "arrow" | "arrow.r" => Some('\u{20d7}'),
        "arrow.l" => Some('\u{20d6}'),
        "arrow.l.r" => Some('\u{20e1}'),
        "harpoon" => Some('\u{20d1}'),
        "harpoon.lt" => Some('\u{20d0}'),
        _ => None,
    }
}

pub(crate) fn normalize_accent_text(value: &str) -> Option<char> {
    named_accent_char(value).or_else(|| {
        ACCENT_ALIASES
            .iter()
            .find_map(|(accent, aliases)| aliases.contains(&value).then_some(*accent))
            .or_else(|| value.parse::<char>().ok())
    })
}

const ACCENT_ALIASES: &[(char, &[&str])] = &[
    ('\u{0300}', &["`"]),
    ('\u{0301}', &["´"]),
    ('\u{0302}', &["^", "ˆ"]),
    ('\u{0303}', &["~", "∼", "˜"]),
    ('\u{0304}', &["¯"]),
    ('\u{0305}', &["-", "–", "‾", "−"]),
    ('\u{0306}', &["˘"]),
    ('\u{0307}', &[".", "˙", "⋅"]),
    ('\u{0308}', &["¨"]),
    ('\u{030a}', &["∘", "○"]),
    ('\u{030b}', &["˝"]),
    ('\u{030c}', &["ˇ"]),
    ('\u{20d6}', &["←"]),
    ('\u{20d7}', &["→", "⟶"]),
    ('\u{20e1}', &["↔", "↔\u{fe0e}", "⟷"]),
    ('\u{20d0}', &["↼"]),
    ('\u{20d1}', &["⇀"]),
];

pub(crate) fn named_math_symbol(name: &str) -> Option<&'static str> {
    match name {
        "alpha" => Some("α"),
        "beta" => Some("β"),
        "gamma" => Some("γ"),
        "delta" => Some("δ"),
        "epsilon" => Some("ε"),
        "zeta" => Some("ζ"),
        "eta" => Some("η"),
        "theta" => Some("θ"),
        "iota" => Some("ι"),
        "kappa" => Some("κ"),
        "lambda" => Some("λ"),
        "mu" => Some("μ"),
        "nu" => Some("ν"),
        "xi" => Some("ξ"),
        "pi" => Some("π"),
        "rho" => Some("ρ"),
        "sigma" => Some("σ"),
        "tau" => Some("τ"),
        "upsilon" => Some("υ"),
        "phi" => Some("φ"),
        "chi" => Some("χ"),
        "psi" => Some("ψ"),
        "omega" => Some("ω"),
        "Gamma" => Some("Γ"),
        "Delta" => Some("Δ"),
        "Theta" => Some("Θ"),
        "Lambda" => Some("Λ"),
        "Xi" => Some("Ξ"),
        "Pi" => Some("Π"),
        "Sigma" => Some("Σ"),
        "Upsilon" => Some("Υ"),
        "Phi" => Some("Φ"),
        "Psi" => Some("Ψ"),
        "Omega" => Some("Ω"),
        "dot" | "dot.op" => Some("⋅"),
        "dot.c" => Some("·"),
        "dots" | "dots.h" => Some("…"),
        "dots.h.c" => Some("⋯"),
        "dots.v" => Some("⋮"),
        "sum" => Some("∑"),
        "prod" | "product" => Some("∏"),
        "integral" => Some("∫"),
        "oo" | "infinity" => Some("∞"),
        "partial" => Some("∂"),
        "gradient" | "nabla" => Some("∇"),
        "RR" => Some("ℝ"),
        "NN" => Some("ℕ"),
        "ZZ" => Some("ℤ"),
        "QQ" => Some("ℚ"),
        "CC" => Some("ℂ"),
        "plus" => Some("+"),
        "plus.minus" => Some("±"),
        "minus" => Some("−"),
        "minus.plus" => Some("∓"),
        "times" => Some("×"),
        "times.big" => Some("⨉"),
        "div" => Some("÷"),
        "eq" => Some("="),
        "eq.not" => Some("≠"),
        "eq.triple" | "equiv" => Some("≡"),
        "eq.triple.not" | "equiv.not" => Some("≢"),
        "lt" => Some("<"),
        "lt.eq" => Some("≤"),
        "lt.eq.not" => Some("≰"),
        "lt.not" => Some("≮"),
        "gt" => Some(">"),
        "gt.eq" => Some("≥"),
        "gt.eq.not" => Some("≱"),
        "gt.not" => Some("≯"),
        "approx" => Some("≈"),
        "approx.not" => Some("≉"),
        "prop" => Some("∝"),
        "emptyset" | "nothing" => Some("∅"),
        "in" => Some("∈"),
        "in.not" => Some("∉"),
        "in.rev" => Some("∋"),
        "in.rev.not" => Some("∌"),
        "subset" => Some("⊂"),
        "subset.eq" => Some("⊆"),
        "subset.eq.not" => Some("⊈"),
        "subset.neq" => Some("⊊"),
        "subset.not" => Some("⊄"),
        "supset" => Some("⊃"),
        "supset.eq" => Some("⊇"),
        "supset.eq.not" => Some("⊉"),
        "supset.neq" => Some("⊋"),
        "supset.not" => Some("⊅"),
        "union" => Some("∪"),
        "union.big" => Some("⋃"),
        "union.plus" => Some("⊎"),
        "inter" => Some("∩"),
        "inter.big" => Some("⋂"),
        "forall" => Some("∀"),
        "exists" => Some("∃"),
        "angle" => Some("∠"),
        "parallel" => Some("∥"),
        "perp" => Some("⟂"),
        "degree" => Some("°"),
        "aleph" => Some("א"),
        "ell" => Some("ℓ"),
        "arrow.r" => Some("→"),
        "arrow.r.long" => Some("⟶"),
        "arrow.r.bar" => Some("↦"),
        "arrow.r.double" => Some("⇒"),
        "arrow.r.double.long" => Some("⟹"),
        "arrow.r.not" => Some("↛"),
        "arrow.l" => Some("←"),
        "arrow.l.long" => Some("⟵"),
        "arrow.l.bar" => Some("↤"),
        "arrow.l.double" => Some("⇐"),
        "arrow.l.double.long" => Some("⟸"),
        "arrow.l.not" => Some("↚"),
        "arrow.l.r" => Some("↔"),
        "arrow.l.r.long" => Some("⟷"),
        "arrow.l.r.double" => Some("⇔"),
        "arrow.l.r.double.long" => Some("⟺"),
        "arrow.t" => Some("↑"),
        "arrow.b" => Some("↓"),
        _ => None,
    }
}

pub(crate) fn predefined_operator_text(name: &str) -> Option<&'static str> {
    match name {
        "arccos" => Some("arccos"),
        "arcsin" => Some("arcsin"),
        "arctan" => Some("arctan"),
        "arg" => Some("arg"),
        "cos" => Some("cos"),
        "cosh" => Some("cosh"),
        "cot" => Some("cot"),
        "coth" => Some("coth"),
        "csc" => Some("csc"),
        "csch" => Some("csch"),
        "ctg" => Some("ctg"),
        "deg" => Some("deg"),
        "det" => Some("det"),
        "dim" => Some("dim"),
        "exp" => Some("exp"),
        "gcd" => Some("gcd"),
        "lcm" => Some("lcm"),
        "hom" => Some("hom"),
        "id" => Some("id"),
        "im" => Some("im"),
        "inf" => Some("inf"),
        "ker" => Some("ker"),
        "lg" => Some("lg"),
        "lim" => Some("lim"),
        "liminf" => Some("lim\u{2009}inf"),
        "limsup" => Some("lim\u{2009}sup"),
        "ln" => Some("ln"),
        "log" => Some("log"),
        "max" => Some("max"),
        "min" => Some("min"),
        "mod" => Some("mod"),
        "Pr" => Some("Pr"),
        "sec" => Some("sec"),
        "sech" => Some("sech"),
        "sin" => Some("sin"),
        "sinc" => Some("sinc"),
        "sinh" => Some("sinh"),
        "sup" => Some("sup"),
        "tan" => Some("tan"),
        "tanh" => Some("tanh"),
        "tg" => Some("tg"),
        "tr" => Some("tr"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typst_library::math::item::MathCancelAngle;

    fn parse(source: &str) -> MathAst {
        parse_math(source, 0).unwrap()
    }

    #[test]
    fn resolves_embedded_math_params() {
        let mut params = LabelParams::new();
        params.insert("slope".to_string(), LabelParamValue::Float(2.5));
        params.insert("intercept".to_string(), LabelParamValue::Int(7));

        let math = parse_math_with_params("y = #slope x + #intercept", 0, &params).unwrap();
        let text = math
            .nodes
            .iter()
            .filter_map(|node| match node {
                MathNode::Text(text) => Some(text.text.as_str()),
                MathNode::Identifier(identifier) => Some(identifier.name.as_str()),
                MathNode::Operator(operator) => Some(operator.operator.as_str()),
                MathNode::Space(_) => Some(" "),
                _ => None,
            })
            .collect::<String>();

        assert_eq!(text, "y = 2.5 x + 7");
    }

    #[test]
    fn parses_core_oracle_fragments() {
        for source in [
            "x",
            "y",
            "t",
            "x^2 + y^2",
            "x_i^2",
            "x'",
            "x''",
            "x_1^2",
            "sqrt(x) / (1 + x^2)",
            "root(3, x)",
            "frac(x + y, z)",
            "binom(n, k)",
            "cancel(x)",
            "a / b",
            "a / (b + c)",
            "J_0(x)",
            "J_n(x)",
            "sum_(i=0)^n i",
            "lim_(x -> oo) f(x)",
            "limits(A)_1^2",
            "scripts(sum)_1^2",
            "sin(x)",
            "sech(x)",
            "liminf_(n -> oo)",
            "op(\"custom\")",
            "op(\"custom\", limits: #true)",
            "abs(x)",
            "norm(v)",
            "floor(x)",
            "ceil(x)",
            "round(x)",
            "mid(|)",
            "ceil.l(x)",
            "floor.l(x)",
            "paren.l(x)",
            "brace.l(x)",
            "bracket.l(x)",
            "chevron.l(x)",
            "bar.double(x)",
            "stretch(->, size: #200%)",
            "alpha + beta -> gamma",
            "alpha + pi + sum",
            "x(t)",
            "x(t) = A r^t",
            "R^2 = 0.94",
            "y = sqrt(x) / (1 + x^2)",
        ] {
            parse_math(source, 0)
                .unwrap_or_else(|err| panic!("Typst math parser failed for {source:?}: {err:?}"));
        }
    }

    #[test]
    fn parses_slash_fraction_with_parenthesized_denominator() {
        let math = parse("sqrt(x) / (1 + x^2)");

        assert_eq!(math.nodes.len(), 1);
        let MathNode::Fraction(fraction) = &math.nodes[0] else {
            panic!("expected slash fraction");
        };
        assert!(matches!(
            fraction.numerator.as_slice(),
            [MathNode::Call(call)] if call.name == "sqrt"
        ));
        assert!(matches!(
            fraction.denominator.as_slice(),
            [MathNode::Group(group)] if group.left == '(' && group.right == ')'
        ));
    }

    #[test]
    fn parses_frac_style_options() {
        let math =
            parse("frac(x, y) + frac(x, y, style: \"skewed\") + frac(x, y, style: \"horizontal\")");

        assert!(matches!(
            &math.nodes[0],
            MathNode::Fraction(fraction)
                if fraction.style == MathFractionStyle::Vertical
                    && fraction.numerator.len() == 1
                    && fraction.denominator.len() == 1
        ));
        assert!(matches!(
            &math.nodes[4],
            MathNode::Fraction(fraction) if fraction.style == MathFractionStyle::Skewed
        ));
        assert!(matches!(
            &math.nodes[8],
            MathNode::Fraction(fraction) if fraction.style == MathFractionStyle::Horizontal
        ));
    }

    #[test]
    fn parses_variadic_binom_call() {
        let math = parse("binom(n, k_1, k_2, k_3)");

        assert!(matches!(
            &math.nodes[0],
            MathNode::Call(call) if call.name == "binom" && call.args.len() == 4
        ));
    }

    #[test]
    fn parses_delimiter_size_options() {
        let math = parse("lr(size: #240%, |x|) + abs(x, size: #2em) + norm(v, size: #18pt)");

        let MathNode::Call(lr) = &math.nodes[0] else {
            panic!("lr should lower to a typed call");
        };
        assert_eq!(lr.name, "lr");
        assert_eq!(lr.args.len(), 1);
        let lr_size = lr
            .options
            .delimiter_size
            .expect("lr size option should be retained");
        assert!((lr_size.relative - 2.4).abs() < f32::EPSILON);
        assert!((lr_size.absolute_em - 0.0).abs() < f32::EPSILON);
        assert!((lr_size.absolute_pt - 0.0).abs() < f32::EPSILON);

        let MathNode::Call(abs) = &math.nodes[4] else {
            panic!("abs should lower to a typed call");
        };
        assert_eq!(abs.name, "abs");
        let abs_size = abs
            .options
            .delimiter_size
            .expect("abs size option should be retained");
        assert!((abs_size.relative - 0.0).abs() < f32::EPSILON);
        assert!((abs_size.absolute_em - 2.0).abs() < f32::EPSILON);
        assert!((abs_size.absolute_pt - 0.0).abs() < f32::EPSILON);

        let MathNode::Call(norm) = &math.nodes[8] else {
            panic!("norm should lower to a typed call");
        };
        assert_eq!(norm.name, "norm");
        let norm_size = norm
            .options
            .delimiter_size
            .expect("norm size option should be retained");
        assert!((norm_size.relative - 0.0).abs() < f32::EPSILON);
        assert!((norm_size.absolute_em - 0.0).abs() < f32::EPSILON);
        assert!((norm_size.absolute_pt - 18.0).abs() < f32::EPSILON);
    }

    #[test]
    fn parses_callable_delimiter_symbol_size_options() {
        let math = parse("bracket.l(x, size: #400%)");

        let MathNode::Call(bracket) = &math.nodes[0] else {
            panic!("bracket.l should lower to a typed call");
        };
        assert_eq!(bracket.name, "bracket.l");
        assert_eq!(bracket.args.len(), 1);
        let size = bracket
            .options
            .delimiter_size
            .expect("bracket.l size option should be retained");
        assert!((size.relative - 4.0).abs() < f32::EPSILON);
        assert!((size.absolute_em - 0.0).abs() < f32::EPSILON);
        assert!((size.absolute_pt - 0.0).abs() < f32::EPSILON);

        for source in [
            "ceil.l(x)",
            "floor.l(x)",
            "paren.l(x)",
            "brace.l(x)",
            "chevron.l(x)",
            "bar.double(x)",
        ] {
            let math = parse(source);
            assert!(
                matches!(&math.nodes[..], [MathNode::Call(call)] if call.name == source.trim_end_matches("(x)")),
                "{source} should lower to a typed delimiter symbol call"
            );
        }
    }

    #[test]
    fn rejects_invalid_delimiter_size_options() {
        for (source, message) in [
            ("lr(|x|, foo: #true)", "unsupported lr option"),
            ("abs(x, foo: #true)", "unsupported delimiter option"),
            (
                "bracket.l(x, nope: \"nope\")",
                "unsupported delimiter option",
            ),
            ("abs(x, size: #auto)", "unsupported delimiter size value"),
            ("abs(x, size: #45deg)", "unsupported delimiter size value"),
            (
                "abs(x, y)",
                "delimiter call expects exactly one body argument",
            ),
        ] {
            let err = parse_math(source, 0).unwrap_err();
            assert!(
                format!("{err}").contains(message),
                "{source}: expected {message}, got {err}"
            );
        }
    }

    #[test]
    fn rejects_invalid_mid_calls() {
        for (source, message) in [
            ("mid()", "mid expects a body argument"),
            ("mid(|, |)", "mid expects exactly one body argument"),
        ] {
            let err = parse_math(source, 0).unwrap_err();
            assert!(
                format!("{err}").contains(message),
                "{source}: expected {message}, got {err}"
            );
        }
    }

    #[test]
    fn parses_stretch_size_options() {
        let math = parse("stretch(->, size: #200%) + stretch(|, size: #2em)");

        let MathNode::Call(horizontal) = &math.nodes[0] else {
            panic!("stretch should lower to a typed call");
        };
        assert_eq!(horizontal.name, "stretch");
        assert_eq!(horizontal.args.len(), 1);
        let horizontal_size = horizontal
            .options
            .stretch_size
            .expect("stretch size option should be retained");
        assert!((horizontal_size.relative - 2.0).abs() < f32::EPSILON);
        assert!((horizontal_size.absolute_em - 0.0).abs() < f32::EPSILON);
        assert!((horizontal_size.absolute_pt - 0.0).abs() < f32::EPSILON);

        let MathNode::Call(vertical) = &math.nodes[4] else {
            panic!("second stretch should lower to a typed call");
        };
        assert_eq!(vertical.name, "stretch");
        let vertical_size = vertical
            .options
            .stretch_size
            .expect("vertical stretch size option should be retained");
        assert!((vertical_size.relative - 0.0).abs() < f32::EPSILON);
        assert!((vertical_size.absolute_em - 2.0).abs() < f32::EPSILON);
        assert!((vertical_size.absolute_pt - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn rejects_invalid_stretch_options() {
        for (source, message) in [
            ("stretch(->, foo: #true)", "unsupported stretch option"),
            ("stretch(->, size: #auto)", "unsupported stretch size value"),
            (
                "stretch(->, size: #45deg)",
                "unsupported stretch size value",
            ),
            (
                "stretch(->, x)",
                "stretch expects exactly one body argument",
            ),
        ] {
            let err = parse_math(source, 0).unwrap_err();
            assert!(
                format!("{err}").contains(message),
                "{source}: expected {message}, got {err}"
            );
        }
    }

    #[test]
    fn parses_scripts_and_primes() {
        let math = parse("x_i^2 + x''");

        assert!(matches!(
            &math.nodes[0],
            MathNode::Attach(attach)
                if attach.top.is_some() && attach.bottom.is_some() && attach.primes == 0
        ));
        assert!(matches!(
            &math.nodes[4],
            MathNode::Attach(attach) if attach.primes == 2
        ));
    }

    #[test]
    fn parses_attachment_mode_calls() {
        let limits = parse("limits(A)_1^2");
        assert!(matches!(
            &limits.nodes[0],
            MathNode::Attach(attach)
                if matches!(attach.base.as_ref(), MathNode::Call(call) if call.name == "limits" && call.args.len() == 1)
        ));

        let display_limits = parse("limits(A, inline: #false)_1^2");
        assert!(matches!(
            &display_limits.nodes[0],
            MathNode::Attach(attach)
                if matches!(attach.base.as_ref(), MathNode::Call(call) if call.name == "limits_display" && call.args.len() == 1)
        ));

        let scripts = parse("scripts(sum)_1^2");
        assert!(matches!(
            &scripts.nodes[0],
            MathNode::Attach(attach)
                if matches!(attach.base.as_ref(), MathNode::Call(call) if call.name == "scripts" && call.args.len() == 1)
        ));
    }

    #[test]
    fn rejects_unsupported_attachment_mode_options() {
        let err = parse_math("limits(A, inline: #auto)", 0).unwrap_err();
        assert!(format!("{err}").contains("unsupported limits inline value"));

        let err = parse_math("scripts(A, inline: #true)", 0).unwrap_err();
        assert!(format!("{err}").contains("unsupported scripts option"));
    }

    #[test]
    fn parses_cancel_options() {
        let math = parse("cancel(x, length: #200%, inverted: #true, cross: #true, angle: #45deg)");
        let [MathNode::Cancel(cancel)] = &math.nodes[..] else {
            panic!("cancel call should lower to typed cancel node");
        };
        assert_eq!(cancel.body.len(), 1);
        assert!((cancel.options.length.relative - 2.0).abs() < f32::EPSILON);
        assert!((cancel.options.length.absolute_em - 0.0).abs() < f32::EPSILON);
        assert!(cancel.options.inverted);
        assert!(cancel.options.cross);
        assert!(matches!(
            cancel.options.angle,
            MathCancelAngle::Degrees(value) if (value - 45.0).abs() < f32::EPSILON
        ));

        let em_length = parse("cancel(x, length: #1.5em, angle: #auto)");
        let [MathNode::Cancel(cancel)] = &em_length.nodes[..] else {
            panic!("cancel call should lower to typed cancel node");
        };
        assert!((cancel.options.length.relative - 0.0).abs() < f32::EPSILON);
        assert!((cancel.options.length.absolute_em - 1.5).abs() < f32::EPSILON);
        assert_eq!(cancel.options.angle, MathCancelAngle::Auto);

        let stroke = parse("cancel(x, stroke: #(thickness: 0.25em, paint: red, cap: \"round\"))");
        let [MathNode::Cancel(cancel)] = &stroke.nodes[..] else {
            panic!("cancel call should lower to typed cancel node");
        };
        assert_eq!(
            cancel.options.stroke.paint,
            Some(crate::typst_library::Color::rgba(1.0, 0.0, 0.0, 1.0))
        );
        assert_eq!(
            cancel.options.stroke.thickness,
            Some(crate::typst_library::text::content::DecorationLength::Em(
                0.25,
            ))
        );
        assert_eq!(
            cancel.options.stroke.line_cap,
            Some(crate::typst_svg::StrokeCap::Round)
        );
    }

    #[test]
    fn rejects_invalid_cancel_options() {
        for (source, message) in [
            ("cancel(x, foo: #true)", "unsupported cancel option"),
            (
                "cancel(x, length: #12pt)",
                "unsupported cancel length value",
            ),
            (
                "cancel(x, inverted: #auto)",
                "unsupported cancel inverted value",
            ),
            ("cancel(x, cross: #auto)", "unsupported cancel cross value"),
            ("cancel(x, angle: #50%)", "unsupported cancel angle value"),
            (
                "cancel(x, stroke: #auto.none)",
                "unsupported decoration stroke value",
            ),
        ] {
            let err = parse_math(source, 0).unwrap_err();
            assert!(
                format!("{err}").contains(message),
                "{source}: expected {message}, got {err}"
            );
        }
    }

    #[test]
    fn parses_accent_calls() {
        let math = parse("grave(a) + dot.double(a) + arrow.l.r(Z) + accent(v, <-)");

        assert!(matches!(
            &math.nodes[0],
            MathNode::Accent(accent) if accent.accent == '\u{0300}' && accent.dotless
        ));
        assert!(matches!(
            &math.nodes[4],
            MathNode::Accent(accent) if accent.accent == '\u{0308}' && accent.dotless
        ));
        assert!(matches!(
            &math.nodes[8],
            MathNode::Accent(accent) if accent.accent == '\u{20e1}' && accent.dotless
        ));
        assert!(matches!(
            &math.nodes[12],
            MathNode::Accent(accent) if accent.accent == '\u{20d6}' && accent.dotless
        ));
    }

    #[test]
    fn parses_accent_dotless_option() {
        let math = parse("hat(dotless: #false, i) + accent(dotless: #true, j, \".\")");

        assert!(matches!(
            &math.nodes[0],
            MathNode::Accent(accent) if accent.accent == '\u{0302}' && !accent.dotless
        ));
        assert!(matches!(
            &math.nodes[4],
            MathNode::Accent(accent) if accent.accent == '\u{0307}' && accent.dotless
        ));
    }

    #[test]
    fn rejects_unsupported_accent_options() {
        for (source, message) in [
            (
                "hat(x, size: #150%)",
                "accent size option is not supported yet",
            ),
            ("hat(x, dotless: 1)", "unsupported accent dotless value"),
            ("accent(x, ., foo: #true)", "unsupported accent option"),
            ("accent(x)", "accent math expects a base and accent"),
        ] {
            let err = parse_math(source, 0).unwrap_err();
            assert!(
                format!("{err}").contains(message),
                "{source}: expected {message}, got {err}"
            );
        }
    }

    #[test]
    fn parses_symbols_and_shorthands() {
        let math = parse("alpha -> RR + in.not + subset.eq + arrow.r.double");

        assert!(matches!(
            &math.nodes[0],
            MathNode::Identifier(ident)
                if ident.name == "alpha" && ident.symbol == Some("α")
        ));
        assert!(matches!(
            &math.nodes[2],
            MathNode::Shorthand(shorthand)
                if shorthand.source == "->" && shorthand.replacement == "→"
        ));
        assert!(matches!(
            &math.nodes[4],
            MathNode::Identifier(ident) if ident.symbol == Some("ℝ")
        ));
        assert!(matches!(
            &math.nodes[8],
            MathNode::Identifier(ident)
                if ident.name == "in.not" && ident.symbol == Some("∉")
        ));
        assert!(matches!(
            &math.nodes[12],
            MathNode::Identifier(ident)
                if ident.name == "subset.eq" && ident.symbol == Some("⊆")
        ));
        assert!(matches!(
            &math.nodes[16],
            MathNode::Identifier(ident)
                if ident.name == "arrow.r.double" && ident.symbol == Some("⇒")
        ));
    }

    #[test]
    fn dotted_symbol_suffixes_only_consume_known_symbols() {
        let math = parse("arrow.unknown");

        assert!(matches!(
            &math.nodes[..],
            [
                MathNode::Identifier(identifier),
                MathNode::Operator(operator),
                MathNode::Identifier(suffix),
            ] if identifier.name == "arrow"
                && identifier.symbol.is_none()
                && operator.operator == "."
                && suffix.name == "unknown"
        ));
    }

    #[test]
    fn parses_whitelisted_function_calls() {
        let math = parse(
            "frac(x, y) + op(\"custom\", limits: #true) + bb(R) + scr(P) + class(\"relation\", !) + overline(underline(x)) + attach(Pi, t: alpha, b: beta, tl: 1, tr: 2+3, bl: 4+5, br: 6)",
        );

        assert!(matches!(
            &math.nodes[0],
            MathNode::Fraction(fraction)
                if fraction.style == MathFractionStyle::Vertical
                    && fraction.numerator.len() == 1
                    && fraction.denominator.len() == 1
        ));
        assert!(matches!(
            &math.nodes[4],
            MathNode::Call(call) if call.name == "op_limits" && call.args.len() == 1
        ));
        assert!(matches!(
            &math.nodes[8],
            MathNode::Call(call) if call.name == "bb" && call.args.len() == 1
        ));
        assert!(matches!(
            &math.nodes[12],
            MathNode::Call(call) if call.name == "scr" && call.args.len() == 1
        ));
        assert!(matches!(
            &math.nodes[16],
            MathNode::Call(call) if call.name == "class" && call.args.len() == 2
        ));
        assert!(matches!(
            &math.nodes[20],
            MathNode::Call(call) if call.name == "overline" && call.args.len() == 1
        ));
        assert!(matches!(
            &math.nodes[24],
            MathNode::Attach(attach)
                if attach.top.is_some()
                && attach.bottom.is_some()
                && attach.top_left.is_some()
                && attach.top_right.is_some()
                && attach.bottom_left.is_some()
                && attach.bottom_right.is_some()
        ));
    }

    #[test]
    fn rejects_invalid_class_calls() {
        for (source, position, message) in [
            (
                "class(relation, !)",
                6,
                "math class name must be a string literal",
            ),
            ("class(\"unknown\", !)", 6, "unsupported math class name"),
            (
                "class(\"relation\")",
                0,
                "class math expects a class name and body",
            ),
        ] {
            let err = parse_math(source, 0).unwrap_err();
            assert_eq!(err, LabelError::UnsupportedSyntax { position, message });
        }
    }

    #[test]
    fn rejects_invalid_binom_calls() {
        let err = parse_math("binom(n)", 0).unwrap_err();
        assert_eq!(
            err,
            LabelError::UnsupportedSyntax {
                position: 0,
                message: "binom math expects upper and at least one lower argument"
            }
        );
    }

    #[test]
    fn parses_predefined_operator_call_names() {
        for name in [
            "arccos", "arcsin", "arctan", "arg", "cos", "cosh", "cot", "coth", "csc", "csch",
            "ctg", "deg", "det", "dim", "exp", "gcd", "lcm", "hom", "id", "im", "inf", "ker", "lg",
            "lim", "liminf", "limsup", "ln", "log", "max", "min", "mod", "Pr", "sec", "sech",
            "sin", "sinc", "sinh", "sup", "tan", "tanh", "tg", "tr",
        ] {
            let source = format!("{name}(x)");
            let math = parse(&source);
            assert!(
                matches!(&math.nodes[0], MathNode::Call(call) if call.name == name),
                "{source}"
            );
        }
    }

    #[test]
    fn rejects_invalid_op_options() {
        let err = parse_math("op(\"custom\", foo: #true)", 0).unwrap_err();
        assert!(format!("{err}").contains("unsupported op option"));

        let err = parse_math("op(\"custom\", limits: #auto)", 0).unwrap_err();
        assert!(format!("{err}").contains("unsupported op limits value"));
    }

    #[test]
    fn parses_math_size_calls_with_literal_cramped_option() {
        let math = parse("display(a/b) + inline(a/b) + script(a/b, cramped: #true) + sscript(a/b)");

        assert!(matches!(
            &math.nodes[0],
            MathNode::Call(call) if call.name == "display" && call.args.len() == 1
        ));
        assert!(matches!(
            &math.nodes[4],
            MathNode::Call(call) if call.name == "inline" && call.args.len() == 1
        ));
        assert!(matches!(
            &math.nodes[8],
            MathNode::Call(call) if call.name == "script" && call.args.len() == 1
        ));
        assert!(matches!(
            &math.nodes[12],
            MathNode::Call(call) if call.name == "sscript" && call.args.len() == 1
        ));
    }

    #[test]
    fn rejects_invalid_math_size_call_options() {
        for (source, position, message) in [
            (
                "script(a/b, tight: true)",
                17,
                "unsupported math size option",
            ),
            (
                "script(a/b, cramped: #auto)",
                12,
                "unsupported math size cramped value",
            ),
            (
                "script(a/b, c/d)",
                12,
                "math size call expects one body argument",
            ),
        ] {
            let err = parse_math(source, 0).unwrap_err();
            assert_eq!(err, LabelError::UnsupportedSyntax { position, message });
        }
    }

    #[test]
    fn leaves_unknown_function_like_identifiers_as_groups() {
        let math = parse("f(x)");

        assert!(matches!(
            &math.nodes[..],
            [MathNode::Identifier(_), MathNode::Group(_)]
        ));
    }

    #[test]
    fn rejects_matrix_calls() {
        for (source, position) in [
            ("mat(1, 2; 3, 4)", 10),
            ("vec(1, 2, 3)", 10),
            ("cases(x, y)", 10),
        ] {
            let err = parse_math(source, 10).unwrap_err();

            assert_eq!(
                err,
                LabelError::UnsupportedSyntax {
                    position,
                    message: "matrix/table math is not supported in Avenger Typst subset"
                },
                "{source}"
            );
        }
    }

    #[test]
    fn rejects_semicolon_arguments() {
        let err = parse_math("frac(1; 2)", 0).unwrap_err();

        assert_eq!(
            err,
            LabelError::UnsupportedSyntax {
                position: 6,
                message: "semicolon math arguments are not supported in Avenger Typst subset"
            }
        );
    }

    #[test]
    fn rejects_top_level_semicolon_math() {
        let err = parse_math("x; y", 5).unwrap_err();

        assert_eq!(
            err,
            LabelError::UnsupportedSyntax {
                position: 6,
                message: "semicolon math arguments are not supported in Avenger Typst subset"
            }
        );
    }

    #[test]
    fn rejects_multiline_math() {
        let err = parse_math("x\n+ y", 5).unwrap_err();

        assert_eq!(
            err,
            LabelError::UnsupportedSyntax {
                position: 6,
                message: "multi-line math is not supported in Avenger Typst subset"
            }
        );
    }

    #[test]
    fn rejects_alignment_markers() {
        let err = parse_math("x &= y", 5).unwrap_err();

        assert_eq!(
            err,
            LabelError::UnsupportedSyntax {
                position: 7,
                message: "math alignment markers are not supported in Avenger Typst subset"
            }
        );
    }

    #[test]
    fn rejects_named_call_arguments() {
        let err = parse_math("sqrt(num: x)", 5).unwrap_err();

        assert_eq!(
            err,
            LabelError::UnsupportedSyntax {
                position: 13,
                message: "named math arguments are not supported in Avenger Typst subset"
            }
        );
    }

    #[test]
    fn rejects_invalid_frac_options() {
        for (source, position, message) in [
            ("frac(x, y, foo: \"bar\")", 14, "unsupported frac option"),
            (
                "frac(x, y, style: \"diagonal\")",
                16,
                "unsupported frac style value",
            ),
            (
                "frac(x, y, style: #true)",
                16,
                "unsupported frac style value",
            ),
        ] {
            let err = parse_math(source, 0).unwrap_err();
            assert_eq!(err, LabelError::UnsupportedSyntax { position, message });
        }
    }

    #[test]
    fn rejects_unterminated_groups() {
        let err = parse_math("sqrt(x", 0).unwrap_err();

        assert!(matches!(err, LabelError::Syntax { .. }));
    }
}
