//! Ported from crates/typst-eval/src/markup.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: a label is one paragraph of inline markup, so paragraph breaks, multi-line raw text,
//! headings, lists, links, labels and references are errors, as are set and show rules.

use crate::typst_library::diag::{At, SourceResult, bail};
use crate::typst_library::foundations::{Content, NativeElement, Symbol, Value};
use crate::typst_library::model::{EmphElem, StrongElem};
use crate::typst_library::text::{
    LinebreakElem, RawContent, RawElem, SmartQuoteElem, SpaceElem, TextElem,
};
use typst_syntax::ast::{self, AstNode};

use super::{Eval, Vm};

impl Eval for ast::Markup<'_> {
    type Output = Content;

    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        eval_markup(vm, &mut self.exprs())
    }
}

/// Evaluate a stream of markup.
// avenger: no set or show rules, flow or labels.
fn eval_markup<'a>(
    vm: &mut Vm,
    exprs: &mut impl Iterator<Item = ast::Expr<'a>>,
) -> SourceResult<Content> {
    let mut seq = Vec::with_capacity(exprs.size_hint().1.unwrap_or_default());

    for expr in exprs {
        match expr {
            ast::Expr::SetRule(set) => {
                bail!(set.span(), "set rules are not supported in labels")
            }
            ast::Expr::ShowRule(show) => {
                bail!(show.span(), "show rules are not supported in labels")
            }
            expr => {
                let value = expr.eval(vm)?;
                seq.push(value.display().at(expr.span())?.spanned(expr.span()))
            }
        }
    }

    Ok(Content::sequence(seq))
}

impl Eval for ast::Text<'_> {
    type Output = Content;

    fn eval(self, _: &mut Vm) -> SourceResult<Self::Output> {
        Ok(TextElem::packed(self.get().clone()))
    }
}

impl Eval for ast::Space<'_> {
    type Output = Content;

    fn eval(self, _: &mut Vm) -> SourceResult<Self::Output> {
        Ok(SpaceElem::shared().clone())
    }
}

impl Eval for ast::Linebreak<'_> {
    type Output = Content;

    fn eval(self, _: &mut Vm) -> SourceResult<Self::Output> {
        Ok(LinebreakElem::shared().clone())
    }
}

// avenger: no `Parbreak`, which `Expr::eval` rejects.

impl Eval for ast::Escape<'_> {
    type Output = Value;

    // avenger: an escaped line break is data, so it becomes a space (D5).
    fn eval(self, _: &mut Vm) -> SourceResult<Self::Output> {
        Ok(Value::Symbol(Symbol::runtime_char(one_line_char(self.get()))))
    }
}

impl Eval for ast::Shorthand<'_> {
    type Output = Value;

    fn eval(self, _: &mut Vm) -> SourceResult<Self::Output> {
        Ok(Value::Symbol(Symbol::runtime_char(self.get())))
    }
}

impl Eval for ast::SmartQuote<'_> {
    type Output = Content;

    fn eval(self, _: &mut Vm) -> SourceResult<Self::Output> {
        Ok(SmartQuoteElem::new().with_double(self.double()).pack())
    }
}

impl Eval for ast::Strong<'_> {
    type Output = Content;

    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        let body = self.body().eval(vm)?;
        Ok(StrongElem::new(body).pack())
    }
}

impl Eval for ast::Emph<'_> {
    type Output = Content;

    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        let body = self.body().eval(vm)?;
        Ok(EmphElem::new(body).pack())
    }
}

impl Eval for ast::Raw<'_> {
    type Output = Content;

    // avenger: raw text is one line without syntax highlighting.
    fn eval(self, _: &mut Vm) -> SourceResult<Self::Output> {
        if let Some(lang) = self.lang() {
            bail!(
                lang.span(), "syntax highlighting is not supported in labels";
                hint: "remove the language tag";
            );
        }
        let lines: Vec<_> =
            self.lines().map(|line| (line.get().clone(), line.span())).collect();
        if lines.len() > 1 {
            bail!(self.span(), "raw text in a label must be a single line");
        }
        Ok(RawElem::new(RawContent::Lines(lines.into()))
            .with_block(self.block())
            .pack())
    }
}

// avenger: no `Link`, `Label`, `Ref`, `Heading`, `ListItem`, `EnumItem` or `TermItem`, which
// `Expr::eval` rejects.

/// A mandatory line break in data as a space (D5).
// avenger: the data rule for escapes.
fn one_line_char(c: char) -> char {
    match c {
        '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}' => ' ',
        c => c,
    }
}
