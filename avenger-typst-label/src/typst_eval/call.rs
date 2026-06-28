use std::ops::Range;

use crate::label::LabelError;
use crate::typst_library::math::call::{
    is_math_accent_call_name, is_math_call_name, is_math_delimiter_helper_call_name,
    is_math_delimiter_symbol_call_name, is_math_size_call_name, is_math_under_over_call_name,
    is_unsupported_math_table_call_name,
};
use crate::typst_library::math::item as math_item;
use crate::typst_library::math::item::{
    MathAccent, MathArg, MathAttach, MathCall, MathCallOptions, MathCancel, MathCancelAngle,
    MathCancelLength, MathCancelOptions, MathDelimitedSize, MathFraction, MathFractionStyle,
    MathGroup, MathIdentifier, MathNode, MathOperator, MathSpace, MathStretchSize,
};
use crate::typst_library::text::call::parse_decoration_stroke;

use crate::typst_syntax::ast::{self as typst_ast, AstNode};
use crate::typst_syntax::{SpanKind, SyntaxNode};

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
        return Err(unsupported_feature(
            range.start,
            &name,
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
        } else if is_math_under_over_call_name(&name) {
            validate_math_under_over_call_args(&args, range.start)?;
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
    let mut size = math_item::MathAccentSize::default();
    let mut saw_size = false;
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
                        if saw_size {
                            return Err(unsupported(position, "duplicate accent size option"));
                        }
                        size = parse_math_stretch_size(
                            named.expr(),
                            source,
                            position,
                            "unsupported accent size value",
                        )?;
                        saw_size = true;
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
        size,
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
    let mut saw_named = false;
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
                saw_named = true;
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
    if !saw_named {
        let byte_range = offset_range(args.to_untyped().range(), offset);
        let nodes = lower_math_args_as_group_body(args, ctx)?;
        if nodes.is_empty() {
            return Err(unsupported(
                position,
                "math size call expects one body argument",
            ));
        }
        return Ok(vec![MathArg { nodes, byte_range }]);
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

pub(crate) fn validate_math_under_over_call_args(
    args: &[MathArg],
    position: usize,
) -> Result<(), LabelError> {
    if !(1..=2).contains(&args.len()) {
        return Err(unsupported(
            position,
            "under/over math calls require a body and optional annotation",
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

fn unsupported_feature(position: usize, feature: &str, message: &'static str) -> LabelError {
    LabelError::UnsupportedFeature {
        position,
        feature: feature.to_string(),
        message,
    }
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
