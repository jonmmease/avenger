//! The rule that plans `timeunit(..) OVER (..)` as scalar functions, aggregates, and joins.

use super::*;

use std::sync::Arc;

use crate::utils::{
    PartitionCalls, PartitionValue, is_call, literals, partition_calls, partition_values,
    split_window,
};
use datafusion::common::tree_node::Transformed;
use datafusion::common::{Column, Result};
use datafusion::logical_expr::{AggregateUDF, Expr, LogicalPlan, ScalarUDF};
use datafusion::optimizer::{ApplyOrder, OptimizerConfig, OptimizerRule};

#[derive(Debug)]
pub struct TimeUnitWindowRewrite {
    pub(super) scalar: Arc<ScalarUDF>,
    pub(super) apply: Arc<ScalarUDF>,
    pub(super) choose: Arc<AggregateUDF>,
}

impl Default for TimeUnitWindowRewrite {
    fn default() -> Self {
        Self {
            scalar: Arc::new(ScalarUDF::new_from_impl(TimeUnitScalar::default())),
            apply: Arc::new(ScalarUDF::new_from_impl(TimeUnitApply::default())),
            choose: Arc::new(AggregateUDF::new_from_impl(TimeUnitChoose::default())),
        }
    }
}

impl OptimizerRule for TimeUnitWindowRewrite {
    fn name(&self) -> &str {
        "timeunit_window_rewrite"
    }

    fn apply_order(&self) -> Option<ApplyOrder> {
        Some(ApplyOrder::BottomUp)
    }

    fn rewrite(
        &self,
        plan: LogicalPlan,
        _config: &dyn OptimizerConfig,
    ) -> Result<Transformed<LogicalPlan>> {
        let LogicalPlan::Window(window) = &plan else {
            return Ok(Transformed::no(plan));
        };
        if let Some(split) = split_window(window, |_, e| is_call::<TimeUnitWindow>(e))? {
            let lower = self.rewrite(split.lower.clone(), _config)?.data;
            return Ok(Transformed::yes(split.rebuild(lower, Ok)?));
        }
        let Some(PartitionCalls { calls, keys }) =
            partition_calls::<TimeUnitWindow>(window, "timeunit")?
        else {
            return Ok(Transformed::no(plan));
        };
        let mut options = Vec::new();
        for call in &calls {
            let exprs = call.params.args[1..].to_vec();
            let o = TimeUnitOptions::parse(&literals("timeunit", &exprs)?)?;
            options.push((exprs, o));
        }

        let input = window.input.as_ref().clone();
        let n_in = input.schema().fields().len();
        let mut exprs: Vec<Expr> = input
            .schema()
            .columns()
            .into_iter()
            .map(Expr::Column)
            .collect();
        let out = |i: usize| window.schema.qualified_field(n_in + i);

        // The unit of each call without one: chosen from the values of each partition. A call
        // with a unit is the scalar function.
        let mut units: Vec<Option<Expr>> = vec![None; calls.len()];
        let auto: Vec<usize> = (0..calls.len())
            .filter(|&i| options[i].1.unit.is_none())
            .collect();
        let values = auto
            .iter()
            .map(|&i| {
                let alias = format!("__timeunit_unit_{i}");
                let mut args = vec![calls[i].params.args[0].clone()];
                args.extend(options[i].0.iter().cloned());
                PartitionValue {
                    aggregates: vec![self.choose.call(args).alias(&alias)],
                    value: Expr::Column(Column::from_name(alias)),
                }
            })
            .collect();
        let (builder, chosen) = partition_values(input, &keys, values, "timeunit")?;
        for (i, e) in auto.into_iter().zip(chosen) {
            units[i] = Some(e);
        }

        for (i, call) in calls.iter().enumerate() {
            let (q, f) = out(i);
            let x = call.params.args[0].clone();
            let e = match units[i].take() {
                None => {
                    let mut args = vec![x];
                    args.extend(options[i].0.iter().cloned());
                    self.scalar.call(args)
                }
                Some(unit) => {
                    let mut args = vec![x, unit];
                    args.extend(options[i].0.iter().cloned());
                    self.apply.call(args)
                }
            };
            exprs.push(e.alias_qualified(q.cloned(), f.name()));
        }
        Ok(Transformed::yes(builder.project(exprs)?.build()?))
    }
}
