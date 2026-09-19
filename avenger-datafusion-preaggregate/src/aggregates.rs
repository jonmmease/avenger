use avenger_datafusion_aggregate_state::functions as state;
use datafusion::{
    common::{DFSchema, DataFusionError, Result},
    functions_aggregate::{
        average::Avg,
        count::Count,
        min_max::{Max, Min},
        stddev::{Stddev, StddevPop},
        sum::Sum,
        variance::{VariancePopulation, VarianceSample},
    },
    logical_expr::{col, lit, Expr, ExprFunctionExt, ExprSchemable},
};

use crate::DirectReason;
use crate::{expressions, Eligibility, ExpressionProperties};
use std::sync::Arc;

/// One opaque state column and the aggregate that merges it into a final value.
#[derive(Clone, Debug)]
pub(super) struct AggregateRewrite {
    pub state: Expr,
    pub merge: Expr,
}

impl AggregateRewrite {
    pub fn analyze(
        expr: &Expr,
        index: usize,
        schema: &DFSchema,
        properties: &Arc<dyn ExpressionProperties>,
    ) -> Result<Eligibility<Self>> {
        let expr = match expr {
            Expr::Alias(a) => a.expr.as_ref(),
            expr => expr,
        };
        let Expr::AggregateFunction(agg) = expr else {
            return Ok(Err(DirectReason::UnsupportedAggregate));
        };
        let p = &agg.params;
        let count = agg.func.inner().is::<Count>();
        if p.distinct
            || !p.order_by.is_empty()
            || p.null_treatment.is_some()
            || !(p.args.len() == 1 || count && p.args.is_empty())
        {
            return Ok(Err(DirectReason::UnsupportedAggregate));
        }
        let implementation = agg.func.inner();
        let (state, merge) = if count {
            (state::count_state_udaf(), state::count_merge_udaf())
        } else if implementation.is::<Sum>() {
            (state::sum_state_udaf(), state::sum_merge_udaf())
        } else if implementation.is::<Min>() {
            (state::min_state_udaf(), state::min_merge_udaf())
        } else if implementation.is::<Max>() {
            (state::max_state_udaf(), state::max_merge_udaf())
        } else if implementation.is::<Avg>() {
            (state::avg_state_udaf(), state::avg_merge_udaf())
        } else if implementation.is::<VarianceSample>() {
            (state::var_samp_state_udaf(), state::var_samp_merge_udaf())
        } else if implementation.is::<VariancePopulation>() {
            (state::var_pop_state_udaf(), state::var_pop_merge_udaf())
        } else if implementation.is::<Stddev>() {
            (
                state::stddev_samp_state_udaf(),
                state::stddev_samp_merge_udaf(),
            )
        } else if implementation.is::<StddevPop>() {
            (
                state::stddev_pop_state_udaf(),
                state::stddev_pop_merge_udaf(),
            )
        } else {
            return Ok(Err(DirectReason::UnsupportedAggregate));
        };
        let arg = expressions::coerce(
            p.args.first().cloned().unwrap_or_else(|| lit(1_i64)),
            schema,
        )?;
        let types = match state.coerce_types(&[arg.get_type(schema)?]) {
            Ok(types) => types,
            Err(DataFusionError::Plan(_) | DataFusionError::NotImplemented(_)) => {
                return Ok(Err(DirectReason::UnsupportedAggregate));
            }
            Err(error) => return Err(error),
        };
        // The merge plan needs the coerced state type before execution-time analysis.
        let arg = arg.cast_to(&types[0], schema)?;
        if !expressions::properties(&arg, schema, properties).total {
            return Ok(Err(DirectReason::UnsafeMovedExpression));
        }
        let filter = p
            .filter
            .as_ref()
            .map(|f| expressions::boolean(f.as_ref().clone(), schema, false))
            .transpose()?;
        if filter
            .as_ref()
            .is_some_and(|f| !expressions::properties(f, schema, properties).total)
        {
            return Ok(Err(DirectReason::UnsafeMovedExpression));
        }
        let state_name = format!("__preagg_state_{index}");
        let state_expr = match filter {
            Some(filter) => state.call(vec![arg]).filter(filter).build()?,
            None => state.call(vec![arg]),
        };
        Ok(Ok(Self {
            state: state_expr.alias(&state_name),
            merge: merge
                .call(vec![col(state_name)])
                .alias(format!("__preagg_merge_{index}")),
        }))
    }
}
