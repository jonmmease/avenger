use indexmap::IndexSet;

use crate::label::LabelError;
use crate::typst_library::foundations::Scope;
use crate::typst_library::text::call::{
    is_retained_markup_name, parse_text_markup_option, text_span_kind,
};
use crate::typst_library::text::content::{TextMarkupKind, TextMarkupOptions};
use crate::typst_syntax::ast::{self as typst_ast, AstNode};
use crate::typst_syntax::{
    RangeMapper, RootedPath, SpanKind, SyntaxKind, SyntaxNode, VirtualPath, VirtualRoot,
};

pub(crate) fn referenced_params(source: &str) -> Result<Vec<String>, LabelError> {
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

    let mut names = IndexSet::new();
    collect_markup(markup, source, &mut names)?;
    Ok(names.into_iter().collect())
}

fn collect_markup(
    markup: typst_ast::Markup<'_>,
    source: &str,
    names: &mut IndexSet<String>,
) -> Result<(), LabelError> {
    for expr in markup.exprs() {
        collect_markup_expr(expr, source, names)?;
    }
    Ok(())
}

fn collect_markup_expr(
    expr: typst_ast::Expr<'_>,
    source: &str,
    names: &mut IndexSet<String>,
) -> Result<(), LabelError> {
    match expr {
        typst_ast::Expr::Text(_)
        | typst_ast::Expr::Space(_)
        | typst_ast::Expr::Escape(_)
        | typst_ast::Expr::Shorthand(_)
        | typst_ast::Expr::SmartQuote(_)
        | typst_ast::Expr::Raw(_) => {}
        typst_ast::Expr::Equation(equation) => {
            if equation.block() {
                return Err(unsupported(
                    equation.to_untyped().range().start,
                    "display math is not supported in Avenger text lines",
                ));
            }
            let body = equation.body();
            let body_range = body.to_untyped().range();
            collect_math_source(&source[body_range.clone()], body_range.start, names)?;
        }
        typst_ast::Expr::FuncCall(call) => {
            collect_static_call(call, source, names)?;
        }
        typst_ast::Expr::Ident(ident) => {
            names.insert(ident.as_str().to_string());
        }
        typst_ast::Expr::FieldAccess(access) => {
            let range = expand_hash_range(source, access.to_untyped().range());
            let Some(name) = code_field_access_name(access) else {
                return Err(unsupported(range.start, "unsupported static text command"));
            };
            if name.starts_with("emoji.") || name.starts_with("sym.") {
                return Ok(());
            }
            return Err(unsupported(range.start, "unsupported static text command"));
        }
        typst_ast::Expr::Strong(strong) => collect_markup(strong.body(), source, names)?,
        typst_ast::Expr::Emph(emph) => collect_markup(emph.body(), source, names)?,
        other => {
            return Err(unsupported_markup_expr(source, other));
        }
    }
    Ok(())
}

fn collect_static_call(
    call: typst_ast::FuncCall<'_>,
    source: &str,
    names: &mut IndexSet<String>,
) -> Result<(), LabelError> {
    let range = expand_hash_range(source, call.to_untyped().range());
    let Some(name) = code_expr_name(call.callee()) else {
        return Err(unsupported(range.start, "unsupported static text command"));
    };
    if name == "numfmt" || name == "datefmt" {
        collect_format_call(call, names);
        return Ok(());
    }
    let Some(kind) = text_span_kind(&name) else {
        return Err(unsupported(range.start, "unsupported static text command"));
    };

    let mut options = TextMarkupOptions::default();
    for arg in call.args().items() {
        match arg {
            typst_ast::Arg::Pos(typst_ast::Expr::ContentBlock(block)) => {
                if kind == TextMarkupKind::Raw {
                    return Err(unsupported(range.start, "raw expects a string literal"));
                }
                collect_markup(block.body(), source, names)?;
            }
            typst_ast::Arg::Pos(typst_ast::Expr::Str(_))
                if kind.is_case_transform() || kind == TextMarkupKind::Raw => {}
            typst_ast::Arg::Named(named) => {
                let empty = Scope::default();
                if parse_text_markup_option(kind, named, &empty, &mut options).is_err() {
                    collect_unknown_code_idents(named.expr(), names);
                }
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
    Ok(())
}

fn collect_format_call(call: typst_ast::FuncCall<'_>, names: &mut IndexSet<String>) {
    for arg in call.args().items() {
        match arg {
            typst_ast::Arg::Pos(expr) => collect_unknown_code_idents(expr, names),
            typst_ast::Arg::Named(named) => collect_unknown_code_idents(named.expr(), names),
            typst_ast::Arg::Spread(spread) => collect_unknown_code_idents(spread.expr(), names),
        }
    }
}

fn collect_math_source(
    source: &str,
    offset: usize,
    names: &mut IndexSet<String>,
) -> Result<(), LabelError> {
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
    synthesize_ranges(&mut root, source.len())?;
    reject_syntax_errors_with_offset(&root, offset)?;
    let math = root
        .cast::<typst_ast::Math>()
        .ok_or_else(|| LabelError::Engine {
            start: offset,
            end: offset + source.len(),
            message: "Typst parser did not return a math root".to_string(),
        })?;
    collect_math(math, names);
    Ok(())
}

fn collect_math(math: typst_ast::Math<'_>, names: &mut IndexSet<String>) {
    for expr in math.exprs() {
        collect_math_expr(expr, names);
    }
}

fn collect_math_expr(expr: typst_ast::Expr<'_>, names: &mut IndexSet<String>) {
    match expr {
        typst_ast::Expr::Math(math) => collect_math(math, names),
        typst_ast::Expr::Ident(ident) => {
            names.insert(ident.as_str().to_string());
        }
        typst_ast::Expr::MathDelimited(delimited) => collect_math(delimited.body(), names),
        typst_ast::Expr::MathAttach(attach) => {
            collect_math_expr(attach.base(), names);
            if let Some(top) = attach.top() {
                collect_math_expr(top, names);
            }
            if let Some(bottom) = attach.bottom() {
                collect_math_expr(bottom, names);
            }
        }
        typst_ast::Expr::MathFrac(frac) => {
            collect_math_expr(frac.num(), names);
            collect_math_expr(frac.denom(), names);
        }
        typst_ast::Expr::MathRoot(root) => {
            collect_math_expr(root.radicand(), names);
        }
        typst_ast::Expr::MathCall(call) => {
            for item in call.args().arg_items() {
                match item.arg {
                    typst_ast::Arg::Pos(expr) => collect_math_expr(expr, names),
                    typst_ast::Arg::Named(named) => collect_math_expr(named.expr(), names),
                    typst_ast::Arg::Spread(spread) => collect_math_expr(spread.expr(), names),
                }
            }
        }
        typst_ast::Expr::Parenthesized(parenthesized) => {
            collect_math_expr(parenthesized.expr(), names);
        }
        typst_ast::Expr::Array(array) => {
            for item in array.items() {
                match item {
                    typst_ast::ArrayItem::Pos(expr) => collect_math_expr(expr, names),
                    typst_ast::ArrayItem::Spread(spread) => collect_math_expr(spread.expr(), names),
                }
            }
        }
        typst_ast::Expr::Dict(dict) => {
            for item in dict.items() {
                match item {
                    typst_ast::DictItem::Named(named) => collect_math_expr(named.expr(), names),
                    typst_ast::DictItem::Keyed(keyed) => {
                        collect_math_expr(keyed.key(), names);
                        collect_math_expr(keyed.expr(), names);
                    }
                    typst_ast::DictItem::Spread(spread) => collect_math_expr(spread.expr(), names),
                }
            }
        }
        typst_ast::Expr::Unary(unary) => collect_math_expr(unary.expr(), names),
        typst_ast::Expr::Binary(binary) => {
            collect_math_expr(binary.lhs(), names);
            collect_math_expr(binary.rhs(), names);
        }
        typst_ast::Expr::FuncCall(call) => {
            for arg in call.args().items() {
                match arg {
                    typst_ast::Arg::Pos(expr) => collect_math_expr(expr, names),
                    typst_ast::Arg::Named(named) => collect_math_expr(named.expr(), names),
                    typst_ast::Arg::Spread(spread) => collect_math_expr(spread.expr(), names),
                }
            }
        }
        _ => {}
    }
}

fn collect_unknown_code_idents(expr: typst_ast::Expr<'_>, names: &mut IndexSet<String>) {
    match expr {
        typst_ast::Expr::Ident(ident) => {
            let name = ident.as_str();
            if !is_retained_markup_name(name) {
                names.insert(name.to_string());
            }
        }
        typst_ast::Expr::Parenthesized(parenthesized) => {
            collect_unknown_code_idents(parenthesized.expr(), names);
        }
        typst_ast::Expr::Array(array) => {
            for item in array.items() {
                match item {
                    typst_ast::ArrayItem::Pos(expr) => collect_unknown_code_idents(expr, names),
                    typst_ast::ArrayItem::Spread(spread) => {
                        collect_unknown_code_idents(spread.expr(), names);
                    }
                }
            }
        }
        typst_ast::Expr::Dict(dict) => {
            for item in dict.items() {
                match item {
                    typst_ast::DictItem::Named(named) => {
                        collect_unknown_code_idents(named.expr(), names);
                    }
                    typst_ast::DictItem::Keyed(keyed) => {
                        collect_unknown_code_idents(keyed.key(), names);
                        collect_unknown_code_idents(keyed.expr(), names);
                    }
                    typst_ast::DictItem::Spread(spread) => {
                        collect_unknown_code_idents(spread.expr(), names);
                    }
                }
            }
        }
        typst_ast::Expr::Unary(unary) => collect_unknown_code_idents(unary.expr(), names),
        typst_ast::Expr::Binary(binary) => {
            collect_unknown_code_idents(binary.lhs(), names);
            collect_unknown_code_idents(binary.rhs(), names);
        }
        typst_ast::Expr::FuncCall(call) => {
            for arg in call.args().items() {
                match arg {
                    typst_ast::Arg::Pos(expr) => collect_unknown_code_idents(expr, names),
                    typst_ast::Arg::Named(named) => {
                        collect_unknown_code_idents(named.expr(), names);
                    }
                    typst_ast::Arg::Spread(spread) => {
                        collect_unknown_code_idents(spread.expr(), names);
                    }
                }
            }
        }
        _ => {}
    }
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

fn expand_hash_range(source: &str, range: std::ops::Range<usize>) -> std::ops::Range<usize> {
    if range.start > 0 && source.as_bytes().get(range.start - 1) == Some(&b'#') {
        range.start - 1..range.end
    } else {
        range
    }
}

fn synthesize_ranges(root: &mut SyntaxNode, source_len: usize) -> Result<(), LabelError> {
    let mapper =
        RangeMapper::new(std::iter::once(0..source_len)).map_err(|message| LabelError::Engine {
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

fn reject_syntax_errors(root: &SyntaxNode) -> Result<(), LabelError> {
    reject_syntax_errors_with_offset(root, 0)
}

fn reject_syntax_errors_with_offset(root: &SyntaxNode, offset: usize) -> Result<(), LabelError> {
    if !root.diagnosis().errors {
        return Ok(());
    }
    let (errors, _) = root.errors_and_warnings();
    let message = errors
        .first()
        .map(|error| error.message.to_string())
        .unwrap_or_else(|| "invalid Typst syntax".to_string());
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

fn scratch_file_id() -> crate::typst_syntax::FileId {
    RootedPath::new(
        VirtualRoot::Project,
        VirtualPath::new("avenger-typst-label-param-scan.typ")
            .expect("static virtual path is valid"),
    )
    .intern()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_markup_math_and_option_params_in_source_order() {
        let names = referenced_params(
            "#upper[#series] #underline(stroke: series_color, offset: label_offset)[care] \
             $y = #slope x + #intercept$ #series",
        )
        .unwrap();
        assert_eq!(
            names,
            vec![
                "series",
                "series_color",
                "label_offset",
                "slope",
                "intercept",
            ]
        );
    }

    #[test]
    fn ignores_retained_literals_and_bare_math_identifiers() {
        let names = referenced_params(
            "#underline(stroke: 1.5pt + red, evade: true)[care] $alpha + frac(1, sqrt(x))$",
        )
        .unwrap();
        assert!(names.is_empty());
    }

    #[test]
    fn extracts_numfmt_params() {
        let names = referenced_params(
            "Peak #numfmt(value, \".3f\", precision: precision, currency: currency_code)",
        )
        .unwrap();
        assert_eq!(names, vec!["value", "precision", "currency_code"]);
    }

    #[test]
    fn extracts_datefmt_params() {
        let names =
            referenced_params("Report #datefmt(report_date, date_format, date_style: style_name)")
                .unwrap();
        assert_eq!(names, vec!["report_date", "date_format", "style_name"]);
    }
}
