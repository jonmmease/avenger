use std::ops::Range;

use crate::delimiter::{MathDelimiterInfo, MathDelimiterOptions, MathDisplayHint};
use crate::error::MathTypesetError;

use super::ast::{
    EmojiAlias, LineNode, MathSpan, ParsedLine, PlainTextNode, TextMarkupKind, TextMarkupSpan,
};

use crate::syntax::ast::{self as typst_ast, AstNode};
use crate::syntax::{
    RangeMapper, RootedPath, SpanKind, SyntaxKind, SyntaxNode, VirtualPath, VirtualRoot,
};

pub(crate) fn parse_line(
    source: &str,
    _delimiters: &MathDelimiterOptions,
) -> Result<ParsedLine, MathTypesetError> {
    let mut root = crate::syntax::parse(source);
    synthesize_ranges(&mut root, source.len())?;
    reject_syntax_errors(&root)?;
    let markup = root
        .cast::<typst_ast::Markup>()
        .ok_or_else(|| MathTypesetError::Engine {
            start: 0,
            end: source.len(),
            message: "Typst parser did not return a markup root".to_string(),
        })?;

    let mut nodes = Vec::new();
    lower_markup(markup, source, &mut nodes)?;
    Ok(ParsedLine {
        source: source.to_string(),
        nodes,
    })
}

fn lower_markup(
    markup: typst_ast::Markup<'_>,
    source: &str,
    nodes: &mut Vec<LineNode>,
) -> Result<(), MathTypesetError> {
    for expr in markup.exprs() {
        lower_markup_expr(expr, source, nodes)?;
    }
    Ok(())
}

fn lower_markup_expr(
    expr: typst_ast::Expr<'_>,
    source: &str,
    nodes: &mut Vec<LineNode>,
) -> Result<(), MathTypesetError> {
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
            lower_static_call(call, source, nodes)?;
        }
        typst_ast::Expr::FieldAccess(access) => {
            lower_static_field_access(access, source, nodes)?;
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
    nodes: &mut Vec<LineNode>,
) -> Result<(), MathTypesetError> {
    let range = expand_hash_range(source, call.to_untyped().range());
    let Some(name) = code_expr_name(call.callee()) else {
        return Err(unsupported(range.start, "unsupported static text command"));
    };
    let Some(kind) = text_span_kind(&name) else {
        return Err(unsupported(range.start, "unsupported static text command"));
    };

    let mut body = None;
    for arg in call.args().items() {
        match arg {
            typst_ast::Arg::Pos(typst_ast::Expr::ContentBlock(block)) => {
                if body.replace(block).is_some() {
                    return Err(unsupported(
                        range.start,
                        "static text command expects one bracketed content block",
                    ));
                }
            }
            typst_ast::Arg::Named(_) => {
                return Err(unsupported(
                    range.start,
                    "static text commands do not support Typst-style options",
                ));
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

    let body_markup = body.body();
    let body_range = body_markup.to_untyped().range();
    let mut body_nodes = Vec::new();
    lower_markup(body_markup, source, &mut body_nodes)?;
    nodes.push(LineNode::TextSpan(TextMarkupSpan {
        kind,
        body: body_nodes,
        byte_range: range,
        body_range,
    }));
    Ok(())
}

fn lower_static_field_access(
    access: typst_ast::FieldAccess<'_>,
    source: &str,
    nodes: &mut Vec<LineNode>,
) -> Result<(), MathTypesetError> {
    let range = expand_hash_range(source, access.to_untyped().range());
    let Some(name) = code_field_access_name(access) else {
        return Err(unsupported(range.start, "unsupported static text command"));
    };
    let Some(alias) = name.strip_prefix("emoji.") else {
        return Err(unsupported(range.start, "unsupported static text command"));
    };
    let Some(emoji) = emoji_alias(alias) else {
        return Err(unsupported(range.start, "unknown emoji alias"));
    };
    nodes.push(LineNode::Emoji(EmojiAlias {
        name: alias.to_string(),
        emoji,
        byte_range: range,
    }));
    Ok(())
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
        _ => None,
    }
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

fn reject_syntax_errors(root: &SyntaxNode) -> Result<(), MathTypesetError> {
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
    Err(MathTypesetError::Syntax { position, message })
}

fn first_error_range(node: &SyntaxNode) -> Option<Range<usize>> {
    if node.kind() == SyntaxKind::Error {
        return Some(node.range());
    }
    node.children().find_map(first_error_range)
}

fn unsupported_markup_expr(source: &str, expr: typst_ast::Expr<'_>) -> MathTypesetError {
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

fn unsupported(position: usize, message: &'static str) -> MathTypesetError {
    MathTypesetError::UnsupportedSyntax { position, message }
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

fn synthesize_ranges(root: &mut SyntaxNode, source_len: usize) -> Result<(), MathTypesetError> {
    let mapper = RangeMapper::new([0..source_len]).map_err(|message| MathTypesetError::Engine {
        start: 0,
        end: source_len,
        message: message.to_string(),
    })?;
    root.synthesize_mapped(scratch_file_id(), &mapper)
        .map_err(|message| MathTypesetError::Engine {
            start: 0,
            end: source_len,
            message: message.to_string(),
        })
}

fn scratch_file_id() -> crate::syntax::FileId {
    RootedPath::new(
        VirtualRoot::Project,
        VirtualPath::new("avenger-typst-line.typ").expect("static virtual path is valid"),
    )
    .intern()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> ParsedLine {
        parse_line(source, &MathDelimiterOptions::default()).unwrap()
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
        let err = parse_line("cost $5", &MathDelimiterOptions::default()).unwrap_err();
        assert!(matches!(err, MathTypesetError::Syntax { .. }));
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
    fn rejects_static_command_options() {
        let err = parse_line(
            "#underline(stroke: red)[important]",
            &MathDelimiterOptions::default(),
        )
        .unwrap_err();

        assert_eq!(
            err,
            MathTypesetError::UnsupportedSyntax {
                position: 0,
                message: "static text commands do not support Typst-style options"
            }
        );
    }

    #[test]
    fn rejects_unknown_hash_commands() {
        let err = parse_line("#let x = 1", &MathDelimiterOptions::default()).unwrap_err();

        assert!(matches!(
            err,
            MathTypesetError::UnsupportedSyntax {
                position: 0,
                message: "unsupported static text command"
            }
        ));
    }

    #[test]
    fn rejects_unknown_emoji_aliases() {
        let err = parse_line("#emoji.not.real", &MathDelimiterOptions::default()).unwrap_err();

        assert_eq!(
            err,
            MathTypesetError::UnsupportedSyntax {
                position: 0,
                message: "unknown emoji alias"
            }
        );
    }
}
