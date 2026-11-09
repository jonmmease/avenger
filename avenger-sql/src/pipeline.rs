//! The query pipeline: parsing, the rewrites, and planning.

use std::ops::ControlFlow;

use datafusion::{
    common::{plan_err, DataFusionError, Result},
    dataframe::DataFrame,
    execution::SessionState,
    prelude::SessionContext,
    sql::{
        parser::Statement as DFStatement,
        sqlparser::{
            ast::{
                Expr, Function, FunctionArg, FunctionArgExpr, FunctionArgOperator,
                FunctionArguments, Ident, ObjectNamePart, Query, SelectFlavor, SelectItem, SetExpr,
                Spanned, Statement, Value, VisitMut, VisitorMut, WildcardAdditionalOptions,
            },
            dialect::GenericDialect,
            parser::{Parser, ParserError},
        },
    },
};

/// The SQL of the query `text` after the rewrites, for the functions of `state`, without planning
/// it.
pub fn expand(state: &SessionState, text: &str) -> Result<String> {
    Ok(parse(state, text)?.to_string())
}

/// Plans the query `text` in `ctx`, after the rewrites. `text` holds one statement.
pub async fn sql(ctx: &SessionContext, text: &str) -> Result<DataFrame> {
    let state = ctx.state();
    let statement = parse(&state, text)?;
    let plan = state
        .statement_to_plan(DFStatement::Statement(Box::new(statement)))
        .await?;
    ctx.execute_logical_plan(plan).await
}

/// The one statement of `text`, rewritten.
fn parse(state: &SessionState, text: &str) -> Result<Statement> {
    let mut statements = Parser::new(&GenericDialect {})
        .try_with_sql(text)
        .and_then(|mut parser| parser.parse_statements())
        .map_err(sql_error)?;
    let mut statement = match statements.len() {
        1 => statements.remove(0),
        count => return plan_err!("a query holds one statement, not {count}"),
    };
    let parameters = |name: &str| parameter_names(state, name);
    match VisitMut::visit(&mut statement, &mut Rewrite { parameters }) {
        ControlFlow::Break(error) => Err(error),
        ControlFlow::Continue(()) => Ok(statement),
    }
}

/// A parser error as DataFusion reports one.
fn sql_error(error: ParserError) -> DataFusionError {
    DataFusionError::SQL(Box::new(error), None)
}

/// The parameter names of the function `name` of `state`, a scalar, an aggregate, or a window
/// function, when its signature declares them.
fn parameter_names(state: &SessionState, name: &str) -> Option<Vec<String>> {
    let signature = if let Some(function) = state.scalar_functions().get(name) {
        function.signature()
    } else if let Some(function) = state.aggregate_functions().get(name) {
        function.signature()
    } else {
        state.window_functions().get(name)?.signature()
    };
    signature.parameter_names.clone()
}

/// The rewrites of each query and each function call of a statement.
struct Rewrite<F> {
    parameters: F,
}

impl<F: Fn(&str) -> Option<Vec<String>>> VisitorMut for Rewrite<F> {
    type Break = DataFusionError;

    fn pre_visit_query(&mut self, query: &mut Query) -> ControlFlow<DataFusionError> {
        if let SetExpr::Select(select) = query.body.as_mut() {
            if select.flavor == SelectFlavor::FromFirstNoSelect {
                select.projection =
                    vec![SelectItem::Wildcard(WildcardAdditionalOptions::default())];
                select.flavor = SelectFlavor::Standard;
            }
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &mut Expr) -> ControlFlow<DataFusionError> {
        if let Expr::Function(function) = expr {
            fill_omitted(function, &self.parameters);
        }
        ControlFlow::Continue(())
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
/// error.
fn fill_omitted(function: &mut Function, parameters: &dyn Fn(&str) -> Option<Vec<String>>) {
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
    let name = match function.name.0.as_slice() {
        [ObjectNamePart::Identifier(ident)] => match ident.quote_style {
            Some(_) => ident.value.clone(),
            None => ident.value.to_lowercase(),
        },
        _ => return,
    };
    let Some(parameters) = parameters(&name) else {
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
