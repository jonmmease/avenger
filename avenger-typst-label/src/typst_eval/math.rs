//! Ported from crates/typst-eval/src/math.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: a label's equations are inline, so block equations and alignment points are
//! errors.

use ecow::eco_format;

use crate::typst_library::diag::{At, SourceResult, bail};
use crate::typst_library::foundations::{
    Content, NativeElement, Symbol, SymbolElem, Value,
};
use crate::typst_library::math::{
    AttachElem, EquationElem, FracElem, LrElem, PrimesElem, RootElem,
};
use crate::typst_library::text::TextElem;
use crate::typst_syntax::ast::{self, AstNode, MathTextKind};

use super::{Eval, Vm};

impl Eval for ast::Equation<'_> {
    type Output = Content;

    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        let block = self.block();
        // avenger: a label is one line.
        if block {
            bail!(
                self.span(), "block equations are not supported in labels";
                hint: "remove the spaces just inside the dollar signs to make the equation inline";
            );
        }
        let body = self.body().eval(vm)?;
        Ok(EquationElem::new(body).with_block(block).pack())
    }
}

impl Eval for ast::Math<'_> {
    type Output = Content;

    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        Ok(Content::sequence(
            self.exprs()
                .map(|expr| expr.eval_display(vm))
                .collect::<SourceResult<Vec<_>>>()?,
        ))
    }
}

impl Eval for ast::MathText<'_> {
    type Output = Content;

    fn eval(self, _: &mut Vm) -> SourceResult<Self::Output> {
        match self.get() {
            MathTextKind::Grapheme(text) => Ok(SymbolElem::packed(text.clone())),
            MathTextKind::Number(text) => Ok(TextElem::packed(text.clone())),
        }
    }
}

impl Eval for ast::MathIdent<'_> {
    type Output = Value;

    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        let span = self.span();
        Ok(vm
            .scopes
            .get_in_math(self.as_str())
            .at(span)?
            .read_checked((&mut vm.engine, span))
            .clone())
    }
}

impl Eval for ast::MathFieldAccess<'_> {
    type Output = Value;

    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        let target = self.target().eval(vm)?;
        let field = self.field();
        super::code::access_field(vm, target, field.as_str(), field.span())
    }
}

impl Eval for ast::MathAccess<'_> {
    type Output = Value;

    // avenger: no tracing.
    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        match self {
            ast::MathAccess::MathIdent(ident) => ident.eval(vm),
            ast::MathAccess::MathFieldAccess(access) => access.eval(vm),
        }
    }
}

impl Eval for ast::MathShorthand<'_> {
    type Output = Value;

    fn eval(self, _: &mut Vm) -> SourceResult<Self::Output> {
        Ok(Value::Symbol(Symbol::runtime_char(self.get())))
    }
}

// avenger: no `MathAlignPoint`, which `Expr::eval` rejects.

impl Eval for ast::MathDelimited<'_> {
    type Output = Content;

    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        let open = self.open().eval_display(vm)?;
        let body = self.body().eval(vm)?;
        let close = self.close().eval_display(vm)?;
        Ok(LrElem::new(open + body + close).pack())
    }
}

impl Eval for ast::MathAttach<'_> {
    type Output = Content;

    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        let base = self.base().eval_display(vm)?;
        let mut elem = AttachElem::new(base);

        if let Some(expr) = self.top() {
            elem.t.set(Some(expr.eval_display(vm)?));
        }

        // Always attach primes in scripts style (not limits style),
        // i.e. at the top-right corner.
        if let Some(primes) = self.primes() {
            elem.tr.set(Some(primes.eval(vm)?));
        }

        if let Some(expr) = self.bottom() {
            elem.b.set(Some(expr.eval_display(vm)?));
        }

        Ok(elem.pack())
    }
}

impl Eval for ast::MathPrimes<'_> {
    type Output = Content;

    fn eval(self, _: &mut Vm) -> SourceResult<Self::Output> {
        Ok(PrimesElem::new(self.count()).pack())
    }
}

impl Eval for ast::MathFrac<'_> {
    type Output = Content;

    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        let num_expr = self.num();
        let num = num_expr.eval_display(vm)?;
        let denom_expr = self.denom();
        let denom = denom_expr.eval_display(vm)?;

        let num_depar =
            matches!(num_expr, ast::Expr::Math(math) if math.was_deparenthesized());
        let denom_depar =
            matches!(denom_expr, ast::Expr::Math(math) if math.was_deparenthesized());

        Ok(FracElem::new(num, denom)
            .with_num_deparenthesized(num_depar)
            .with_denom_deparenthesized(denom_depar)
            .pack())
    }
}

impl Eval for ast::MathRoot<'_> {
    type Output = Content;

    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        // Use `TextElem` to match `MathTextKind::Number` above.
        let index = self.index().map(|i| TextElem::packed(eco_format!("{i}")));
        let radicand = self.radicand().eval_display(vm)?;
        Ok(RootElem::new(radicand).with_index(index).pack())
    }
}

trait ExprExt {
    fn eval_display(&self, vm: &mut Vm) -> SourceResult<Content>;
}

impl ExprExt for ast::Expr<'_> {
    // avenger: displaying can fail in a label (D3).
    fn eval_display(&self, vm: &mut Vm) -> SourceResult<Content> {
        Ok(self.eval(vm)?.display().at(self.span())?.spanned(self.span()))
    }
}
