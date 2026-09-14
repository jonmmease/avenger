use crate::label::LabelError;
use crate::typst_eval::call::{MathCallLoweringContext, lower_math_call};
use crate::typst_library::foundations::{Scope, Value};
use crate::typst_library::math::call::{
    is_math_differential_name, is_retained_math_name as is_retained_math_name_with,
};
use crate::typst_library::math::item::{
    MathArg, MathAst, MathAttach, MathCall, MathCallOptions, MathFraction, MathFractionStyle,
    MathGroup, MathIdentifier, MathNode, MathOperator, MathShorthand, MathSpace, MathSpacing,
    MathSpacingKind, MathStringLiteral, MathText, MathTextKind,
};
use crate::typst_library::symbols::{named_accent_char, named_symbol, normalize_accent_text};

use crate::typst_syntax::ast::{self as typst_ast, AstNode};
use crate::typst_syntax::{
    RangeMapper, RootedPath, SpanKind, SyntaxKind, SyntaxNode, VirtualPath, VirtualRoot,
};

#[cfg(test)]
pub(crate) fn parse_math(source: &str, offset: usize) -> Result<MathAst, LabelError> {
    parse_math_with_params(source, offset, &Scope::default())
}

pub(crate) fn parse_math_with_params(
    source: &str,
    offset: usize,
    params: &Scope,
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
    params: &Scope,
) -> Result<Vec<MathNode>, LabelError> {
    let mut nodes = Vec::new();
    for expr in math.exprs() {
        nodes.extend(lower_math_expr(expr, source, offset, params)?);
    }
    Ok(nodes)
}

struct MathEvalContext<'a> {
    source: &'a str,
    offset: usize,
    params: &'a Scope,
}

impl<'a> MathCallLoweringContext<'a> for MathEvalContext<'a> {
    fn source(&self) -> &'a str {
        self.source
    }

    fn offset(&self) -> usize {
        self.offset
    }

    fn lower_math_expr(&mut self, expr: typst_ast::Expr<'a>) -> Result<Vec<MathNode>, LabelError> {
        lower_math_expr(expr, self.source, self.offset, self.params)
    }

    fn lower_math_expr_as_single(
        &mut self,
        expr: typst_ast::Expr<'a>,
    ) -> Result<MathNode, LabelError> {
        lower_math_expr_as_single(expr, self.source, self.offset, self.params)
    }

    fn named_math_symbol(&self, name: &str) -> Option<&'static str> {
        named_symbol(name)
    }

    fn named_accent_char(&self, name: &str) -> Option<char> {
        named_accent_char(name)
    }

    fn normalize_accent_text(&self, value: &str) -> Option<char> {
        normalize_accent_text(value)
    }

    fn predefined_operator_text(&self, name: &str) -> Option<&'static str> {
        predefined_operator_text(name)
    }
}

fn lower_math_expr(
    expr: typst_ast::Expr<'_>,
    source: &str,
    offset: usize,
    params: &Scope,
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
                        symbol: named_symbol(value),
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
            let byte_range = offset_range(ident.to_untyped().range(), offset);
            if let Some(kind) = math_spacing_kind(&name) {
                return Ok(vec![MathNode::Spacing(MathSpacing {
                    kind,
                    weak: false,
                    byte_range,
                })]);
            }
            if is_math_differential_name(&name) {
                return Ok(math_differential_nodes(&name, byte_range));
            }
            Ok(vec![MathNode::Identifier(MathIdentifier {
                symbol: named_symbol(&name),
                name,
                byte_range,
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
                .map(|(script, continuation)| (Some(script), continuation))
                .unwrap_or((None, Vec::new()));
            let (bottom, bottom_continuation) = attach
                .bottom()
                .map(|expr| lower_script_expr(expr, source, offset, params))
                .transpose()?
                .map(|(script, continuation)| (Some(script), continuation))
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
        typst_ast::Expr::MathCall(call) => {
            let mut ctx = MathEvalContext {
                source,
                offset,
                params,
            };
            lower_math_call(call, &mut ctx)
        }
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
    params: &Scope,
) -> Result<Vec<MathNode>, LabelError> {
    let local_range = expand_hash_range(source, ident.to_untyped().range());
    let range = offset_range(local_range, offset);
    let Some(value) = params.get(ident.as_str()) else {
        return Err(unsupported(range.start, "unknown label parameter"));
    };
    scope_value_to_math_nodes(value, range)
}

fn scope_value_to_math_nodes(
    value: &Value,
    byte_range: std::ops::Range<usize>,
) -> Result<Vec<MathNode>, LabelError> {
    match value {
        Value::None => Ok(Vec::new()),
        Value::Bool(value) => Ok(vec![math_text(
            value.to_string(),
            MathTextKind::Grapheme,
            byte_range,
        )]),
        Value::Int(value) => Ok(vec![math_text(
            value.to_string(),
            MathTextKind::Number,
            byte_range,
        )]),
        Value::Float(value) if value.is_finite() => Ok(vec![math_text(
            format_f64(*value),
            MathTextKind::Number,
            byte_range,
        )]),
        Value::Float(_) => Err(LabelError::UnsupportedSyntax {
            position: byte_range.start,
            message: "non-finite label parameter is not supported",
        }),
        Value::Str(value) => Ok(vec![math_text(
            value.clone(),
            if is_plain_numeric_text(value) {
                MathTextKind::Number
            } else {
                MathTextKind::Grapheme
            },
            byte_range,
        )]),
        Value::Date(value) => Ok(vec![math_text(
            value.to_string(),
            MathTextKind::Grapheme,
            byte_range,
        )]),
        Value::DateTime(value) => Ok(vec![math_text(
            value.to_string(),
            MathTextKind::Grapheme,
            byte_range,
        )]),
        Value::UtcDateTime(value) => Ok(vec![math_text(
            value.to_rfc3339(),
            MathTextKind::Grapheme,
            byte_range,
        )]),
        Value::Array(_) | Value::Dict(_) => Err(LabelError::UnsupportedSyntax {
            position: byte_range.start,
            message: "label parameter value cannot be rendered as math",
        }),
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

fn math_spacing_kind(name: &str) -> Option<MathSpacingKind> {
    match name {
        "thin" => Some(MathSpacingKind::Thin),
        "med" => Some(MathSpacingKind::Medium),
        "thick" => Some(MathSpacingKind::Thick),
        "quad" => Some(MathSpacingKind::Quad),
        "wide" => Some(MathSpacingKind::Wide),
        _ => None,
    }
}

fn math_differential_nodes(name: &str, byte_range: std::ops::Range<usize>) -> Vec<MathNode> {
    let letter = if name == "Dif" { "D" } else { "d" };
    vec![
        MathNode::Spacing(MathSpacing {
            kind: MathSpacingKind::Thin,
            weak: true,
            byte_range: byte_range.clone(),
        }),
        MathNode::Text(MathText {
            text: letter.to_string(),
            kind: MathTextKind::Upright,
            byte_range,
        }),
    ]
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
    params: &Scope,
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
    params: &Scope,
) -> Result<(Vec<MathNode>, Vec<MathNode>), LabelError> {
    let range = expr.to_untyped().range();
    if script_expr_is_parenthesized_group(source, range.clone()) {
        let mut nodes = lower_math_expr(expr, source, offset, params)?;
        nodes.retain(|node| !matches!(node, MathNode::Space(_)));
        if nodes.is_empty() {
            return Err(unsupported(
                range.start + offset,
                "math script expects an expression",
            ));
        }
        return Ok((nodes, Vec::new()));
    }

    let mut nodes = lower_math_expr(expr, source, offset, params)?;
    nodes.retain(|node| !matches!(node, MathNode::Space(_)));
    if nodes.is_empty() {
        return Err(unsupported(
            range.start + offset,
            "math script expects an expression",
        ));
    }
    let script = nodes.remove(0);
    Ok((vec![script], nodes))
}

fn script_expr_is_parenthesized_group(source: &str, range: std::ops::Range<usize>) -> bool {
    let text = &source[range.clone()];
    (text.starts_with('(') && text.ends_with(')'))
        || (range.start > 0
            && range.end < source.len()
            && source[..range.start].ends_with('(')
            && source[range.end..].starts_with(')'))
}

fn last_node_byte_range(nodes: &[MathNode]) -> Option<std::ops::Range<usize>> {
    nodes.last().map(MathNode::byte_range)
}

fn lower_math_root(
    root: typst_ast::MathRoot<'_>,
    source: &str,
    offset: usize,
    params: &Scope,
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
                symbol: named_symbol(&name),
                name,
                byte_range: offset_range(ident.to_untyped().range(), offset),
            })])
        }
        typst_ast::MathAccess::MathFieldAccess(access) => {
            let full_name = math_field_access_name(access);
            if named_symbol(&full_name).is_some() {
                return Ok(vec![MathNode::Identifier(MathIdentifier {
                    symbol: named_symbol(&full_name),
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
                symbol: named_symbol(&field_name),
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
    let mapper =
        RangeMapper::new(std::iter::once(0..source_len)).map_err(|message| LabelError::Engine {
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

pub(crate) fn is_retained_math_name(name: &str) -> bool {
    is_retained_math_name_with(
        name,
        |name| predefined_operator_text(name).is_some(),
        |name| named_accent_char(name).is_some(),
        |name| named_symbol(name).is_some(),
    )
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
        let mut values = indexmap::IndexMap::new();
        values.insert("slope".to_string(), Value::Float(2.5));
        values.insert("intercept".to_string(), Value::Int(7));
        let params = Scope::new(values);

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
    fn parses_explicit_spacings_and_differentials() {
        let math = parse("a thin b med c thick d quad e wide f");
        let spacings: Vec<_> = math
            .nodes
            .iter()
            .filter_map(|node| match node {
                MathNode::Spacing(spacing) => Some(spacing.kind),
                _ => None,
            })
            .collect();
        assert_eq!(
            spacings,
            [
                MathSpacingKind::Thin,
                MathSpacingKind::Medium,
                MathSpacingKind::Thick,
                MathSpacingKind::Quad,
                MathSpacingKind::Wide,
            ]
        );

        let math = parse("x dif y Dif z");
        let weak_spacings = math
            .nodes
            .iter()
            .filter(|node| matches!(node, MathNode::Spacing(spacing) if spacing.weak))
            .count();
        let upright_text: String = math
            .nodes
            .iter()
            .filter_map(|node| match node {
                MathNode::Text(text) if text.kind == MathTextKind::Upright => {
                    Some(text.text.as_str())
                }
                _ => None,
            })
            .collect();

        assert_eq!(weak_spacings, 2);
        assert_eq!(upright_text, "dD");
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
            "bar(x)",
            "bar.double(x)",
            "mid(slash)",
            "mid(bar.v.double)",
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
            "bar(x)",
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
    fn resolves_named_mid_delimiter_symbols() {
        for (source, expected) in [("mid(slash)", "/"), ("mid(bar.v.double)", "‖")] {
            let math = parse(source);
            let [MathNode::Call(call)] = &math.nodes[..] else {
                panic!("{source} should lower to a mid call");
            };
            assert_eq!(call.name, "mid");
            let [arg] = &call.args[..] else {
                panic!("{source} should retain one mid argument");
            };
            let [MathNode::Identifier(identifier)] = &arg.nodes[..] else {
                panic!("{source} should retain one named delimiter identifier");
            };
            assert_eq!(identifier.symbol, Some(expected));
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
    fn parses_parenthesized_script_as_one_script_body() {
        let grouped = parse("sum_(i=0)^n");
        let [MathNode::Attach(grouped_attach)] = &grouped.nodes[..] else {
            panic!("grouped sum script should lower to one attachment");
        };
        assert_eq!(grouped_attach.bottom.as_ref().map(Vec::len), Some(3));

        let ungrouped = parse("sum_i=0");
        let [
            MathNode::Attach(ungrouped_attach),
            MathNode::Operator(operator),
            MathNode::Text(number),
        ] = &ungrouped.nodes[..]
        else {
            panic!("ungrouped script continuation should remain outside the attachment");
        };
        assert_eq!(ungrouped_attach.bottom.as_ref().map(Vec::len), Some(1));
        assert_eq!(operator.operator, "=");
        assert_eq!(number.text, "0");
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
            Some(crate::typst_library::Color::rgba(
                1.0,
                65.0 / 255.0,
                54.0 / 255.0,
                1.0
            ))
        );
        assert_eq!(
            cancel.options.stroke.thickness,
            Some(crate::typst_library::text::content::DecorationLength::Em(
                0.25,
            ))
        );
        assert_eq!(
            cancel.options.stroke.line_cap,
            Some(crate::typst_svg::LineCap::Round)
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
            (
                "cancel(x, stroke: #(paint: gradient.linear(red, blue)))",
                "unsupported decoration paint",
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
        let math = parse("hat(dotless: #false, size: #150%, i) + accent(dotless: #true, j, \".\")");

        assert!(matches!(
            &math.nodes[0],
            MathNode::Accent(accent)
                if accent.accent == '\u{0302}'
                    && !accent.dotless
                    && (accent.size.relative - 1.5).abs() < f32::EPSILON
        ));
        assert!(matches!(
            &math.nodes[4],
            MathNode::Accent(accent) if accent.accent == '\u{0307}' && accent.dotless
        ));
    }

    #[test]
    fn rejects_unsupported_accent_options() {
        for (source, message) in [
            ("hat(x, size: #auto)", "unsupported accent size value"),
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
        let math = parse(
            "alpha -> RR + in.not + subset.eq + arrow.r.double + arrow.double.r + forces.not",
        );

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
        assert!(math.nodes.iter().any(|node| matches!(
            node,
            MathNode::Identifier(ident)
                if ident.name == "arrow.double.r" && ident.symbol == Some("⇒")
        )));
        assert!(math.nodes.iter().any(|node| matches!(
            node,
            MathNode::Identifier(ident)
                if ident.name == "forces.not" && ident.symbol == Some("⊮")
        )));
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
                && identifier.symbol == Some("→")
                && operator.operator == "."
                && suffix.name == "unknown"
        ));
    }

    #[test]
    fn parses_whitelisted_function_calls() {
        let math = parse(
            "frac(x, y) + op(\"custom\", limits: #true) + bb(R) + scr(P) + class(\"relation\", !) + overline(underline(x)) + overbrace(x, \"note\") + underparen(y, alpha) + attach(Pi, t: alpha, b: beta, tl: 1, tr: 2+3, bl: 4+5, br: 6)",
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
        assert!(math.nodes.iter().any(|node| matches!(
            node,
            MathNode::Call(call) if call.name == "overbrace" && call.args.len() == 2
        )));
        assert!(math.nodes.iter().any(|node| matches!(
            node,
            MathNode::Call(call) if call.name == "underparen" && call.args.len() == 2
        )));
        assert!(matches!(
            &math.nodes[32],
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
        for (source, feature, position) in [
            ("mat(1, 2; 3, 4)", "mat", 10),
            ("vec(1, 2, 3)", "vec", 10),
            ("cases(x, y)", "cases", 10),
        ] {
            let err = parse_math(source, 10).unwrap_err();

            assert_eq!(
                err,
                LabelError::UnsupportedFeature {
                    position,
                    feature: feature.to_string(),
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
