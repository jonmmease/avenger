use crate::common::expr_node;
use async_trait::async_trait;
use avenger_chart_core::{
    AvengerChartError, CompiledDataTransform, DataTransform, DataTransformCompileContext,
    DataTransformExecutionContext, DataTransformResult, DefaultLogicalExprNodeExt,
    SerializableExpr,
};
use datafusion::{dataframe::DataFrame, logical_expr::Expr};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledFilterTransform {
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub predicate: LogicalExprNode,
}

#[derive(Clone, Debug)]
pub struct Filter {
    predicate: Expr,
}

impl Filter {
    pub fn new(predicate: Expr) -> Self {
        Self { predicate }
    }
}

impl DataTransform for Filter {
    type Output = ();

    fn into_compiled_and_output(
        self,
        _ctx: DataTransformCompileContext,
    ) -> Result<(Box<dyn CompiledDataTransform>, Self::Output), AvengerChartError> {
        Ok((
            Box::new(CompiledFilterTransform {
                predicate: expr_node(self.predicate, "filter predicate expression"),
            }),
            (),
        ))
    }
}

#[typetag::serde(name = "filter")]
#[async_trait]
impl CompiledDataTransform for CompiledFilterTransform {
    fn clone_box(&self) -> Box<dyn CompiledDataTransform> {
        Box::new(self.clone())
    }

    async fn apply(
        &self,
        dataframe: DataFrame,
        ctx: &DataTransformExecutionContext<'_>,
    ) -> Result<DataTransformResult, AvengerChartError> {
        Ok(DataTransformResult::dataframe(
            dataframe
                .filter(self.predicate.to_default_expr(ctx.session_context)?)
                .map_err(AvengerChartError::DataFusionError)?,
        ))
    }
}
