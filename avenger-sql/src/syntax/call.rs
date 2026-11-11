//! Pipe `CALL`: DataFusion 55.2 does not plan a pipe `CALL` of a table function, so the pass
//! writes `|> CALL f(<args>) [OVER (PARTITION BY ...)] [AS t]` as `SELECT * FROM f((<prefix>),
//! <args>, partition_by => [...]) [AS t]`, the `FROM`-position call over the query before it.
//! Removed by DataFusion planning a pipe `CALL` with a table argument.

use datafusion::{
    common::Result,
    sql::sqlparser::ast::{
        Array, Expr, Function, FunctionArg, FunctionArgExpr, FunctionArgOperator,
        FunctionArguments, Ident, PipeOperator, Query, SetExpr, Spanned, TableAlias, TableFactor,
        TableFunctionArgs, WindowType,
    },
};

use super::{build, split_prefix};
use crate::{chart::PARTITION_BY, error::at};

/// Whether `operator` is a pipe `CALL`.
pub(super) fn is_call(operator: &PipeOperator) -> bool {
    matches!(operator, PipeOperator::Call { .. })
}

/// Writes the pipe `CALL` at `index` of `query` as the `FROM`-position call of its function over
/// the prefix, the query before the operator. The `OVER` clause of the call has only a
/// `PARTITION BY` list of columns (`r[call.over.column]`).
pub(super) fn split(query: &mut Query, index: usize) -> Result<()> {
    let (prefix, operator) = split_prefix(query, index);
    let PipeOperator::Call { function, alias } = operator else {
        unreachable!("the operator is a pipe CALL")
    };
    let partition = match function.over {
        Some(_) => Some(partition(&function)?),
        None => None,
    };
    let span = function.name.span();
    let call_args = match function.args {
        FunctionArguments::List(list) => list.args,
        FunctionArguments::None => Vec::new(),
        FunctionArguments::Subquery(query) => {
            vec![FunctionArg::Unnamed(FunctionArgExpr::Expr(Expr::Subquery(
                query,
            )))]
        }
    };
    let mut args = vec![FunctionArg::Unnamed(FunctionArgExpr::Expr(Expr::Subquery(
        Box::new(prefix),
    )))];
    args.extend(call_args);
    if let Some(partition) = partition {
        args.push(FunctionArg::Named {
            name: Ident::with_span(span, PARTITION_BY),
            arg: FunctionArgExpr::Expr(Expr::Array(Array {
                elem: partition,
                named: false,
            })),
            operator: FunctionArgOperator::RightArrow,
        });
    }
    let table = TableFactor::Table {
        name: function.name,
        alias: alias.map(|name| TableAlias {
            explicit: true,
            name,
            columns: Vec::new(),
            at: None,
        }),
        args: Some(TableFunctionArgs {
            args,
            settings: None,
        }),
        with_hints: Vec::new(),
        version: None,
        with_ordinality: false,
        partitions: Vec::new(),
        json_path: None,
        sample: None,
        index_hints: Vec::new(),
    };
    *query.body = SetExpr::Select(Box::new(build::select(
        vec![build::wildcard()],
        Some(table),
    )));
    Ok(())
}

/// The partition columns of the `OVER` clause of the pipe `CALL` of `function`, or none without
/// one.
fn partition(function: &Function) -> Result<Vec<Expr>> {
    let operator = format!("CALL {}", function.name);
    let form = format!("the OVER clause of {operator} has only a PARTITION BY list of columns");
    match &function.over {
        None => Ok(Vec::new()),
        Some(WindowType::NamedWindow(name)) => Err(at(name.span, form)),
        Some(WindowType::WindowSpec(spec)) => {
            if let Some(name) = &spec.window_name {
                return Err(at(name.span, form));
            }
            if let Some(item) = spec.order_by.first() {
                return Err(at(item.expr.span(), format!("{form}, not an ORDER BY")));
            }
            if spec.window_frame.is_some() {
                return Err(at(function.name.span(), format!("{form}, not a frame")));
            }
            for item in &spec.partition_by {
                if !matches!(item, Expr::Identifier(_) | Expr::CompoundIdentifier(_)) {
                    return Err(at(
                        item.span(),
                        format!("a partition of {operator} is a column, not `{item}`"),
                    ));
                }
            }
            Ok(spec.partition_by.clone())
        }
    }
}
