use std::ops::Range;

use crate::label::LabelError;
use crate::typst_eval::delimiter::{DelimiterDisplayHint, DelimiterInfo};
use crate::typst_library::foundations::Scope;
use crate::typst_library::symbols::{named_emoji, named_symbol};
use crate::typst_library::text::call::{parse_text_markup_option, text_span_kind};
use crate::typst_library::text::content::{
    EmojiAlias, LabelContent, LabelParamRef, LineNode, MathSpan, PlainTextNode, SmartQuoteNode,
    SymbolAlias, TextMarkupKind, TextMarkupOptions, TextMarkupSpan,
};
use crate::typst_library::text::smartquote::SmartQuote;

use crate::typst_syntax::ast::{self as typst_ast, AstNode};
use crate::typst_syntax::{
    RangeMapper, RootedPath, SpanKind, SyntaxKind, SyntaxNode, VirtualPath, VirtualRoot,
};

#[cfg(test)]
pub(crate) fn parse_line(source: &str) -> Result<LabelContent, LabelError> {
    parse_line_with_params(source, &Scope::default())
}

pub(crate) fn parse_line_with_params(
    source: &str,
    params: &Scope,
) -> Result<LabelContent, LabelError> {
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
    Ok(LabelContent {
        source: source.to_string(),
        nodes,
    })
}

fn lower_markup(
    markup: typst_ast::Markup<'_>,
    source: &str,
    params: &Scope,
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
    params: &Scope,
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
            nodes.push(LineNode::SmartQuote(SmartQuoteNode {
                quote: SmartQuote {
                    double: quote.double(),
                },
                byte_range: node.range(),
            }));
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
    params: &Scope,
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
    params: &Scope,
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
        let Some(emoji) = named_emoji(alias) else {
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
        let Some(text) = named_symbol(alias) else {
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

fn math_delimiter_info(full_range: Range<usize>) -> DelimiterInfo {
    DelimiterInfo {
        opening_range: full_range.start..full_range.start + 1,
        closing_range: full_range.end.saturating_sub(1)..full_range.end,
        full_range,
        display_hint: DelimiterDisplayHint::Inline,
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
    use crate::typst_library::Color;
    use crate::typst_library::foundations::Value;
    use crate::typst_library::text::content::DecorationLength;
    use crate::typst_svg::{LineCap, LineJoin};

    fn parse(source: &str) -> LabelContent {
        parse_line(source).unwrap()
    }

    fn parse_with_params(source: &str, params: &Scope) -> LabelContent {
        parse_line_with_params(source, params).unwrap()
    }

    fn scope(values: impl IntoIterator<Item = (&'static str, Value)>) -> Scope {
        Scope::new(
            values
                .into_iter()
                .map(|(name, value)| (name.to_string(), value))
                .collect(),
        )
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
        let line = parse("Revenue #emoji.rocket #emoji.chart.up #emoji.face.halo");

        assert_eq!(line.nodes.len(), 6);
        assert!(
            matches!(&line.nodes[1], LineNode::Emoji(alias) if alias.name == "rocket" && alias.emoji == "🚀")
        );
        assert!(
            matches!(&line.nodes[3], LineNode::Emoji(alias) if alias.name == "chart.up" && alias.emoji == "📈")
        );
        assert!(
            matches!(&line.nodes[5], LineNode::Emoji(alias) if alias.name == "face.halo" && alias.emoji == "😇")
        );
    }

    #[test]
    fn parses_named_symbol_aliases() {
        let line = parse(
            "Flow #sym.arrow.r target #sym.gt.eq.not #sym.arrow.double.r #sym.forces.not #sym.gender.male.stroke.t",
        );

        assert_eq!(line.nodes.len(), 10);
        assert!(
            matches!(&line.nodes[1], LineNode::Symbol(alias) if alias.name == "arrow.r" && alias.text == "→")
        );
        assert!(
            matches!(&line.nodes[3], LineNode::Symbol(alias) if alias.name == "gt.eq.not" && alias.text == "≱")
        );
        assert!(
            matches!(&line.nodes[5], LineNode::Symbol(alias) if alias.name == "arrow.double.r" && alias.text == "⇒")
        );
        assert!(
            matches!(&line.nodes[7], LineNode::Symbol(alias) if alias.name == "forces.not" && alias.text == "⊮")
        );
        assert!(
            matches!(&line.nodes[9], LineNode::Symbol(alias) if alias.name == "gender.male.stroke.t" && alias.text == "⚨")
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
        let params = scope([
            ("underline_offset", Value::Str("2pt".to_string())),
            ("underline_extent", Value::Str("-0.5em".to_string())),
            ("underline_background", Value::Bool(true)),
            ("underline_evade", Value::Bool(false)),
        ]);

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
        let params = scope([
            ("use_typographic", Value::Bool(false)),
            ("script_baseline", Value::Str("-0.25em".to_string())),
            ("script_size", Value::Str("8pt".to_string())),
            ("all_caps", Value::Bool(true)),
            ("weight_delta", Value::Int(150)),
        ]);

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
        let params = scope([("badlength", Value::Bool(true))]);

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

        let err = parse_line("#underline(stroke: (miter-limit: 2pt))[group]").unwrap_err();

        assert_eq!(
            err,
            LabelError::UnsupportedSyntax {
                position: 20,
                message: "unsupported stroke miter limit"
            }
        );

        let err =
            parse_line("#underline(stroke: 1pt + gradient.linear(red, blue))[group]").unwrap_err();

        assert_eq!(
            err,
            LabelError::UnsupportedSyntax {
                position: 11,
                message: "unsupported decoration paint"
            }
        );

        let err = parse_line("#underline(stroke: pattern())[group]").unwrap_err();

        assert_eq!(
            err,
            LabelError::UnsupportedSyntax {
                position: 11,
                message: "unsupported decoration paint"
            }
        );
    }

    #[test]
    fn parses_stroke_cap_join_and_dash() {
        let line = parse(
            "#underline(stroke: (cap: \"round\", join: \"bevel\", dash: (array: (2pt, \"dot\"), phase: 0.5pt), miter-limit: 2))[x]",
        );

        let LineNode::TextSpan(span) = &line.nodes[0] else {
            panic!("expected text span");
        };
        assert_eq!(
            span.options.decoration.stroke.line_cap,
            Some(LineCap::Round)
        );
        assert_eq!(
            span.options.decoration.stroke.line_join,
            Some(LineJoin::Bevel)
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
        assert_eq!(
            span.options
                .decoration
                .stroke
                .dash
                .as_ref()
                .map(|dash| dash.phase),
            Some(DecorationLength::Pt(0.5))
        );
        assert_eq!(span.options.decoration.stroke.miter_limit, Some(2.0));
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
