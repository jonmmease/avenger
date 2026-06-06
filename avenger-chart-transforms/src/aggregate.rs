use crate::common::{expr_node, simple_column_name, validate_output_names};
use async_trait::async_trait;
use avenger_chart_core::{
    AvengerChartError, CompiledDataTransform, DataTransform, DataTransformCompileContext,
    DataTransformExecutionContext, DataTransformResult, DefaultLogicalExprNodeExt, IntoExpr,
    SerializableExpr,
};
use datafusion::{
    dataframe::DataFrame,
    functions_aggregate::expr_fn::{avg, count, max, min, sum},
    logical_expr::{Expr, col, lit},
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AggregateOp {
    Sum,
    Count,
    Mean,
    Min,
    Max,
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
#[async_trait]
impl CompiledDataTransform for CompiledAggregateTransform {
    fn clone_box(&self) -> Box<dyn CompiledDataTransform> {
        Box::new(self.clone())
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
                    if let Some(alias) = &group.alias {
                        if !group_alias_is_identity(group, alias, ctx.session_context)? {
                            names.push(alias.as_str());
                        }
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
    };
    Ok(expr.alias(&measure.name))
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
