use crate::api::TypstEngineConfig;
use crate::error::{MathTypesetError, TypstInitError};
use crate::types::{MathFragmentOptions, MathRunArtifact};

use typst_library::foundations::{Content, NativeElement, SymbolElem};
use typst_library::math::{
    AlignPointElem, AttachElem, FracElem, LrElem, MatElem, PrimesElem, RootElem,
};
use typst_library::text::TextElem;
use typst_syntax::ast::{self, Arg, MathTextKind};

#[derive(Debug, Clone)]
pub(crate) struct TypstMathEngine;

impl TypstMathEngine {
    pub(crate) fn new(_config: &TypstEngineConfig) -> Result<Self, TypstInitError> {
        Ok(Self)
    }

    pub(crate) fn typeset_fragment(
        &self,
        source: &str,
        _options: &MathFragmentOptions,
    ) -> Result<MathRunArtifact, MathTypesetError> {
        let _content = lower_math_source(source)?;
        Err(MathTypesetError::UnsupportedOutput(
            "vendor-typst layout is not wired yet",
        ))
    }
}

fn lower_math_source(source: &str) -> Result<Content, MathTypesetError> {
    let root = typst_syntax::parse_math(source);
    let math = root
        .cast::<ast::Math>()
        .ok_or_else(|| unsupported(0, "expected Typst math root"))?;
    lower_math(math)
}

fn lower_math(math: ast::Math<'_>) -> Result<Content, MathTypesetError> {
    math.exprs()
        .map(lower_expr)
        .collect::<Result<Vec<_>, _>>()
        .map(Content::sequence)
}

fn lower_expr(expr: ast::Expr<'_>) -> Result<Content, MathTypesetError> {
    match expr {
        ast::Expr::Space(_) => Ok(SymbolElem::packed(" ")),
        ast::Expr::Math(math) => lower_math(math),
        ast::Expr::MathText(text) => lower_math_text(text),
        ast::Expr::MathIdent(ident) => Ok(symbol_or_identifier(ident.as_str())),
        ast::Expr::MathShorthand(shorthand) => Ok(SymbolElem::packed(shorthand.get().to_string())),
        ast::Expr::MathAlignPoint(_) => Ok(AlignPointElem::shared().clone()),
        ast::Expr::MathDelimited(delimited) => lower_math_delimited(delimited),
        ast::Expr::MathAttach(attach) => lower_math_attach(attach),
        ast::Expr::MathPrimes(primes) => Ok(PrimesElem::new(primes.count()).pack()),
        ast::Expr::MathFrac(frac) => lower_math_frac(frac),
        ast::Expr::MathRoot(root) => lower_math_root(root),
        ast::Expr::MathCall(call) => lower_math_call(call),
        _ => Err(unsupported(
            0,
            "unsupported Typst math expression in strict Avenger subset",
        )),
    }
}

fn lower_math_text(text: ast::MathText<'_>) -> Result<Content, MathTypesetError> {
    match text.get() {
        MathTextKind::Grapheme(text) => Ok(SymbolElem::packed(text.clone())),
        MathTextKind::Number(text) => Ok(TextElem::packed(text.clone())),
    }
}

fn lower_math_delimited(delimited: ast::MathDelimited<'_>) -> Result<Content, MathTypesetError> {
    let open = lower_expr(delimited.open())?;
    let body = lower_math(delimited.body())?;
    let close = lower_expr(delimited.close())?;
    Ok(LrElem::new(open + body + close).pack())
}

fn lower_math_attach(attach: ast::MathAttach<'_>) -> Result<Content, MathTypesetError> {
    let mut elem = AttachElem::new(lower_expr(attach.base())?);

    if let Some(top) = attach.top() {
        elem.t.set(Some(lower_expr(top)?));
    }
    if let Some(primes) = attach.primes() {
        elem.tr.set(Some(PrimesElem::new(primes.count()).pack()));
    }
    if let Some(bottom) = attach.bottom() {
        elem.b.set(Some(lower_expr(bottom)?));
    }

    Ok(elem.pack())
}

fn lower_math_frac(frac: ast::MathFrac<'_>) -> Result<Content, MathTypesetError> {
    let num_expr = frac.num();
    let denom_expr = frac.denom();
    let num_deparenthesized =
        matches!(num_expr, ast::Expr::Math(math) if math.was_deparenthesized());
    let denom_deparenthesized =
        matches!(denom_expr, ast::Expr::Math(math) if math.was_deparenthesized());

    Ok(
        FracElem::new(lower_expr(num_expr)?, lower_expr(denom_expr)?)
            .with_num_deparenthesized(num_deparenthesized)
            .with_denom_deparenthesized(denom_deparenthesized)
            .pack(),
    )
}

fn lower_math_root(root: ast::MathRoot<'_>) -> Result<Content, MathTypesetError> {
    let index = root
        .index()
        .map(|index| TextElem::packed(index.to_string()));
    Ok(RootElem::new(lower_expr(root.radicand())?)
        .with_index(index)
        .pack())
}

fn lower_math_call(call: ast::MathCall<'_>) -> Result<Content, MathTypesetError> {
    let ast::MathAccess::MathIdent(callee) = call.callee() else {
        return Err(unsupported(
            0,
            "field-access math calls are not supported in strict Avenger subset",
        ));
    };

    match callee.as_str() {
        "frac" => lower_two_arg_call(call, |num, denom| FracElem::new(num, denom).pack()),
        "sqrt" => lower_one_arg_call(call, |radicand| RootElem::new(radicand).pack()),
        "root" => lower_two_arg_call(call, |index, radicand| {
            RootElem::new(radicand).with_index(Some(index)).pack()
        }),
        "mat" => lower_matrix_call(call),
        _ => Err(unsupported(
            0,
            "unsupported math function in strict Avenger subset",
        )),
    }
}

fn lower_one_arg_call(
    call: ast::MathCall<'_>,
    build: impl FnOnce(Content) -> Content,
) -> Result<Content, MathTypesetError> {
    let args = positional_args(call)?;
    let [arg]: [Content; 1] = args
        .try_into()
        .map_err(|_| unsupported(0, "math function expects exactly one positional argument"))?;
    Ok(build(arg))
}

fn lower_two_arg_call(
    call: ast::MathCall<'_>,
    build: impl FnOnce(Content, Content) -> Content,
) -> Result<Content, MathTypesetError> {
    let args = positional_args(call)?;
    let [first, second]: [Content; 2] = args
        .try_into()
        .map_err(|_| unsupported(0, "math function expects exactly two positional arguments"))?;
    Ok(build(first, second))
}

fn positional_args(call: ast::MathCall<'_>) -> Result<Vec<Content>, MathTypesetError> {
    call.args()
        .arg_items()
        .map(|item| lower_positional_arg(item.arg))
        .collect()
}

fn lower_positional_arg(arg: Arg<'_>) -> Result<Content, MathTypesetError> {
    match arg {
        Arg::Pos(expr) => lower_expr(expr),
        Arg::Named(_) => Err(unsupported(
            0,
            "named math arguments are not supported in strict Avenger subset",
        )),
        Arg::Spread(_) => Err(unsupported(
            0,
            "spread math arguments are not supported in strict Avenger subset",
        )),
    }
}

fn lower_matrix_call(call: ast::MathCall<'_>) -> Result<Content, MathTypesetError> {
    let mut rows = vec![Vec::new()];

    for item in call.args().arg_items() {
        rows.last_mut()
            .expect("matrix row is initialized")
            .push(lower_positional_arg(item.arg)?);
        if item.ends_in_semicolon {
            rows.push(Vec::new());
        }
    }

    if matches!(rows.last(), Some(row) if row.is_empty()) && rows.len() > 1 {
        rows.pop();
    }

    Ok(MatElem::new(rows).pack())
}

fn symbol_or_identifier(name: &str) -> Content {
    SymbolElem::packed(match named_math_symbol(name) {
        Some(symbol) => symbol,
        None => name,
    })
}

fn named_math_symbol(name: &str) -> Option<&'static str> {
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
        "sum" => Some("∑"),
        "prod" => Some("∏"),
        "integral" => Some("∫"),
        "oo" | "infinity" => Some("∞"),
        "partial" => Some("∂"),
        "nabla" => Some("∇"),
        _ => None,
    }
}

fn unsupported(position: usize, message: &'static str) -> MathTypesetError {
    MathTypesetError::UnsupportedSyntax { position, message }
}

#[cfg(test)]
mod tests {
    use super::*;
    use typst_library::math::{AttachElem, FracElem, MatElem, RootElem};

    #[test]
    fn lowers_basic_symbol_text() {
        let content = lower_math_source("x^2 + y^2").unwrap();

        assert!(content.plain_text().contains("x2 + y2"));
    }

    #[test]
    fn lowers_named_symbols() {
        let content = lower_math_source("alpha + pi + sum").unwrap();

        assert_eq!(content.plain_text().as_str(), "α + π + ∑");
    }

    #[test]
    fn lowers_fraction_syntax() {
        let content = lower_math_source("a / b").unwrap();

        assert!(content.is::<FracElem>());
    }

    #[test]
    fn lowers_root_call() {
        let content = lower_math_source("sqrt(x^2 + y^2)").unwrap();

        assert!(content.is::<RootElem>());
    }

    #[test]
    fn lowers_attachment_syntax() {
        let content = lower_math_source("sum_(i=1)^n x_i").unwrap();

        assert!(content.plain_text().contains("∑"));
        assert!(contains_sequence_child::<AttachElem>(&content));
    }

    #[test]
    fn lowers_matrix_call() {
        let content = lower_math_source("mat(1, 2; 3, 4)").unwrap();

        assert!(content.is::<MatElem>());
    }

    #[test]
    fn rejects_unsupported_call() {
        let err = lower_math_source("foo(x)").unwrap_err();

        assert_eq!(
            err,
            MathTypesetError::UnsupportedSyntax {
                position: 0,
                message: "unsupported math function in strict Avenger subset"
            }
        );
    }

    fn contains_sequence_child<T: NativeElement>(content: &Content) -> bool {
        let mut found = false;
        content.sequence_recursive_for_each(&mut |child| {
            found |= child.is::<T>();
        });
        found
    }
}
