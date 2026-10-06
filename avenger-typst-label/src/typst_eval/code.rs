//! Ported from crates/typst-eval/src/code.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: the expressions a label rejects fail here with an error that names them.

use ecow::{EcoVec, eco_vec};

use crate::typst_library::diag::{At, SourceResult, bail};
use crate::typst_library::foundations::{Array, Content, Dict, Str, Value, ops};
use crate::typst_syntax::Span;
use crate::typst_syntax::ast::{self, AstNode};

use super::{Eval, Vm};

impl Eval for ast::Code<'_> {
    type Output = Value;

    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        eval_code(vm, &mut self.exprs())
    }
}

/// Evaluate a stream of expressions.
// avenger: no set or show rules, and no flow.
fn eval_code<'a>(
    vm: &mut Vm,
    exprs: &mut impl Iterator<Item = ast::Expr<'a>>,
) -> SourceResult<Value> {
    let mut output = Value::None;

    for expr in exprs {
        let span = expr.span();
        let value = expr.eval(vm)?;
        output = ops::join(output, value).at(span)?;
    }

    Ok(output)
}

impl Eval for ast::Expr<'_> {
    type Output = Value;

    // avenger: no tracing; unsupported expressions are errors.
    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        let span = self.span();
        let unsupported = |what| bail!(span, "{what} are not supported in labels");

        let value = match self {
            Self::Text(v) => v.eval(vm).map(Value::Content),
            Self::Space(v) => v.eval(vm).map(Value::Content),
            Self::Linebreak(_) => bail!(
                span, "line breaks are not supported in labels";
                hint: "a label is a single line";
            ),
            Self::Parbreak(_) => bail!(
                span, "paragraph breaks are not supported in labels";
                hint: "a label is a single line";
            ),
            Self::Escape(v) => v.eval(vm),
            Self::Shorthand(v) => v.eval(vm),
            Self::SmartQuote(v) => v.eval(vm).map(Value::Content),
            Self::Strong(v) => v.eval(vm).map(Value::Content),
            Self::Emph(v) => v.eval(vm).map(Value::Content),
            Self::Raw(v) => v.eval(vm).map(Value::Content),
            Self::Link(_) => unsupported("links"),
            Self::Label(_) => unsupported("`<label>` markers"),
            Self::Ref(_) => unsupported("references"),
            Self::Heading(_) => unsupported("headings"),
            Self::ListItem(_) | Self::EnumItem(_) | Self::TermItem(_) => {
                unsupported("lists")
            }
            Self::Equation(v) => v.eval(vm).map(Value::Content),
            Self::Math(v) => v.eval(vm).map(Value::Content),
            Self::MathText(v) => v.eval(vm).map(Value::Content),
            Self::MathIdent(v) => v.eval(vm),
            Self::MathFieldAccess(v) => v.eval(vm),
            Self::MathShorthand(v) => v.eval(vm),
            Self::MathAlignPoint(_) => unsupported("alignment points"),
            Self::MathCall(v) => v.eval(vm),
            Self::MathDelimited(v) => v.eval(vm).map(Value::Content),
            Self::MathAttach(v) => v.eval(vm).map(Value::Content),
            Self::MathPrimes(v) => v.eval(vm).map(Value::Content),
            Self::MathFrac(v) => v.eval(vm).map(Value::Content),
            Self::MathRoot(v) => v.eval(vm).map(Value::Content),
            Self::Ident(v) => v.eval(vm),
            Self::None(v) => v.eval(vm),
            Self::Auto(v) => v.eval(vm),
            Self::Bool(v) => v.eval(vm),
            Self::Int(v) => v.eval(vm),
            Self::Float(v) => v.eval(vm),
            Self::Numeric(v) => v.eval(vm),
            Self::Str(v) => v.eval(vm),
            Self::CodeBlock(v) => v.eval(vm),
            Self::ContentBlock(v) => v.eval(vm).map(Value::Content),
            Self::Array(v) => v.eval(vm).map(Value::Array),
            Self::Dict(v) => v.eval(vm).map(Value::Dict),
            Self::Parenthesized(v) => v.eval(vm),
            Self::FieldAccess(v) => v.eval(vm),
            Self::FuncCall(v) => v.eval(vm),
            Self::Closure(_) => unsupported("functions defined in a label"),
            Self::Unary(v) => v.eval(vm),
            Self::Binary(v) => v.eval(vm),
            Self::LetBinding(_) => unsupported("let bindings"),
            Self::DestructAssignment(_) => unsupported("assignments"),
            Self::SetRule(_) => unsupported("set rules"),
            Self::ShowRule(_) => unsupported("show rules"),
            Self::Contextual(_) => unsupported("context expressions"),
            Self::Conditional(_) => unsupported("conditionals"),
            Self::WhileLoop(_) | Self::ForLoop(_) => unsupported("loops"),
            Self::ModuleImport(_) => unsupported("imports"),
            Self::ModuleInclude(_) => unsupported("includes"),
            Self::LoopBreak(_) | Self::LoopContinue(_) => unsupported("loops"),
            Self::FuncReturn(_) => unsupported("returns"),
        }?
        .spanned(span);

        Ok(value)
    }
}

impl Eval for ast::Ident<'_> {
    type Output = Value;

    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        let span = self.span();
        Ok(vm
            .scopes
            .get(self.as_str())
            .at(span)?
            .read_checked((&mut vm.engine, span))
            .clone())
    }
}

impl Eval for ast::None<'_> {
    type Output = Value;

    fn eval(self, _: &mut Vm) -> SourceResult<Self::Output> {
        Ok(Value::None)
    }
}

impl Eval for ast::Auto<'_> {
    type Output = Value;

    fn eval(self, _: &mut Vm) -> SourceResult<Self::Output> {
        Ok(Value::Auto)
    }
}

impl Eval for ast::Bool<'_> {
    type Output = Value;

    fn eval(self, _: &mut Vm) -> SourceResult<Self::Output> {
        Ok(Value::Bool(self.get()))
    }
}

impl Eval for ast::Int<'_> {
    type Output = Value;

    fn eval(self, _: &mut Vm) -> SourceResult<Self::Output> {
        Ok(Value::Int(self.get()))
    }
}

impl Eval for ast::Float<'_> {
    type Output = Value;

    fn eval(self, _: &mut Vm) -> SourceResult<Self::Output> {
        Ok(Value::Float(self.get()))
    }
}

impl Eval for ast::Numeric<'_> {
    type Output = Value;

    fn eval(self, _: &mut Vm) -> SourceResult<Self::Output> {
        Ok(Value::numeric(self.get()))
    }
}

impl Eval for ast::Str<'_> {
    type Output = Value;

    fn eval(self, _: &mut Vm) -> SourceResult<Self::Output> {
        Ok(Value::Str(self.get().into()))
    }
}

impl Eval for ast::Array<'_> {
    type Output = Array;

    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        let mut items = self.items();

        let mut vec = EcoVec::with_capacity(items.size_hint().0);

        // We raise an error when one of the array items is the spread of a
        // dictionary. If _all_ of the array items are spreads of dictionaries,
        // the user probably wanted to write `(: ..dict_a, ..dict_b)` instead
        // to create a dictionary, not an array.
        let mut all_dict_spreads = true;

        while let Some(item) = items.next() {
            match item {
                ast::ArrayItem::Pos(expr) => {
                    all_dict_spreads = false;
                    vec.push(expr.eval(vm)?)
                }
                ast::ArrayItem::Spread(spread) => match spread.expr().eval(vm)? {
                    Value::None => {}
                    Value::Array(array) => {
                        all_dict_spreads = false;
                        vec.extend(array);
                    }
                    v @ Value::Dict(_)
                        if all_dict_spreads
                        // Lookahead to see whether remaining items are spreads
                        // of dicts
                        && items.all(|item| matches!(
                            item,
                            ast::ArrayItem::Spread(spread) if matches!(
                                spread.expr().eval(vm),
                                Ok(Value::Dict(_)),
                            ),
                        )) =>
                    {
                        let fixed = self.to_untyped().full_text().replacen("(", "(: ", 1);
                        bail!(
                            spread.span(), "cannot spread {} into array", v.ty();
                            hint: "add a colon to create a dictionary instead: `{fixed}`";
                        )
                    }
                    v => bail!(spread.span(), "cannot spread {} into array", v.ty()),
                },
            }
        }

        Ok(vec.into())
    }
}

impl Eval for ast::Dict<'_> {
    type Output = Dict;

    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        let mut map = indexmap::IndexMap::default();
        let mut invalid_keys = eco_vec![];

        for item in self.items() {
            match item {
                ast::DictItem::Named(named) => {
                    map.insert(named.name().get().clone().into(), named.expr().eval(vm)?);
                }
                ast::DictItem::Keyed(keyed) => {
                    let raw_key = keyed.key();
                    let key = raw_key.eval(vm)?;
                    let key =
                        key.cast::<Str>().at(raw_key.span()).unwrap_or_else(|errors| {
                            invalid_keys.extend(errors);
                            Str::default()
                        });
                    map.insert(key, keyed.expr().eval(vm)?);
                }
                ast::DictItem::Spread(spread) => match spread.expr().eval(vm)? {
                    Value::None => {}
                    Value::Dict(dict) => map.extend(dict),
                    v => bail!(spread.span(), "cannot spread {} into dictionary", v.ty()),
                },
            }
        }

        if !invalid_keys.is_empty() {
            return Err(invalid_keys);
        }

        Ok(map.into())
    }
}

impl Eval for ast::CodeBlock<'_> {
    type Output = Value;

    // avenger: no scopes to enter, since nothing binds.
    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        self.body().eval(vm)
    }
}

impl Eval for ast::ContentBlock<'_> {
    type Output = Content;

    // avenger: no scopes to enter, since nothing binds.
    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        self.body().eval(vm)
    }
}

impl Eval for ast::Parenthesized<'_> {
    type Output = Value;

    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        self.expr().eval(vm)
    }
}

impl Eval for ast::FieldAccess<'_> {
    type Output = Value;

    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        let target = self.target().eval(vm)?;
        let field = self.field();
        access_field(vm, target, field.as_str(), field.span())
    }
}

/// Access a field on a target value.
// avenger: no get rules, since labels have no context.
pub(crate) fn access_field(
    vm: &mut Vm,
    target: Value,
    field: &str,
    field_span: Span,
) -> SourceResult<Value> {
    target.field(field, (&mut vm.engine, field_span)).at(field_span)
}
