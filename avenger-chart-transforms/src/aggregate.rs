use crate::common::{
    expr_node, map_expr_node, map_optional_expr_node, simple_column_name, validate_output_names,
};
use async_trait::async_trait;
use avenger_chart_core::{
    AvengerChartError, CompiledDataTransform, DataTransform, DataTransformCompileContext,
    DataTransformExecutionContext, DataTransformResult, DefaultLogicalExprNodeExt, ExecutionShape,
    IntoExpr, SerializableExpr, simplify_to_scalar_sync,
};
use datafusion::{
    common::ScalarValue,
    dataframe::DataFrame,
    functions_aggregate::expr_fn::{approx_percentile_cont, avg, count, max, median, min, sum},
    logical_expr::{Expr, col, expr::Sort, lit},
};
use datafusion_proto::protobuf::LogicalExprNode;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledAggregateTransform {
    pub group_by: Vec<AggregateGroupKeySpec>,
    pub measures: Vec<AggregateMeasureSpec>,
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AggregateGroupKeySpec {
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub expr: LogicalExprNode,
    pub alias: Option<String>,
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AggregateMeasureSpec {
    pub name: String,
    pub op: AggregateOp,
    #[serde_as(as = "Option<FromInto<SerializableExpr>>")]
    pub expr: Option<LogicalExprNode>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum AggregateOp {
    Sum,
    Count,
    Mean,
    Min,
    Max,
    Median,
    ApproxPercentileCont {
        percentile: f64,
        centroids: Option<u32>,
    },
}

#[derive(Clone, Debug, Default)]
pub struct Aggregate {
    group_by: Vec<AggregateGroupKeySpec>,
    measures: Vec<AggregateMeasureSpec>,
}

impl Aggregate {
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
                expr: expr_node(expr, "aggregate group_by expression"),
                alias,
            }
        }));
        self
    }

    pub fn group_by_as(mut self, alias: impl Into<String>, expr: impl IntoExpr) -> Self {
        self.group_by.push(AggregateGroupKeySpec {
            expr: expr_node(expr.into_expr(), "aggregate group_by expression"),
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
            expr: expr.map(|expr| expr_node(expr, "aggregate measure expression")),
        });
        self
    }
}

impl DataTransform for Aggregate {
    type Output = AggregateOutput;

    fn into_compiled_and_output(
        self,
        _ctx: DataTransformCompileContext,
    ) -> Result<(Box<dyn CompiledDataTransform>, Self::Output), AvengerChartError> {
        validate_aggregate_ops(&self.measures)?;
        let mut names = IndexMap::new();
        for group in &self.group_by {
            if let Some(alias) = &group.alias {
                names.insert(alias.clone(), ());
            }
        }
        for measure in &self.measures {
            names.insert(measure.name.clone(), ());
        }
        let transform = CompiledAggregateTransform {
            group_by: self.group_by,
            measures: self.measures,
        };
        Ok((
            Box::new(transform),
            AggregateOutput {
                names: names.keys().cloned().collect(),
            },
        ))
    }
}

#[derive(Clone, Debug)]
pub struct AggregateOutput {
    names: Vec<String>,
}

impl AggregateOutput {
    pub fn output(&self, name: &str) -> Expr {
        if !self.names.iter().any(|candidate| candidate == name) {
            panic!("Unknown aggregate output '{name}'");
        }
        col(name)
    }
}

#[typetag::serde(name = "aggregate")]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl CompiledDataTransform for CompiledAggregateTransform {
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
        validate_output_names(
            dataframe.schema().fields().iter().map(|field| field.name()),
            {
                let mut names = Vec::new();
                for group in &self.group_by {
                    if let Some(alias) = &group.alias
                        && !group_alias_is_identity(group, alias, ctx.session_context)?
                    {
                        names.push(alias.as_str());
                    }
                }
                for measure in &self.measures {
                    names.push(measure.name.as_str());
                }
                names
            },
        )?;

        let group_exprs = self
            .group_by
            .iter()
            .map(|group| {
                let expr = group.expr.to_default_expr(ctx.session_context)?;
                Ok(match &group.alias {
                    Some(alias) => expr.alias(alias),
                    None => expr,
                })
            })
            .collect::<Result<Vec<_>, AvengerChartError>>()?;
        let agg_exprs = self
            .measures
            .iter()
            .map(|measure| aggregate_expr(measure, ctx.session_context))
            .collect::<Result<Vec<_>, _>>()?;
        let dataframe = dataframe
            .aggregate(group_exprs, agg_exprs)
            .map_err(AvengerChartError::DataFusionError)?;
        Ok(DataTransformResult::dataframe(dataframe))
    }
}

pub(crate) fn map_aggregate_group_keys(
    group_by: &[AggregateGroupKeySpec],
    f: &mut dyn FnMut(Expr) -> Result<Expr, AvengerChartError>,
) -> Result<Vec<AggregateGroupKeySpec>, AvengerChartError> {
    group_by
        .iter()
        .map(|group| {
            Ok(AggregateGroupKeySpec {
                expr: map_expr_node(&group.expr, f)?,
                alias: map_alias(&group.alias, f)?,
            })
        })
        .collect()
}

fn map_alias(
    alias: &Option<String>,
    f: &mut dyn FnMut(Expr) -> Result<Expr, AvengerChartError>,
) -> Result<Option<String>, AvengerChartError> {
    let Some(alias) = alias else {
        return Ok(None);
    };
    if !alias.contains("$__repeat_") {
        return Ok(Some(alias.clone()));
    }
    let mapped = f(lit(alias.clone()))?;
    match simplify_to_scalar_sync(mapped).map_err(AvengerChartError::DataFusionError)? {
        ScalarValue::Utf8(Some(value)) => Ok(Some(value)),
        other => Err(AvengerChartError::InvalidArgument(format!(
            "aggregate group_by alias expression must resolve to a string, got {other}"
        ))),
    }
}

pub(crate) fn map_aggregate_measures(
    measures: &[AggregateMeasureSpec],
    f: &mut dyn FnMut(Expr) -> Result<Expr, AvengerChartError>,
) -> Result<Vec<AggregateMeasureSpec>, AvengerChartError> {
    measures
        .iter()
        .map(|measure| {
            Ok(AggregateMeasureSpec {
                name: measure.name.clone(),
                op: measure.op,
                expr: map_optional_expr_node(&measure.expr, f)?,
            })
        })
        .collect()
}

pub(crate) fn aggregate_expr(
    measure: &AggregateMeasureSpec,
    ctx: &datafusion::prelude::SessionContext,
) -> Result<Expr, AvengerChartError> {
    let expr = match measure.op {
        AggregateOp::Count => count(lit(1)),
        AggregateOp::Sum => sum(required_measure_expr(measure, ctx)?),
        AggregateOp::Mean => avg(required_measure_expr(measure, ctx)?),
        AggregateOp::Min => min(required_measure_expr(measure, ctx)?),
        AggregateOp::Max => max(required_measure_expr(measure, ctx)?),
        AggregateOp::Median => median(required_measure_expr(measure, ctx)?),
        AggregateOp::ApproxPercentileCont {
            percentile,
            centroids,
        } => approx_percentile_cont(
            Sort {
                expr: required_measure_expr(measure, ctx)?,
                asc: true,
                nulls_first: true,
            },
            lit(percentile),
            centroids.map(|centroids| lit(centroids as i64)),
        ),
    };
    Ok(expr.alias(&measure.name))
}

pub(crate) fn validate_aggregate_ops(
    measures: &[AggregateMeasureSpec],
) -> Result<(), AvengerChartError> {
    for measure in measures {
        if let AggregateOp::ApproxPercentileCont {
            percentile,
            centroids,
        } = measure.op
        {
            if !percentile.is_finite() || !(0.0..=1.0).contains(&percentile) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Aggregate measure '{}' has invalid percentile {percentile}; expected a finite value between 0.0 and 1.0",
                    measure.name
                )));
            }
            if centroids == Some(0) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Aggregate measure '{}' has invalid centroid count 0; expected at least 1",
                    measure.name
                )));
            }
        }
    }
    Ok(())
}

fn required_measure_expr(
    measure: &AggregateMeasureSpec,
    ctx: &datafusion::prelude::SessionContext,
) -> Result<Expr, AvengerChartError> {
    let Some(expr) = &measure.expr else {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Aggregate measure '{}' requires an input expression",
            measure.name
        )));
    };
    expr.to_default_expr(ctx)
}

fn group_alias_is_identity(
    group: &AggregateGroupKeySpec,
    alias: &str,
    ctx: &datafusion::prelude::SessionContext,
) -> Result<bool, AvengerChartError> {
    Ok(simple_column_name(&group.expr.to_default_expr(ctx)?).as_deref() == Some(alias))
}

#[cfg(test)]
mod tests {
    use avenger_chart_core::{
        RepeatContext, ResolvedRepeatVariable, repeat, resolve_repeat_placeholders,
    };
    use datafusion::{logical_expr::col, prelude::SessionContext};

    use super::*;

    #[test]
    fn map_group_keys_resolves_repeat_name_aliases() {
        let group_by = vec![AggregateGroupKeySpec {
            expr: expr_node(
                col(repeat::column_name()),
                "aggregate group_by repeat column name",
            ),
            alias: Some(repeat::column_name()),
        }];
        let repeat_context = RepeatContext::new().with_column(
            ResolvedRepeatVariable {
                id: "team".to_string(),
                expr: col("team"),
                title: "Team".to_string(),
                type_hint: None,
            },
            0,
            2,
        );

        let mapped = map_aggregate_group_keys(&group_by, &mut |expr| {
            resolve_repeat_placeholders(expr, &repeat_context)
        })
        .expect("resolve repeat placeholders");

        assert_eq!(mapped[0].alias.as_deref(), Some("team"));
        assert_eq!(
            mapped[0]
                .expr
                .to_default_expr(&SessionContext::new())
                .expect("expr deserializes")
                .to_string(),
            "team"
        );
    }
}
