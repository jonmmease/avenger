use crate::common::{expr_node, map_expr_node, validate_generated_name};
use async_trait::async_trait;
use avenger_chart_core::{
    AvengerChartError, CompiledDataTransform, DataTransform, DataTransformCompileContext,
    DataTransformExecutionContext, DataTransformResult, DefaultLogicalExprNodeExt, ExecutionShape,
    IntoExpr, SerializableExpr,
};
use datafusion::{dataframe::DataFrame, logical_expr::Expr};
use datafusion_proto::protobuf::LogicalExprNode;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledCalculateTransform {
    pub exprs: Vec<CalculateExprSpec>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub simultaneous: bool,
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalculateExprSpec {
    pub name: String,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub expr: LogicalExprNode,
}

#[derive(Clone, Debug, Default)]
pub struct Calculate {
    exprs: IndexMap<String, Expr>,
    simultaneous: bool,
}

impl Calculate {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn expr(mut self, name: impl Into<String>, expr: impl IntoExpr) -> Self {
        self.exprs.insert(name.into(), expr.into_expr());
        self
    }

    /// Evaluate every expression against the same input relation.
    ///
    /// The default remains sequential for compatibility with compound-mark
    /// pipelines that intentionally reference columns created earlier in the
    /// same transform. Projection-list language lowering opts into this mode.
    pub fn simultaneous(mut self) -> Self {
        self.simultaneous = true;
        self
    }
}

impl DataTransform for Calculate {
    type Output = ();

    fn into_compiled_and_output(
        self,
        _ctx: DataTransformCompileContext,
    ) -> Result<(Box<dyn CompiledDataTransform>, Self::Output), AvengerChartError> {
        for name in self.exprs.keys() {
            validate_generated_name(name)?;
        }
        let exprs = self
            .exprs
            .into_iter()
            .map(|(name, expr)| CalculateExprSpec {
                name,
                expr: expr_node(expr, "calculate expression"),
            })
            .collect();
        Ok((
            Box::new(CompiledCalculateTransform {
                exprs,
                simultaneous: self.simultaneous,
            }),
            (),
        ))
    }
}

#[typetag::serde(name = "calculate")]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl CompiledDataTransform for CompiledCalculateTransform {
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
            exprs: self
                .exprs
                .iter()
                .map(|spec| {
                    Ok(CalculateExprSpec {
                        name: spec.name.clone(),
                        expr: map_expr_node(&spec.expr, f)?,
                    })
                })
                .collect::<Result<_, AvengerChartError>>()?,
            simultaneous: self.simultaneous,
        }))
    }

    async fn apply(
        &self,
        dataframe: DataFrame,
        ctx: &DataTransformExecutionContext<'_>,
    ) -> Result<DataTransformResult, AvengerChartError> {
        if !self.simultaneous {
            let mut dataframe = dataframe;
            for spec in &self.exprs {
                validate_generated_name(&spec.name)?;
                dataframe = dataframe
                    .with_column(&spec.name, spec.expr.to_default_expr(ctx.session_context)?)
                    .map_err(AvengerChartError::DataFusionError)?;
            }
            return Ok(DataTransformResult::dataframe(dataframe));
        }

        let expressions = self
            .exprs
            .iter()
            .map(|spec| {
                validate_generated_name(&spec.name)?;
                Ok((
                    spec.name.as_str(),
                    spec.expr.to_default_expr(ctx.session_context)?,
                ))
            })
            .collect::<Result<Vec<_>, AvengerChartError>>()?;
        let mut projection = dataframe
            .schema()
            .iter()
            .map(|(qualifier, field)| {
                expressions
                    .iter()
                    .find(|(name, _)| *name == field.name())
                    .map_or_else(
                        || Expr::Column(datafusion::common::Column::from((qualifier, field))),
                        |(name, expression)| expression.clone().alias(*name),
                    )
            })
            .collect::<Vec<_>>();
        for (name, expression) in expressions {
            if !dataframe
                .schema()
                .fields()
                .iter()
                .any(|field| field.name() == name)
            {
                projection.push(expression.alias(name));
            }
        }
        let dataframe = dataframe
            .select(projection)
            .map_err(AvengerChartError::DataFusionError)?;
        Ok(DataTransformResult::dataframe(dataframe))
    }
}

fn is_false(value: &bool) -> bool {
    !value
}

#[cfg(test)]
mod tests {
    use avenger_chart_core::{
        CoordinationScope, DataTransform, DataTransformCompileContext,
        DataTransformExecutionContext, TimeContext,
    };
    use datafusion::{
        arrow::array::Int64Array,
        prelude::{SessionContext, col, lit},
    };
    use indexmap::IndexMap;

    use super::Calculate;

    #[tokio::test]
    async fn simultaneous_sibling_calculations_read_the_original_input_relation() {
        let session = SessionContext::new();
        let dataframe = session.sql("SELECT 3 AS x").await.unwrap();
        let (compiled, ()) = Calculate::new()
            .simultaneous()
            .expr("x", col("x") + lit(1_i64))
            .expr("y", col("x") * lit(2_i64))
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free))
            .unwrap();
        let params = IndexMap::new();
        let result = compiled
            .apply(
                dataframe,
                &DataTransformExecutionContext {
                    session_context: &session,
                    params: &params,
                    time_context: TimeContext::default(),
                    facet_context: None,
                },
            )
            .await
            .unwrap();
        let batches = result.dataframe.collect().await.unwrap();
        let batch = &batches[0];
        assert_eq!(
            batch
                .column_by_name("x")
                .unwrap()
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .value(0),
            4
        );
        assert_eq!(
            batch
                .column_by_name("y")
                .unwrap()
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .value(0),
            6
        );
    }

    #[tokio::test]
    async fn default_calculations_retain_sequential_rust_builder_semantics() {
        let session = SessionContext::new();
        let dataframe = session.sql("SELECT 3 AS x").await.unwrap();
        let (compiled, ()) = Calculate::new()
            .expr("x", col("x") + lit(1_i64))
            .expr("y", col("x") * lit(2_i64))
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free))
            .unwrap();
        let params = IndexMap::new();
        let result = compiled
            .apply(
                dataframe,
                &DataTransformExecutionContext {
                    session_context: &session,
                    params: &params,
                    time_context: TimeContext::default(),
                    facet_context: None,
                },
            )
            .await
            .unwrap();
        let batches = result.dataframe.collect().await.unwrap();
        let batch = &batches[0];
        assert_eq!(
            batch
                .column_by_name("y")
                .unwrap()
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .value(0),
            8
        );
    }
}
