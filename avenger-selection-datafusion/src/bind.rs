//! Binding selections' states to a plan: each `selected` call on a state's placeholder lowers to
//! a filter of that state, and each `in_keys` call to a check of the owner's clauses. A plan that
//! runs with either call still in it fails with an error that names the selection that was not
//! registered.

use std::{any::Any, collections::BTreeMap};

use avenger_selection::SelectionState;
use datafusion::{
    common::{
        config::ConfigOptions,
        plan_err,
        tree_node::{Transformed, TransformedResult, TreeNode},
        DFSchema, Result,
    },
    logical_expr::{
        expr::{Placeholder, ScalarFunction},
        expr_rewriter::{FunctionRewrite, NamePreserver},
        utils::merge_schema,
        Expr, ExprSchemable, LogicalPlan, ScalarUDF,
    },
};

use crate::{
    functions::{owners_clauses, InKeys},
    lower::{fold_options, is_selected, Call},
};

/// `plan` with each `selected` call whose state is a placeholder that `states` names, such as
/// `("brush", &state)` for `$brush`, lowered to a filter of that state, and each `in_keys` call
/// of a pre-aggregation's rollup to a check of the owner's clauses in that state. The state never becomes a
/// value of the plan, so planning does not copy or format the state's values. Bind the plan's
/// other placeholders with `with_param_values` afterwards, and give the calls' options as
/// constants. A `selected` call whose placeholder `states` does not name fails when the plan runs,
/// with an error that names its selection.
///
/// Bind an unoptimized plan, such as from `DataFrame::into_unoptimized_plan`. A `selected` call
/// names no columns until it is bound, so the optimizer's projection pushdown can drop the
/// columns that its conditions read, and binding then fails for a column that the input does not
/// have. The lowered call takes those columns as its arguments, so optimizing after binding keeps
/// them.
pub fn bind_selections(
    plan: LogicalPlan,
    states: &[(&str, &SelectionState)],
) -> Result<LogicalPlan> {
    let states: BTreeMap<String, &SelectionState> = (states.iter())
        .map(|(name, state)| (format!("${}", name.trim_start_matches('$')), *state))
        .collect();
    plan.transform_up_with_subqueries(|plan| {
        let mut schema = merge_schema(&plan.inputs());
        if let LogicalPlan::TableScan(scan) = &plan {
            schema.merge(&DFSchema::try_from_qualified_schema(
                scan.table_name.clone(),
                &scan.source.schema(),
            )?);
        }
        let names = NamePreserver::new(&plan);
        plan.map_expressions(|expr| {
            let name = names.save(&expr);
            let bound = expr.transform_up(|expr| bind(expr, &schema, &states))?;
            Ok(bound.update_data(|expr| name.restore(expr)))
        })
    })
    .data()
}

/// `expr` lowered for its state when it is a `selected` call or an `in_keys` call that takes the
/// state, on one of the placeholders of `states`.
fn bind(
    expr: Expr,
    schema: &DFSchema,
    states: &BTreeMap<String, &SelectionState>,
) -> Result<Transformed<Expr>> {
    let Expr::ScalarFunction(ScalarFunction { func, args }) = &expr else {
        return Ok(Transformed::no(expr));
    };
    let state = match args.first() {
        Some(Expr::Placeholder(Placeholder { id, .. })) => states.get(id),
        _ => None,
    };
    let Some(state) = state else {
        return Ok(Transformed::no(expr));
    };
    if is_selected(func) {
        let call = Call::new((*state).clone(), &fold_options(args, schema)?)?;
        return Ok(Transformed::yes(call.lower(schema)?));
    }
    match (marker(func), args.as_slice()) {
        (Some(_), [_, Expr::Literal(owner, _), keys]) => {
            let clauses = owners_clauses(state, owner.try_as_str().flatten());
            let function = ScalarUDF::from(InKeys::of_clauses(&clauses, &keys.get_type(schema)?)?);
            Ok(Transformed::yes(function.call(vec![keys.clone()])))
        }
        _ => Ok(Transformed::no(expr)),
    }
}

/// `func` as an `in_keys` function that takes the state and the owner.
pub(crate) fn marker(func: &ScalarUDF) -> Option<&InKeys> {
    let in_keys = (func.inner().as_ref() as &dyn Any).downcast_ref::<InKeys>()?;
    in_keys.takes_state().then_some(in_keys)
}

/// The function rewrite that fails for a `selected` or an `in_keys` call that `bind_selections`
/// did not lower,
/// with an error that names the selection that was not registered. DataFusion applies function
/// rewrites before it plans the rest of the query.
#[derive(Debug)]
pub(crate) struct Unbound;

impl FunctionRewrite for Unbound {
    fn name(&self) -> &str {
        "avenger_selection_unbound"
    }

    fn rewrite(&self, expr: Expr, _: &DFSchema, _: &ConfigOptions) -> Result<Transformed<Expr>> {
        let Expr::ScalarFunction(ScalarFunction { func, args }) = &expr else {
            return Ok(Transformed::no(expr));
        };
        if !is_selected(func) && marker(func).is_none() {
            return Ok(Transformed::no(expr));
        }
        match args.first() {
            Some(Expr::Placeholder(Placeholder { id, .. })) => {
                plan_err!(
                    "selection `{}` was not registered",
                    id.trim_start_matches('$')
                )
            }
            Some(other) => plan_err!(
                "`{}` takes the placeholder of a selection, such as `$brush`, not {other}",
                func.name()
            ),
            None => plan_err!(
                "`{}` takes the placeholder of a selection, such as `$brush`",
                func.name()
            ),
        }
    }
}
