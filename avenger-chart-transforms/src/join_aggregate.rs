use crate::aggregate::{
    AggregateGroupKeySpec, AggregateMeasureSpec, AggregateOp, aggregate_expr,
    map_aggregate_group_keys, map_aggregate_measures, validate_aggregate_ops,
};
use crate::common::{expr_node, simple_column_name, validate_output_names};
use async_trait::async_trait;
use avenger_chart_core::{
    AvengerChartError, CompiledDataTransform, DataTransform, DataTransformCompileContext,
    DataTransformExecutionContext, DataTransformResult, DefaultLogicalExprNodeExt, ExecutionShape,
    IntoExpr,
};
use datafusion::dataframe::DataFrame;
use datafusion::logical_expr::{Expr, WindowFunctionDefinition, col, expr::WindowFunction};
use indexmap::IndexSet;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledJoinAggregateTransform {
    pub group_by: Vec<AggregateGroupKeySpec>,
    pub measures: Vec<AggregateMeasureSpec>,
}

#[derive(Clone, Debug, Default)]
pub struct JoinAggregate {
    group_by: Vec<AggregateGroupKeySpec>,
    measures: Vec<AggregateMeasureSpec>,
}

impl JoinAggregate {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn group_by<I, E>(mut self, exprs: I) -> Self
    where
        I: IntoIterator<Item = E>,
        E: IntoExpr,
    {
        self.group_by.extend(exprs.into_iter().map(|expr| {
            let expr = expr.into_expr();
            let alias = simple_column_name(&expr);
            AggregateGroupKeySpec {
                expr: expr_node(expr, "joinaggregate group_by expression"),
                alias,
            }
        }));
        self
    }

    pub fn group_by_as(mut self, alias: impl Into<String>, expr: impl IntoExpr) -> Self {
        self.group_by.push(AggregateGroupKeySpec {
            expr: expr_node(expr.into_expr(), "joinaggregate group_by expression"),
            alias: Some(alias.into()),
        });
        self
    }

    pub fn sum(self, name: impl Into<String>, expr: impl IntoExpr) -> Self {
        self.measure(name, AggregateOp::Sum, Some(expr.into_expr()))
    }

    pub fn count(self, name: impl Into<String>) -> Self {
        self.measure(name, AggregateOp::Count, None)
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
            expr: expr.map(|expr| expr_node(expr, "joinaggregate measure expression")),
        });
        self
    }
}

impl DataTransform for JoinAggregate {
    type Output = ();

    fn into_compiled_and_output(
        self,
        _ctx: DataTransformCompileContext,
    ) -> Result<(Box<dyn CompiledDataTransform>, Self::Output), AvengerChartError> {
        if self.measures.is_empty() {
            return Err(AvengerChartError::InvalidArgument(
                "JoinAggregate transform requires at least one measure".to_string(),
            ));
        }
        validate_measure_names(&self.measures)?;
        validate_aggregate_ops(&self.measures)?;
        Ok((
            Box::new(CompiledJoinAggregateTransform {
                group_by: self.group_by,
                measures: self.measures,
            }),
            (),
        ))
    }
}

#[typetag::serde(name = "join_aggregate")]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl CompiledDataTransform for CompiledJoinAggregateTransform {
    fn clone_box(&self) -> Box<dyn CompiledDataTransform> {
        Box::new(self.clone())
    }

    fn execution_shape(&self) -> ExecutionShape {
        ExecutionShape::PlanRewrite
    }

    fn map_exprs(
        &self,
        f: &mut dyn FnMut(Expr) -> Result<Expr, AvengerChartError>,
    ) -> Result<Box<dyn CompiledDataTransform>, AvengerChartError> {
        Ok(Box::new(Self {
            group_by: map_aggregate_group_keys(&self.group_by, f)?,
            measures: map_aggregate_measures(&self.measures, f)?,
        }))
    }

    async fn apply(
        &self,
        dataframe: DataFrame,
        ctx: &DataTransformExecutionContext<'_>,
    ) -> Result<DataTransformResult, AvengerChartError> {
        if self.measures.is_empty() {
            return Err(AvengerChartError::InvalidArgument(
                "JoinAggregate transform requires at least one measure".to_string(),
            ));
        }
        validate_output_names(
            dataframe.schema().fields().iter().map(|field| field.name()),
            self.measures.iter().map(|measure| measure.name.as_str()),
        )?;

        let output_names = dataframe
            .schema()
            .fields()
            .iter()
            .map(|field| field.name().clone())
            .collect::<Vec<_>>();
        let measure_names = self
            .measures
            .iter()
            .map(|measure| measure.name.clone())
            .collect::<Vec<_>>();

        let partition_by = self
            .group_by
            .iter()
            .map(|group| group.expr.to_default_expr(ctx.session_context))
            .collect::<Result<Vec<_>, AvengerChartError>>()?;
        let window_exprs = self
            .measures
            .iter()
            .map(|measure| window_measure_expr(measure, &partition_by, ctx.session_context))
            .collect::<Result<Vec<_>, AvengerChartError>>()?;
        let result = dataframe
            .window(window_exprs)
            .map_err(AvengerChartError::DataFusionError)?;

        let mut projection = output_names
            .iter()
            .map(|name| col(name).alias(name))
            .collect::<Vec<_>>();
        projection.extend(measure_names.iter().map(|name| col(name).alias(name)));
        let result = result
            .select(projection)
            .map_err(AvengerChartError::DataFusionError)?;
        Ok(DataTransformResult::dataframe(result))
    }
}

/// Lower a measure to `agg(...) OVER (PARTITION BY group_keys)` with the
/// default unbounded frame, so every input row receives the aggregate value of
/// its partition. NULL group keys form their own partition, matching the
/// `IsNotDistinctFrom` join predicates this transform previously built.
fn window_measure_expr(
    measure: &AggregateMeasureSpec,
    partition_by: &[Expr],
    ctx: &datafusion::prelude::SessionContext,
) -> Result<Expr, AvengerChartError> {
    let Expr::Alias(alias) = aggregate_expr(measure, ctx)? else {
        return Err(AvengerChartError::InternalError(format!(
            "JoinAggregate measure '{}' did not lower to an aliased aggregate expression",
            measure.name
        )));
    };
    let Expr::AggregateFunction(aggregate) = *alias.expr else {
        return Err(AvengerChartError::InternalError(format!(
            "JoinAggregate measure '{}' did not lower to an aggregate function",
            measure.name
        )));
    };
    // The aggregate's within-group order_by (approx_percentile_cont) is
    // dropped: its accumulator reads values from the arguments and only uses
    // the ordering for the descending flag, which aggregate_expr never sets.
    let mut window = WindowFunction::new(
        WindowFunctionDefinition::AggregateUDF(aggregate.func),
        aggregate.params.args,
    );
    window.params.partition_by = partition_by.to_vec();
    Ok(Expr::from(window).alias(&measure.name))
}

fn validate_measure_names(measures: &[AggregateMeasureSpec]) -> Result<(), AvengerChartError> {
    let mut seen = IndexSet::<&str>::new();
    for measure in measures {
        if measure.name.is_empty() || measure.name.starts_with("__unused") {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Data transform output name '{}' is invalid",
                measure.name
            )));
        }
        if !seen.insert(measure.name.as_str()) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Data transform output name '{}' is duplicated",
                measure.name
            )));
        }
    }
    Ok(())
}
