//! Named arguments: the option pairs of the window functions of avenger-datafusion-transforms,
//! and the omitted parameters of the functions that declare parameter names.

use datafusion::sql::sqlparser::{
    ast::{
        Expr, Function, FunctionArg, FunctionArgExpr, FunctionArgOperator, FunctionArguments,
        Ident, ObjectNamePart, Spanned, Value,
    },
    tokenizer::Span,
};

/// The window functions of avenger-datafusion-transforms, which take their options as pairs of a
/// name and a value after their positional arguments.
const TRANSFORMS: [&str; 4] = ["bin", "timeunit", "lump", "stack"];

/// The options of `lump` whose values are SQL expressions over the rows of each category, which
/// `lump` takes as text.
const EXPRESSION_OPTIONS: [&str; 2] = ["weight", "keep"];

/// The name of the function of `function`, lowercase unless it is quoted, when it has one part.
pub(super) fn function_name(function: &Function) -> Option<String> {
    match function.name.0.as_slice() {
        [ObjectNamePart::Identifier(ident)] => Some(match ident.quote_style {
            Some(_) => ident.value.clone(),
            None => ident.value.to_lowercase(),
        }),
        _ => None,
    }
}

/// The name of an argument of a call, when it is named.
fn argument_name(argument: &FunctionArg) -> Option<&Ident> {
    match argument {
        FunctionArg::Named { name, .. } => Some(name),
        FunctionArg::ExprNamed {
            name: Expr::Identifier(name),
            ..
        } => Some(name),
        _ => None,
    }
}

/// Writes each named option of a call of `bin`, `timeunit`, `lump`, or `stack` as the option pair
/// `'name', value` after the positional arguments, in the order of the call, so `bin(x, maxbins =>
/// 30)` becomes `bin(x, 'maxbins', 30)`. The value of `weight` and `keep` of `lump` is the text of
/// its expression. Removed by DataFusion resolving named arguments with omitted parameters, when
/// the transforms declare their options as parameters.
pub(super) fn option_pairs(function: &mut Function) {
    let Some(name) = function_name(function) else {
        return;
    };
    if !TRANSFORMS.contains(&name.as_str()) {
        return;
    }
    let FunctionArguments::List(list) = &mut function.args else {
        return;
    };
    let (named, mut args): (Vec<FunctionArg>, Vec<FunctionArg>) = std::mem::take(&mut list.args)
        .into_iter()
        .partition(|argument| {
            argument_name(argument).is_some()
                && matches!(
                    argument,
                    FunctionArg::Named {
                        arg: FunctionArgExpr::Expr(_),
                        ..
                    } | FunctionArg::ExprNamed {
                        arg: FunctionArgExpr::Expr(_),
                        ..
                    }
                )
        });
    for argument in named {
        let (option, value) = match argument {
            FunctionArg::Named {
                name,
                arg: FunctionArgExpr::Expr(value),
                ..
            }
            | FunctionArg::ExprNamed {
                name: Expr::Identifier(name),
                arg: FunctionArgExpr::Expr(value),
                ..
            } => (name, value),
            _ => unreachable!("the partition keeps named expressions"),
        };
        let option_name = match option.quote_style {
            Some(_) => option.value.clone(),
            None => option.value.to_lowercase(),
        };
        let value = match name == "lump" && EXPRESSION_OPTIONS.contains(&option_name.as_str()) {
            true => text(&value),
            false => value,
        };
        args.push(positional(text_literal(&option_name, option.span)));
        args.push(positional(value));
    }
    list.args = args;
}

/// The text of `expr` as a string literal at its span.
fn text(expr: &Expr) -> Expr {
    text_literal(&expr.to_string(), expr.span())
}

/// The string literal `value` at `span`.
fn text_literal(value: &str, span: Span) -> Expr {
    Expr::Value(Value::SingleQuotedString(value.to_string()).with_span(span))
}

/// `expr` as an unnamed argument.
fn positional(expr: Expr) -> FunctionArg {
    FunctionArg::Unnamed(FunctionArgExpr::Expr(expr))
}

/// Whether the argument name `name` names the parameter `parameter`, as DataFusion matches them.
fn names(name: &Ident, parameter: &str) -> bool {
    match name.quote_style {
        Some(_) => name.value == parameter,
        None => name.value.eq_ignore_ascii_case(parameter),
    }
}

/// Adds `parameter => NULL` to `function` for each parameter before its last named argument that
/// it neither names nor takes by position, so DataFusion binds the call. A call of a function
/// without parameter names, or with a name that no parameter has, stays as it is, for DataFusion's
/// error. Removed by DataFusion resolving named arguments with omitted parameters.
pub(super) fn fill_omitted(
    function: &mut Function,
    parameters: &dyn Fn(&str) -> Option<Vec<String>>,
) {
    let FunctionArguments::List(list) = &mut function.args else {
        return;
    };
    if !list
        .args
        .iter()
        .any(|argument| argument_name(argument).is_some())
    {
        return;
    }
    let Some(name) = function_name(function) else {
        return;
    };
    let Some(parameters) = parameters(&name) else {
        return;
    };
    let FunctionArguments::List(list) = &mut function.args else {
        return;
    };
    let positional = (list.args.iter())
        .take_while(|argument| argument_name(argument).is_none())
        .count();
    let mut given = vec![false; parameters.len()];
    given
        .iter_mut()
        .take(positional)
        .for_each(|given| *given = true);
    for name in list.args.iter().filter_map(argument_name) {
        match parameters
            .iter()
            .position(|parameter| names(name, parameter))
        {
            Some(index) => given[index] = true,
            None => return,
        }
    }
    let Some(last) = given.iter().rposition(|given| *given) else {
        return;
    };
    let span = function.name.span();
    for (parameter, _) in (parameters.iter().zip(&given))
        .take(last)
        .filter(|(_, given)| !**given)
    {
        list.args.push(FunctionArg::Named {
            name: Ident::with_span(span, parameter.clone()),
            arg: FunctionArgExpr::Expr(Expr::Value(Value::Null.with_span(span))),
            operator: FunctionArgOperator::RightArrow,
        });
    }
}
