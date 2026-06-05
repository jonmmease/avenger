use crate::common::{expr_node, validate_generated_name};
use async_trait::async_trait;
use avenger_chart_core::{
    AvengerChartError, CompiledDataTransform, DataTransform, DataTransformCompileContext,
    DataTransformExecutionContext, DataTransformResult, DefaultLogicalExprNodeExt,
    SerializableExpr,
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
}

impl Calculate {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn expr(mut self, name: impl Into<String>, expr: Expr) -> Self {
        self.exprs.insert(name.into(), expr);
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
        Ok((Box::new(CompiledCalculateTransform { exprs }), ()))
    }
}

#[typetag::serde(name = "calculate")]
#[async_trait]
impl CompiledDataTransform for CompiledCalculateTransform {
    fn clone_box(&self) -> Box<dyn CompiledDataTransform> {
        Box::new(self.clone())
    }

    async fn apply(
        &self,
        mut dataframe: DataFrame,
        ctx: &DataTransformExecutionContext<'_>,
    ) -> Result<DataTransformResult, AvengerChartError> {
        for spec in &self.exprs {
            validate_generated_name(&spec.name)?;
            dataframe = dataframe
                .with_column(&spec.name, spec.expr.to_default_expr(ctx.session_context)?)
                .map_err(AvengerChartError::DataFusionError)?;
        }
        Ok(DataTransformResult::dataframe(dataframe))
    }
}
