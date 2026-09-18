use datafusion::{
    arrow::datatypes::DataType,
    functions::core::expr_fn::coalesce,
    functions_aggregate::{count::Count, expr_fn::sum},
    logical_expr::{col, expr::Cast, lit, Expr},
};

use crate::DirectReason;

/// An aggregate's sufficient state, its merge operation, and its finalizer.
/// Keep this separate from selection factorization and grouping. Other aggregate
/// recipes can add several state fields without changing the family lifecycle.
#[derive(Clone, Debug)]
pub(super) struct AggregateState {
    states: Vec<Expr>,
    merges: Vec<Expr>,
    finalized: Expr,
}

impl AggregateState {
    pub fn analyze(expr: &Expr, index: usize) -> std::result::Result<Self, DirectReason> {
        let expr = match expr {
            Expr::Alias(a) => a.expr.as_ref(),
            expr => expr,
        };
        let Expr::AggregateFunction(agg) = expr else {
            return Err(DirectReason::UnsupportedAggregate);
        };
        let p = &agg.params;
        if !agg.func.inner().is::<Count>()
            || p.distinct
            || p.filter.is_some()
            || !p.order_by.is_empty()
            || p.null_treatment.is_some()
            || p.args.len() != 1
            || !matches!(&p.args[0], Expr::Column(_) | Expr::Literal(..))
        {
            return Err(DirectReason::UnsupportedAggregate);
        }
        let state_name = format!("__selection_state_{index}");
        let merge_name = format!("__selection_merge_{index}");
        Ok(Self {
            states: vec![expr.clone().alias(&state_name)],
            merges: vec![sum(col(&state_name)).alias(&merge_name)],
            // SUM is nullable for an empty global input. COUNT is a non-null Int64.
            finalized: Expr::Cast(Cast::new(
                Box::new(coalesce(vec![col(&merge_name), lit(0_i64)])),
                DataType::Int64,
            )),
        })
    }

    pub fn state_expressions(&self) -> Vec<Expr> {
        self.states.clone()
    }

    pub fn merge_expressions(&self) -> Vec<Expr> {
        self.merges.clone()
    }

    pub fn finalize(&self) -> Expr {
        self.finalized.clone()
    }
}
