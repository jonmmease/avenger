//! Ported from crates/typst-eval/src/call.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: calls to native functions and element functions. Labels define no closures, and
//! types and elements have no methods, so method calls and closure evaluation are gone.

use ecow::{EcoVec, eco_format};

use crate::typst_library::diag::{At, HintedString, SourceResult, bail, error};
use crate::typst_library::foundations::{
    Arg, Args, Content, Func, NativeElement, SequenceElem, SymbolElem, Value,
};
use crate::typst_library::math::LrElem;
use typst_syntax::ast::{self, AstNode};
use typst_syntax::{Span, Spanned, SyntaxNode};

use super::{Eval, Vm};

impl Eval for ast::FuncCall<'_> {
    type Output = Value;

    // avenger: no call depth to check, since a label's calls nest no deeper than its syntax.
    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        let span = self.span();
        let callee = self.callee();

        // Try to evaluate as a call to an associated function or field.
        if let ast::Expr::FieldAccess(access) = callee {
            let target = access.target().eval(vm)?;
            let field = access.field();
            match eval_field_callee(
                vm,
                access.to_untyped(),
                field.as_str(),
                field.span(),
                target,
                false,
            )? {
                FieldCallee::Func(func) => {
                    let args = self.args().eval(vm)?.spanned(span);
                    call_func(vm, func, args, span)
                }
                FieldCallee::NonFunc(_, err) => Err(err).at(callee.span()),
            }
        } else {
            // Function call order: we evaluate the callee before the arguments.
            let func = callee.eval(vm)?.cast::<Func>().at(callee.span())?;
            let args = self.args().eval(vm)?.spanned(span);
            call_func(vm, func, args, span)
        }
    }
}

impl Eval for ast::MathCall<'_> {
    type Output = Value;

    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        eval_math_call(vm, self)
    }
}

/// Evaluate a function call in math.
fn eval_math_call(vm: &mut Vm, math_call: ast::MathCall) -> SourceResult<Value> {
    let span = math_call.span();
    let callee = math_call.callee();

    let math_call_result = match callee {
        ast::MathAccess::MathIdent(ident) => {
            let callee_value = ident.eval(vm)?;
            match callee_value.clone().cast::<Func>() {
                Ok(func) => FieldCallee::Func(func),
                Err(err) => FieldCallee::NonFunc(callee_value, err),
            }
        }
        ast::MathAccess::MathFieldAccess(access) => {
            let target = access.target().eval(vm)?;
            let field = access.field();
            eval_field_callee(
                vm,
                access.to_untyped(),
                field.as_str(),
                field.span(),
                target,
                true,
            )?
        }
    };

    let args = math_call.args();
    match math_call_result {
        FieldCallee::Func(func) => {
            let args = args.eval(vm)?.spanned(span);
            call_func(vm, func, args, span)
        }
        FieldCallee::NonFunc(callee_value, _) => {
            let parens = unparse_math_args(vm, args, callee)?;
            let callee_content = callee_value.display().at(callee.span())?;
            Ok(Value::Content(callee_content.spanned(callee.span()) + parens))
        }
    }
}

/// Call a function.
// avenger: no call traces, and no stack growth, since calls nest only as deep as the syntax.
fn call_func(vm: &mut Vm, func: Func, args: Args, span: Span) -> SourceResult<Value> {
    let func = func.spanned(span);
    func.call(&mut vm.engine, args)
}

/// The kind of callee in a field-access function call.
// avenger: no methods, which labels' types and elements don't have.
enum FieldCallee {
    /// A plain function to call.
    Func(Func),
    /// The field access doesn't actually produce a function. This will error in
    /// code, but not in math.
    NonFunc(Value, HintedString),
}

/// Evaluate a field-access callee, prioritizing associated functions of the
/// value's type, "methods", over fields on the specific value.
///
/// Calls to fields of a value are only allowed for functions (`assert.eq`),
/// types (`str.to-unicode`, `table.cell`), modules (`pdf.attach`), and symbols
/// (`arrow.l`).
///
/// In particular, calls to a field function are not allowed for dictionaries
/// because it would be ambiguous. If we did allow it, we would either have to
/// prioritize methods or field functions, but both choices are bad:
/// - Prioritizing methods would make all new method additions breaking changes.
/// - Prioritizing field functions would break methods for certain dictionaries,
///   e.g. `(at: x => ...).at(key)`.
// avenger: there are no methods to prioritize.
fn eval_field_callee(
    vm: &mut Vm,
    access: &SyntaxNode,
    field: &str,
    field_span: Span,
    target: Value,
    in_math: bool,
) -> SourceResult<FieldCallee> {
    let sink = (&mut vm.engine, field_span);

    let callee_value =
        if matches!(target, Value::Symbol(_) | Value::Func(_) | Value::Module(_)) {
            // Only these types are allowed to use field call syntax on non-methods.
            target.field(field, sink).at(field_span)?
        } else {
            // Otherwise we cannot call this field and produce an error.
            match target.field(field, sink) {
                // The field does exist.
                Ok(callee_value) => {
                    let is_dict = matches!(target, Value::Dict(_));
                    let mut err = if is_dict {
                        // Dictionaries get a specific error & hint because they're
                        // the easiest to attempt this with, and users need to be
                        // told directly why it's not allowed.
                        error!(
                            access.span(),
                            "cannot directly call dictionary keys as functions";
                        )
                    } else {
                        let (kind, name) = element_or_type_with_name(&target);
                        error!(
                            access.span(),
                            "`{field}` is not a valid method for {kind} `{name}`";
                        )
                    };
                    if callee_value.clone().cast::<Func>().is_ok() {
                        err.hint(eco_format!(
                            "to call the stored function, {}wrap the field access \
                            in parentheses: `{}({})(..)`",
                            if in_math { "use code mode and " } else { "" },
                            if in_math { "#" } else { "" },
                            access.full_text(),
                        ));
                    } else if in_math {
                        err.hint("try adding a space before the parentheses");
                    } else {
                        err.hint(eco_format!(
                        "to access the `{field}` {}, remove the function arguments: `{}`",
                        if is_dict { "key" } else { "field" },
                        access.full_text(),
                    ));
                    }
                    if is_dict {
                        err.hint(
                            "dictionary keys cannot be used with method syntax as keys \
                            could conflict with built-in method names",
                        );
                    }

                    bail!(err)
                }
                // The field does not exist. We don't try as hard on the error here
                // to avoid assuming the user's intent.
                Err(_) => {
                    let (kind, name) = element_or_type_with_name(&target);
                    bail!(access.span(), "{kind} {name} has no method `{field}`")
                }
            }
        };

    match callee_value.clone().cast::<Func>() {
        Ok(func) => Ok(FieldCallee::Func(func)),
        Err(err) => Ok(FieldCallee::NonFunc(callee_value, err)),
    }
}

/// If the value is content, the string "element" and the name of its element
/// function, or the string "type" and the name of the value's type.
fn element_or_type_with_name(value: &Value) -> (&'static str, &'static str) {
    if let Value::Content(content) = value {
        ("element", content.elem().name())
    } else {
        ("type", value.ty().long_name())
    }
}

impl Eval for ast::Args<'_> {
    type Output = Args;

    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        let mut items = EcoVec::with_capacity(self.items().count());

        for arg in self.items() {
            let span = arg.span();
            match arg {
                ast::Arg::Pos(expr) => {
                    items.push(Arg {
                        span,
                        name: None,
                        value: Spanned::new(expr.eval(vm)?, expr.span()),
                    });
                }
                ast::Arg::Named(named) => {
                    let expr = named.expr();
                    items.push(Arg {
                        span,
                        name: Some(named.name().get().clone().into()),
                        value: Spanned::new(expr.eval(vm)?, expr.span()),
                    });
                }
                ast::Arg::Spread(spread) => match spread.expr().eval(vm)? {
                    Value::None => {}
                    Value::Array(array) => {
                        items.extend(array.into_iter().map(|value| Arg {
                            span,
                            name: None,
                            value: Spanned::new(value, span),
                        }));
                    }
                    Value::Dict(dict) => {
                        items.extend(dict.into_iter().map(|(key, value)| Arg {
                            span,
                            name: Some(key),
                            value: Spanned::new(value, span),
                        }));
                    }
                    v => bail!(spread.span(), "cannot spread {}", v.ty()),
                },
            }
        }

        // We do *not* use the `self.span()` here because we want the callsite
        // span to be one level higher (the whole function call).
        Ok(Args { span: Span::detached(), items })
    }
}

impl Eval for ast::MathArgs<'_> {
    type Output = Args;

    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        // Math args need to fully separate named/pos to handle two-dimensional
        // args correctly, for example: `mat(a, delim:"[", b; c, d)`.
        let mut named = EcoVec::new();
        let mut pos = Vec::new();
        let mut two_dim_start: Option<usize> = None;

        /// Optimize two-dimensional args by using `pos` as the sole container
        /// while iterating and only group into an array when we encounter a
        /// semicolon.
        fn drain_into_array(pos: &mut Vec<Arg>, start: usize, span: Span) {
            let array = pos.drain(start..).map(|arg| arg.value.v).collect();
            pos.push(Arg {
                span,
                name: None,
                value: Spanned::new(Value::Array(array), span),
            });
        }

        for ast::MathArg { arg, ends_in_semicolon } in self.arg_items() {
            let span = arg.span();
            match arg {
                ast::Arg::Pos(expr) => {
                    pos.push(Arg {
                        span,
                        name: None,
                        value: Spanned::new(expr.eval(vm)?, expr.span()),
                    });
                }
                ast::Arg::Named(named_arg) => {
                    let expr = named_arg.expr();
                    named.push(Arg {
                        span,
                        name: Some(named_arg.name().get().clone().into()),
                        value: Spanned::new(expr.eval(vm)?, expr.span()),
                    });
                }
                ast::Arg::Spread(spread) => match spread.expr().eval(vm)? {
                    Value::None => {}
                    Value::Array(array) => {
                        pos.extend(array.into_iter().map(|value| Arg {
                            span,
                            name: None,
                            value: Spanned::new(value, span),
                        }));
                    }
                    Value::Dict(dict) => {
                        named.extend(dict.into_iter().map(|(key, value)| Arg {
                            span,
                            name: Some(key),
                            value: Spanned::new(value, span),
                        }));
                    }
                    v => bail!(spread.span(), "cannot spread {}", v.ty()),
                },
            }
            if ends_in_semicolon {
                let start = two_dim_start.unwrap_or(0);
                // There's not really a better span to use :/
                drain_into_array(&mut pos, start, self.span());
                two_dim_start = Some(pos.len());
            }
        }

        if let Some(start) = two_dim_start
            && start != pos.len()
        {
            drain_into_array(&mut pos, start, self.span());
        }

        named.extend(pos);
        Ok(Args { span: Span::detached(), items: named })
    }
}

/// For non-functions in math, we evaluate the arguments and punctuation as
/// content and wrap in an [`LrElem`].
fn unparse_math_args(
    vm: &mut Vm,
    args: ast::MathArgs,
    callee: ast::MathAccess,
) -> SourceResult<Content> {
    let mut body = Vec::new();
    let mut errors = EcoVec::new();
    for item in args.content_items() {
        match item {
            ast::MathArgItem::Space(space) => {
                body.push(space.eval(vm)?.spanned(space.span()));
            }
            ast::MathArgItem::Comma(c, node)
            | ast::MathArgItem::Semicolon(c, node)
            | ast::MathArgItem::LeftParen(c, node)
            | ast::MathArgItem::RightParen(c, node) => {
                body.push(SymbolElem::packed(c).spanned(node.span()));
            }
            ast::MathArgItem::Arg(ast::Arg::Pos(expr)) => {
                // We use `Value::display` to convert argument expressions into
                // content instead of `Content::from_value`. This makes it so we
                // don't error on `$sin(#1)$` because we don't error on `$#1$`.
                // avenger: displaying can fail in a label (D3).
                let value = expr.eval(vm)?.display().at(expr.span())?;
                body.push(value.spanned(expr.span()));
            }
            ast::MathArgItem::Arg(ast::Arg::Named(named)) => {
                let name = callee.to_untyped().full_text();
                let fixed = named.to_untyped().full_text().replacen(":", "\\:", 1);
                errors.push(error!(
                    named.span(), "named-argument syntax can only be used with functions";
                    hint[callee.span()]: "`{name}` is not a function";
                    hint: "to render the colon as text, escape it: `{fixed}`";
                ));
            }
            ast::MathArgItem::Arg(ast::Arg::Spread(spread)) => {
                let name = callee.to_untyped().full_text();
                let fixed = spread.to_untyped().full_text().replacen("..", ".. ", 1);
                errors.push(error!(
                    spread.span(), "spread-argument syntax can only be used with functions";
                    hint[callee.span()]: "`{name}` is not a function";
                    hint: "to render the dots as text, add a space: `{fixed}`";
                ));
            }
        }
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    Ok(LrElem::new(SequenceElem::new(body).pack())
        .pack()
        .spanned(args.span()))
}
