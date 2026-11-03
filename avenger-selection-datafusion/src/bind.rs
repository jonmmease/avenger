//! Binding selections' states to a plan: each `selected` call on a state's placeholder lowers to
//! a filter of that state. A plan that runs with a `selected` call still in it fails with an error
//! that names the selection that was not registered.

use std::collections::BTreeMap;

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
        Expr, LogicalPlan,
    },
};

use crate::lower::{fold_options, is_selected, Call};

/// `plan` with each `selected` call whose state is a placeholder that `states` names, such as
/// `("brush", &state)` for `$brush`, lowered to a filter of that state. The state never becomes a
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

/// `expr` lowered for its state when it is a `selected` call on one of the placeholders of
/// `states`.
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
    match state {
        Some(state) if is_selected(func) => {
            let call = Call::new((*state).clone(), &fold_options(args, schema)?)?;
            Ok(Transformed::yes(call.lower(schema)?))
        }
        _ => Ok(Transformed::no(expr)),
    }
}

/// The function rewrite that fails for a `selected` call that `bind_selections` did not lower,
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
        if !is_selected(func) {
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
                "`selected` takes the placeholder of a selection, such as `$brush`, not {other}"
            ),
            None => plan_err!("`selected` takes the placeholder of a selection, such as `$brush`"),
        }
    }
}
