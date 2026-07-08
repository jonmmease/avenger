use crate::common::{
    expr_node, map_expr_node, select_output_name, validate_generated_name,
    validate_unique_generated_names,
};
use async_trait::async_trait;
use avenger_chart_core::{
    AvengerChartError, CompiledDataTransform, DataTransform, DataTransformCompileContext,
    DataTransformExecutionContext, DataTransformResult, DefaultLogicalExprNodeExt, ExecutionShape,
    IntoExpr, SerializableExpr,
};
use datafusion::{dataframe::DataFrame, logical_expr::Expr};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledSelectTransform {
    pub exprs: Vec<SelectExprSpec>,
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelectExprSpec {
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub expr: LogicalExprNode,
}

#[derive(Clone, Debug, Default)]
pub struct Select {
    exprs: Vec<Expr>,
}

impl Select {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn expr(mut self, expr: impl IntoExpr) -> Self {
        self.exprs.push(expr.into_expr());
        self
    }
}

impl DataTransform for Select {
    type Output = ();

    fn into_compiled_and_output(
        self,
        _ctx: DataTransformCompileContext,
    ) -> Result<(Box<dyn CompiledDataTransform>, Self::Output), AvengerChartError> {
        validate_select_exprs(&self.exprs)?;
        let exprs = self
            .exprs
            .into_iter()
            .map(|expr| SelectExprSpec {
                expr: expr_node(expr, "select expression"),
            })
            .collect();
        Ok((Box::new(CompiledSelectTransform { exprs }), ()))
    }
}

#[typetag::serde(name = "select")]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl CompiledDataTransform for CompiledSelectTransform {
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
                    Ok(SelectExprSpec {
                        expr: map_expr_node(&spec.expr, f)?,
                    })
                })
                .collect::<Result<_, AvengerChartError>>()?,
        }))
    }

    async fn apply(
        &self,
        dataframe: DataFrame,
        ctx: &DataTransformExecutionContext<'_>,
    ) -> Result<DataTransformResult, AvengerChartError> {
        let exprs = self
            .exprs
            .iter()
            .map(|spec| spec.expr.to_default_expr(ctx.session_context))
            .collect::<Result<Vec<_>, _>>()?;
        validate_select_exprs(&exprs)?;
        Ok(DataTransformResult::dataframe(
            dataframe
                .select(exprs)
                .map_err(AvengerChartError::DataFusionError)?,
        ))
    }
}

fn validate_select_exprs(exprs: &[Expr]) -> Result<(), AvengerChartError> {
    let mut names = Vec::with_capacity(exprs.len());
    for expr in exprs {
        let Some(name) = select_output_name(expr) else {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Select transform expression '{expr}' must be a source column or have an explicit alias"
            )));
        };
        validate_generated_name(&name)?;
        names.push(name);
    }
    validate_unique_generated_names(names.iter().map(String::as_str))
}
