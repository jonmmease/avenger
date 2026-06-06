use crate::common::{expr_node, map_expr_node};
use async_trait::async_trait;
use avenger_chart_core::{
    AvengerChartError, CompiledDataTransform, DataTransform, DataTransformCompileContext,
    DataTransformExecutionContext, DataTransformResult, DefaultLogicalExprNodeExt, IntoExpr,
    SerializableExpr,
};
use datafusion::{
    arrow::datatypes::DataType,
    dataframe::DataFrame,
    logical_expr::{Expr, cast},
};
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
    pub fn new(predicate: impl IntoExpr) -> Self {
        Self {
            predicate: predicate.into_expr(),
        }
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

    fn map_exprs(
        &self,
        f: &mut dyn FnMut(Expr) -> Result<Expr, AvengerChartError>,
    ) -> Result<Box<dyn CompiledDataTransform>, AvengerChartError> {
        Ok(Box::new(Self {
            predicate: map_expr_node(&self.predicate, f)?,
        }))
    }

    async fn apply(
        &self,
        dataframe: DataFrame,
        ctx: &DataTransformExecutionContext<'_>,
    ) -> Result<DataTransformResult, AvengerChartError> {
        let predicate = cast(
            self.predicate.to_default_expr(ctx.session_context)?,
            DataType::Boolean,
        );
        Ok(DataTransformResult::dataframe(
            dataframe
                .filter(predicate)
                .map_err(AvengerChartError::DataFusionError)?,
        ))
    }
}
