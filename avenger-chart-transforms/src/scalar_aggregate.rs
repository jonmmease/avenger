//! Whole-input scalar aggregations published as derived scalars.
//!
//! `ScalarAggregate` completes a triangle with the other aggregation
//! transforms:
//!
//! - [`Aggregate`](crate::Aggregate) collapses rows into groups;
//! - [`JoinAggregate`](crate::JoinAggregate) appends aggregate values as
//!   columns on every row;
//! - `ScalarAggregate` passes the input through unchanged and publishes the
//!   aggregations as scalar expressions (derived scalars) that later
//!   transform stages and channel expressions can reference.
//!
//! Each named measure summarizes the input *at the transform's position in
//! the chain* — after upstream filters and facet narrowing. Placement is
//! meaningful: `Filter -> ScalarAggregate::count` counts the filtered rows.
//!
//! # Evaluation modes
//!
//! - [`ScalarAggregateEvaluation::Eager`] (default): `apply` executes the
//!   one-row aggregation immediately (with the execution context's params
//!   bound) and publishes concrete literal values. The query runs exactly
//!   once per chain application, the value works in every expression
//!   context, and it is inspectable in diagnostics. This is the first
//!   transform whose `apply` executes a query rather than only building
//!   plans; the cost is surfaced through a `tracing` span.
//! - [`ScalarAggregateEvaluation::Lazy`]: `apply` publishes an uncorrelated
//!   [`Expr::ScalarSubquery`] over the input plan per measure. `apply` stays
//!   a pure plan-builder and the value is computed wherever consumers
//!   evaluate; each reference site inlines a clone of the subquery plan.
//!
//! Empty input yields `count = 0` and `NULL` for the other measures, in both
//! modes.
//!
//! # Example: scalar gate
//!
//! Render a mark's rows only when few enough rows survive an upstream
//! filter:
//!
//! ```ignore
//! mark.transform(Filter::new(predicate), |mark, _| mark)
//!     .transform(ScalarAggregate::new().count("n"), |mark, stats| {
//!         mark.transform(Filter::new(stats.scalar("n").lt(lit(10_000))), |mark, _| mark)
//!             .x(col("x"))
//!             .y(col("y"))
//!     })
//! ```
//!
//! # Example: scalar-normalized channel
//!
//! ```ignore
//! mark.transform(ScalarAggregate::new().max("max_pop", col("population")), |mark, s| {
//!     mark.size(col("population") / s.scalar("max_pop") * lit(400.0))
//! })
//! ```

use std::sync::Arc;

use crate::aggregate::{
    AggregateMeasureSpec, AggregateOp, aggregate_expr, map_aggregate_measures,
    validate_aggregate_ops,
};
use async_trait::async_trait;
use avenger_chart_core::{
    AvengerChartError, CompiledDataTransform, DataTransform, DataTransformCompileContext,
    DataTransformExecutionContext, DataTransformResult, DerivedScalarMap, IntoExpr,
    derived_scalar, params_to_datafusion,
};
use datafusion::{
    arrow::datatypes::DataType,
    common::{ParamValues, ScalarValue},
    dataframe::DataFrame,
    logical_expr::{Expr, expr_fn::scalar_subquery, lit},
};
use serde::{Deserialize, Serialize};
use tracing::Instrument;

use crate::common::expr_node;

/// When `ScalarAggregate` computes its published scalars.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScalarAggregateEvaluation {
    /// Execute the aggregation during `apply` and publish literal values.
    #[default]
    Eager,
    /// Publish uncorrelated scalar subqueries over the input plan; the value
    /// is computed wherever consumers evaluate.
    Lazy,
}

/// Pass-through transform publishing whole-input aggregations as derived
/// scalars. See the module docs for semantics and examples.
#[derive(Clone, Debug, Default)]
pub struct ScalarAggregate {
    measures: Vec<AggregateMeasureSpec>,
    evaluation: ScalarAggregateEvaluation,
}

impl ScalarAggregate {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the evaluation mode. `Eager` is the default.
    pub fn evaluation(mut self, evaluation: ScalarAggregateEvaluation) -> Self {
        self.evaluation = evaluation;
        self
    }

    /// Publish scalars as uncorrelated subqueries instead of eager literals.
    pub fn lazy(self) -> Self {
        self.evaluation(ScalarAggregateEvaluation::Lazy)
    }

    pub fn count(self, name: impl Into<String>) -> Self {
        self.measure(name, AggregateOp::Count, None)
    }

    pub fn sum(self, name: impl Into<String>, expr: impl IntoExpr) -> Self {
        self.measure(name, AggregateOp::Sum, Some(expr.into_expr()))
    }

    pub fn mean(self, name: impl Into<String>, expr: impl IntoExpr) -> Self {
        self.measure(name, AggregateOp::Mean, Some(expr.into_expr()))
    }

    pub fn min(self, name: impl Into<String>, expr: impl IntoExpr) -> Self {
        self.measure(name, AggregateOp::Min, Some(expr.into_expr()))
    }

    pub fn max(self, name: impl Into<String>, expr: impl IntoExpr) -> Self {
        self.measure(name, AggregateOp::Max, Some(expr.into_expr()))
    }

    pub fn median(self, name: impl Into<String>, expr: impl IntoExpr) -> Self {
        self.measure(name, AggregateOp::Median, Some(expr.into_expr()))
    }

    pub fn approx_percentile_cont(
        self,
        name: impl Into<String>,
        expr: impl IntoExpr,
        percentile: f64,
    ) -> Self {
        self.measure(
            name,
            AggregateOp::ApproxPercentileCont {
                percentile,
                centroids: None,
            },
            Some(expr.into_expr()),
        )
    }

    pub fn approx_percentile_cont_with_centroids(
        self,
        name: impl Into<String>,
        expr: impl IntoExpr,
        percentile: f64,
        centroids: u32,
    ) -> Self {
        self.measure(
            name,
            AggregateOp::ApproxPercentileCont {
                percentile,
                centroids: Some(centroids),
            },
            Some(expr.into_expr()),
        )
    }

    fn measure(mut self, name: impl Into<String>, op: AggregateOp, expr: Option<Expr>) -> Self {
        self.measures.push(AggregateMeasureSpec {
            name: name.into(),
            op,
            expr: expr.map(|expr| expr_node(expr, "scalar aggregate measure expression")),
        });
        self
    }
}

impl DataTransform for ScalarAggregate {
    type Output = ScalarAggregateOutput;

    fn into_compiled_and_output(
        self,
        _ctx: DataTransformCompileContext,
    ) -> Result<(Box<dyn CompiledDataTransform>, Self::Output), AvengerChartError> {
        if self.measures.is_empty() {
            return Err(AvengerChartError::InvalidArgument(
                "ScalarAggregate requires at least one measure".to_string(),
            ));
        }
        validate_aggregate_ops(&self.measures)?;
        let mut names: Vec<(String, AggregateOp)> = Vec::with_capacity(self.measures.len());
        for measure in &self.measures {
            if names.iter().any(|(name, _)| name == &measure.name) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "ScalarAggregate measure name '{}' is declared more than once",
                    measure.name
                )));
            }
            names.push((measure.name.clone(), measure.op));
        }
        let transform = CompiledScalarAggregateTransform {
            measures: self.measures,
            evaluation: self.evaluation,
        };
        Ok((Box::new(transform), ScalarAggregateOutput { names }))
    }
}

/// Output handle for [`ScalarAggregate`]: references published scalars by
/// their measure names.
#[derive(Clone, Debug)]
pub struct ScalarAggregateOutput {
    names: Vec<(String, AggregateOp)>,
}

impl ScalarAggregateOutput {
    /// Derived-scalar placeholder expression for a named measure.
    pub fn scalar(&self, name: &str) -> Expr {
        let Some((_, op)) = self
            .names
            .iter()
            .find(|(candidate, _)| candidate == name)
        else {
            panic!("Unknown scalar aggregate '{name}'");
        };
        derived_scalar(name, measure_placeholder_dtype(*op))
    }
}

fn measure_placeholder_dtype(op: AggregateOp) -> Option<DataType> {
    match op {
        AggregateOp::Count => Some(DataType::Int64),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledScalarAggregateTransform {
    pub measures: Vec<AggregateMeasureSpec>,
    #[serde(default)]
    pub evaluation: ScalarAggregateEvaluation,
}

#[typetag::serde(name = "scalar_aggregate")]
#[async_trait]
impl CompiledDataTransform for CompiledScalarAggregateTransform {
    fn clone_box(&self) -> Box<dyn CompiledDataTransform> {
        Box::new(self.clone())
    }

    fn map_exprs(
        &self,
        f: &mut dyn FnMut(Expr) -> Result<Expr, AvengerChartError>,
    ) -> Result<Box<dyn CompiledDataTransform>, AvengerChartError> {
        Ok(Box::new(Self {
            measures: map_aggregate_measures(&self.measures, f)?,
            evaluation: self.evaluation,
        }))
    }

    async fn apply(
        &self,
        dataframe: DataFrame,
        ctx: &DataTransformExecutionContext<'_>,
    ) -> Result<DataTransformResult, AvengerChartError> {
        let derived_scalars = match self.evaluation {
            ScalarAggregateEvaluation::Eager => self.eager_scalars(&dataframe, ctx).await?,
            ScalarAggregateEvaluation::Lazy => self.lazy_scalars(&dataframe, ctx)?,
        };
        Ok(DataTransformResult {
            dataframe,
            derived_scalars,
        })
    }
}

impl CompiledScalarAggregateTransform {
    /// Execute one aggregation over the input (all measures in a single
    /// query, params bound the same way final mark-data collection binds
    /// them) and publish each measure as a literal.
    async fn eager_scalars(
        &self,
        dataframe: &DataFrame,
        ctx: &DataTransformExecutionContext<'_>,
    ) -> Result<DerivedScalarMap, AvengerChartError> {
        let agg_exprs = self
            .measures
            .iter()
            .map(|measure| aggregate_expr(measure, ctx.session_context))
            .collect::<Result<Vec<_>, _>>()?;
        let aggregated = dataframe
            .clone()
            .aggregate(vec![], agg_exprs)
            .map_err(AvengerChartError::DataFusionError)?
            .with_param_values(
                params_to_datafusion(ctx.params)
                    .unwrap_or_else(|| ParamValues::Map(Default::default())),
            )
            .map_err(AvengerChartError::DataFusionError)?;

        let span = tracing::debug_span!(
            "scalar_aggregate_eager",
            measures = self.measures.len(),
        );
        let batches = async { aggregated.collect().await }
            .instrument(span)
            .await
            .map_err(|err| {
                AvengerChartError::InternalError(format!(
                    "ScalarAggregate failed to evaluate eager measures {:?}: {err}",
                    self.measures
                        .iter()
                        .map(|measure| measure.name.as_str())
                        .collect::<Vec<_>>()
                ))
            })?;
        let batch = batches.first().ok_or_else(|| {
            AvengerChartError::InternalError(
                "ScalarAggregate eager evaluation returned no batches".to_string(),
            )
        })?;

        let mut derived = DerivedScalarMap::new();
        for measure in &self.measures {
            let column_index = batch.schema().index_of(&measure.name).map_err(|err| {
                AvengerChartError::InternalError(format!(
                    "ScalarAggregate measure '{}' missing from eager result: {err}",
                    measure.name
                ))
            })?;
            let value = ScalarValue::try_from_array(batch.column(column_index), 0)
                .map_err(AvengerChartError::DataFusionError)?;
            tracing::debug!(
                measure = measure.name.as_str(),
                value = %value,
                "scalar aggregate eager value"
            );
            derived.insert(measure.name.clone(), lit(value));
        }
        Ok(derived)
    }

    /// Publish each measure as an uncorrelated scalar subquery over the
    /// input plan. One single-column plan per measure; the unoptimized plan
    /// is used so the consuming query's optimizer sees the raw shape.
    fn lazy_scalars(
        &self,
        dataframe: &DataFrame,
        ctx: &DataTransformExecutionContext<'_>,
    ) -> Result<DerivedScalarMap, AvengerChartError> {
        let mut derived = DerivedScalarMap::new();
        for measure in &self.measures {
            let agg_expr = aggregate_expr(measure, ctx.session_context)?;
            let plan = dataframe
                .clone()
                .aggregate(vec![], vec![agg_expr])
                .map_err(AvengerChartError::DataFusionError)?
                .into_unoptimized_plan();
            derived.insert(measure.name.clone(), scalar_subquery(Arc::new(plan)));
        }
        Ok(derived)
    }
}
