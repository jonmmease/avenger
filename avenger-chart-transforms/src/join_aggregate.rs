use crate::aggregate::{AggregateGroupKeySpec, AggregateMeasureSpec, AggregateOp, aggregate_expr};
use crate::common::{expr_node, simple_column_name, validate_output_names};
use async_trait::async_trait;
use avenger_chart_core::{
    AvengerChartError, CompiledDataTransform, DataTransform, DataTransformCompileContext,
    DataTransformExecutionContext, DataTransformResult, DefaultLogicalExprNodeExt,
};
use datafusion::dataframe::DataFrame;
use datafusion::logical_expr::{Expr, JoinType, Operator, binary_expr, col, lit};
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

    pub fn group_by<I>(mut self, exprs: I) -> Self
    where
        I: IntoIterator<Item = Expr>,
    {
        self.group_by.extend(exprs.into_iter().map(|expr| {
            let alias = simple_column_name(&expr);
            AggregateGroupKeySpec {
                expr: expr_node(expr, "joinaggregate group_by expression"),
                alias,
            }
        }));
        self
    }

    pub fn group_by_as(mut self, alias: impl Into<String>, expr: Expr) -> Self {
        self.group_by.push(AggregateGroupKeySpec {
            expr: expr_node(expr, "joinaggregate group_by expression"),
            alias: Some(alias.into()),
        });
        self
    }

    pub fn sum(self, name: impl Into<String>, expr: Expr) -> Self {
        self.measure(name, AggregateOp::Sum, Some(expr))
    }

    pub fn count(self, name: impl Into<String>) -> Self {
        self.measure(name, AggregateOp::Count, None)
    }

    pub fn mean(self, name: impl Into<String>, expr: Expr) -> Self {
        self.measure(name, AggregateOp::Mean, Some(expr))
    }

    pub fn min(self, name: impl Into<String>, expr: Expr) -> Self {
        self.measure(name, AggregateOp::Min, Some(expr))
    }

    pub fn max(self, name: impl Into<String>, expr: Expr) -> Self {
        self.measure(name, AggregateOp::Max, Some(expr))
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
#[async_trait]
impl CompiledDataTransform for CompiledJoinAggregateTransform {
    fn clone_box(&self) -> Box<dyn CompiledDataTransform> {
        Box::new(self.clone())
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

        let result = if self.group_by.is_empty() {
            let aggregate = dataframe
                .clone()
                .aggregate(Vec::<Expr>::new(), aggregate_exprs(&self.measures, ctx)?)
                .map_err(AvengerChartError::DataFusionError)?;
            dataframe
                .join_on(aggregate, JoinType::Left, [lit(true)])
                .map_err(AvengerChartError::DataFusionError)?
        } else {
            let left_key_names = hidden_key_names(&dataframe, self.group_by.len(), "left");
            let right_key_names = hidden_key_names(&dataframe, self.group_by.len(), "right");
            let mut keyed = dataframe;
            for (group, key_name) in self.group_by.iter().zip(left_key_names.iter()) {
                keyed = keyed
                    .with_column(key_name, group.expr.to_default_expr(ctx.session_context)?)
                    .map_err(AvengerChartError::DataFusionError)?;
            }
            let aggregate = keyed
                .clone()
                .aggregate(
                    left_key_names
                        .iter()
                        .zip(right_key_names.iter())
                        .map(|(left, right)| col(left).alias(right))
                        .collect::<Vec<_>>(),
                    aggregate_exprs(&self.measures, ctx)?,
                )
                .map_err(AvengerChartError::DataFusionError)?;
            let predicates = left_key_names
                .iter()
                .zip(right_key_names.iter())
                .map(|(left, right)| {
                    binary_expr(col(left), Operator::IsNotDistinctFrom, col(right))
                })
                .collect::<Vec<_>>();
            keyed
                .join_on(aggregate, JoinType::Left, predicates)
                .map_err(AvengerChartError::DataFusionError)?
        };

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

fn aggregate_exprs(
    measures: &[AggregateMeasureSpec],
    ctx: &DataTransformExecutionContext<'_>,
) -> Result<Vec<Expr>, AvengerChartError> {
    measures
        .iter()
        .map(|measure| aggregate_expr(measure, ctx.session_context))
        .collect()
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

fn hidden_key_names(dataframe: &DataFrame, len: usize, side: &str) -> Vec<String> {
    let existing = dataframe
        .schema()
        .fields()
        .iter()
        .map(|field| field.name().clone())
        .collect::<IndexSet<_>>();
    (0..len)
        .map(|index| {
            let base = format!("__avenger_join_aggregate_{side}_key_{index}");
            if !existing.contains(&base) {
                return base;
            }
            let mut suffix = 1;
            loop {
                let candidate = format!("{base}_{suffix}");
                if !existing.contains(&candidate) {
                    break candidate;
                }
                suffix += 1;
            }
        })
        .collect()
}
