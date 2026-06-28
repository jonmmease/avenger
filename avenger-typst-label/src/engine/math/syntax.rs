use crate::error::LabelError;

use super::ast::{
    MathArg, MathAst, MathAttach, MathCall, MathFraction, MathGroup, MathIdentifier, MathNode,
    MathOperator, MathShorthand, MathSpace, MathStringLiteral, MathText, MathTextKind,
};

use crate::syntax::ast::{self as typst_ast, AstNode};
use crate::syntax::{
    RangeMapper, RootedPath, SpanKind, SyntaxKind, SyntaxNode, VirtualPath, VirtualRoot,
};

pub(crate) fn parse_math(source: &str, offset: usize) -> Result<MathAst, LabelError> {
    if let Some((idx, _)) = source
        .char_indices()
        .find(|(_, ch)| matches!(ch, '\n' | '\r'))
    {
        return Err(unsupported(
            offset + idx,
            "multi-line math is not supported in Avenger Typst subset",
        ));
    }
    let mut root = crate::syntax::parse_math(source);
    synthesize_ranges(&mut root, source.len(), offset)?;
    reject_syntax_errors(&root, offset)?;
    let math = root
        .cast::<typst_ast::Math>()
        .ok_or_else(|| LabelError::Engine {
            start: offset,
            end: offset + source.len(),
            message: "Typst parser did not return a math root".to_string(),
        })?;
    let nodes = lower_math(math, source, offset)?;
    Ok(MathAst {
        source: source.to_string(),
        nodes,
    })
}

fn lower_math(
    math: typst_ast::Math<'_>,
    source: &str,
    offset: usize,
) -> Result<Vec<MathNode>, LabelError> {
    let mut nodes = Vec::new();
    for expr in math.exprs() {
        nodes.extend(lower_math_expr(expr, source, offset)?);
    }
    Ok(nodes)
}

fn lower_math_expr(
    expr: typst_ast::Expr<'_>,
    source: &str,
    offset: usize,
) -> Result<Vec<MathNode>, LabelError> {
    let range = expr.to_untyped().range();
    match expr {
        typst_ast::Expr::Math(math) => lower_math(math, source, offset),
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
                parse_math(&source[body_range.clone()], offset + body_range.start)?.nodes
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
            let base = lower_math_expr_as_single(attach.base(), source, offset)?;
            let (top, mut continuation) = attach
                .top()
                .map(|expr| lower_script_expr(expr, source, offset))
                .transpose()?
                .map(|(script, continuation)| (Some(vec![script]), continuation))
                .unwrap_or((None, Vec::new()));
            let (bottom, bottom_continuation) = attach
                .bottom()
                .map(|expr| lower_script_expr(expr, source, offset))
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
            let numerator = lower_math_expr_as_single(frac.num(), source, offset)?;
            let denominator = lower_math_expr_as_single(frac.denom(), source, offset)?;
            let slash_range = slash_range_between(
                source,
                frac.num().to_untyped().range().end,
                frac.denom().to_untyped().range().start,
                offset,
            );
            let byte_range = numerator.byte_range().start..denominator.byte_range().end;
            Ok(vec![MathNode::Fraction(MathFraction {
                numerator: Box::new(numerator),
                denominator: Box::new(denominator),
                slash_range,
                byte_range,
            })])
        }
        typst_ast::Expr::MathRoot(root) => lower_math_root(root, source, offset),
        typst_ast::Expr::MathCall(call) => lower_math_call(call, source, offset),
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

fn lower_math_expr_as_single(
    expr: typst_ast::Expr<'_>,
    source: &str,
    offset: usize,
) -> Result<MathNode, LabelError> {
    let range = expr.to_untyped().range();
    let mut nodes = lower_math_expr(expr, source, offset)?;
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
) -> Result<(MathNode, Vec<MathNode>), LabelError> {
    let range = expr.to_untyped().range();
    let mut nodes = lower_math_expr(expr, source, offset)?;
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
) -> Result<Vec<MathNode>, LabelError> {
    let radicand = lower_math_expr_as_single(root.radicand(), source, offset)?;
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
        byte_range: offset_range(root.to_untyped().range(), offset),
    })])
}

fn lower_math_call(
    call: typst_ast::MathCall<'_>,
    source: &str,
    offset: usize,
) -> Result<Vec<MathNode>, LabelError> {
    let name = math_access_name(call.callee());
    let range = offset_range(call.to_untyped().range(), offset);
    if is_unsupported_math_table_call_name(&name) {
        return Err(unsupported(
            range.start,
            "matrix/table math is not supported in Avenger Typst subset",
        ));
    }

    if name == "attach" {
        return lower_math_attach_call(call.args(), source, offset, range);
    }

    if is_math_size_call_name(&name) {
        let args = lower_math_size_call_args(call.args(), source, offset, range.start)?;
        return Ok(vec![MathNode::Call(MathCall {
            name,
            args,
            byte_range: range,
        })]);
    }

    if is_math_call_name(&name) {
        let args = lower_math_call_args(call.args(), source, offset)?;
        if name == "class" {
            validate_math_class_call_args(&args, range.start)?;
        }
        return Ok(vec![MathNode::Call(MathCall {
            name,
            args,
            byte_range: range,
        })]);
    }

    let mut nodes = vec![MathNode::Identifier(MathIdentifier {
        symbol: named_math_symbol(&name),
        name,
        byte_range: offset_range(call.callee().to_untyped().range(), offset),
    })];
    let args_range = offset_range(call.args().to_untyped().range(), offset);
    let body = lower_math_args_as_group_body(call.args(), source, offset)?;
    nodes.push(MathNode::Group(MathGroup {
        left: '(',
        right: ')',
        body,
        byte_range: args_range,
    }));
    Ok(nodes)
}

fn lower_math_attach_call(
    args: typst_ast::MathArgs<'_>,
    source: &str,
    offset: usize,
    range: std::ops::Range<usize>,
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
                base = Some(lower_math_expr_as_single(expr, source, offset)?);
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
                let nodes = lower_math_expr(expr, source, offset)?;
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

fn lower_math_call_args(
    args: typst_ast::MathArgs<'_>,
    source: &str,
    offset: usize,
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
                    nodes: lower_math_expr(expr, source, offset)?,
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
                    nodes: lower_math_expr(expr, source, offset)?,
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
                parse_math_bool_literal(named.expr(), item_position)?;
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

fn parse_math_bool_literal(expr: typst_ast::Expr<'_>, position: usize) -> Result<bool, LabelError> {
    match expr {
        typst_ast::Expr::Bool(value) => Ok(value.get()),
        typst_ast::Expr::CodeBlock(block) => {
            let exprs = block.body().exprs().collect::<Vec<_>>();
            let [typst_ast::Expr::Bool(value)] = &exprs[..] else {
                return Err(unsupported(position, "unsupported math size cramped value"));
            };
            Ok(value.get())
        }
        _ => Err(unsupported(position, "unsupported math size cramped value")),
    }
}

fn validate_math_class_call_args(args: &[MathArg], position: usize) -> Result<(), LabelError> {
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

fn lower_math_args_as_group_body(
    args: typst_ast::MathArgs<'_>,
    source: &str,
    offset: usize,
) -> Result<Vec<MathNode>, LabelError> {
    let mut body = Vec::new();
    for item in args.content_items() {
        match item {
            typst_ast::MathArgItem::Arg(typst_ast::Arg::Pos(expr)) => {
                body.extend(lower_math_expr(expr, source, offset)?);
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

fn named_argument_position(named: typst_ast::Named<'_>, source: &str, offset: usize) -> usize {
    let range = named.to_untyped().range();
    source[range.clone()]
        .find(':')
        .map(|idx| offset + range.start + idx)
        .unwrap_or(offset + range.start)
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

fn scratch_file_id() -> crate::syntax::FileId {
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

fn is_math_call_name(name: &str) -> bool {
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
            | "cancel"
            | "class"
            | "underline"
            | "overline"
            | "op"
            | "sin"
            | "cos"
            | "tan"
            | "log"
            | "ln"
            | "lim"
            | "max"
            | "min"
            | "hat"
            | "tilde"
            | "dot"
            | "ddot"
            | "bar"
            | "arrow"
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
    )
}

fn is_unsupported_math_table_call_name(name: &str) -> bool {
    matches!(name, "mat" | "vec" | "cases")
}

fn is_math_size_call_name(name: &str) -> bool {
    matches!(name, "display" | "inline" | "script" | "sscript")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> MathAst {
        parse_math(source, 0).unwrap()
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
            "sin(x)",
            "op(\"custom\")",
            "abs(x)",
            "norm(v)",
            "floor(x)",
            "ceil(x)",
            "round(x)",
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
            fraction.numerator.as_ref(),
            MathNode::Call(call) if call.name == "sqrt"
        ));
        assert!(matches!(
            fraction.denominator.as_ref(),
            MathNode::Group(group) if group.left == '(' && group.right == ')'
        ));
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
            "frac(x, y) + op(\"custom\") + bb(R) + scr(P) + class(\"relation\", !) + overline(underline(x)) + attach(Pi, t: alpha, b: beta, tl: 1, tr: 2+3, bl: 4+5, br: 6)",
        );

        assert!(matches!(
            &math.nodes[0],
            MathNode::Call(call) if call.name == "frac" && call.args.len() == 2
        ));
        assert!(matches!(
            &math.nodes[4],
            MathNode::Call(call) if call.name == "op" && call.args.len() == 1
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
        let err = parse_math("frac(num: x, denom: y)", 5).unwrap_err();

        assert_eq!(
            err,
            LabelError::UnsupportedSyntax {
                position: 13,
                message: "named math arguments are not supported in Avenger Typst subset"
            }
        );
    }

    #[test]
    fn rejects_unterminated_groups() {
        let err = parse_math("sqrt(x", 0).unwrap_err();

        assert!(matches!(err, LabelError::Syntax { .. }));
    }
}
