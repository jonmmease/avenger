use crate::common::{
    expr_node, map_expr_node, map_expr_nodes, validate_output_names,
    validate_unique_generated_names,
};
use async_trait::async_trait;
use avenger_chart_core::{
    AvengerChartError, CompiledDataTransform, DataTransform, DataTransformCompileContext,
    DataTransformExecutionContext, DataTransformResult, DefaultLogicalExprNodeExt, IntoExpr,
    SerializableExpr,
};
use datafusion::{
    dataframe::DataFrame,
    logical_expr::{
        Expr, WindowFrame, WindowFunctionDefinition,
        expr::{Sort, WindowFunction},
    },
};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledWindowTransform {
    #[serde_as(as = "Vec<FromInto<SerializableExpr>>")]
    pub partition_by: Vec<LogicalExprNode>,
    pub order_by: Vec<WindowSortSpec>,
    pub exprs: Vec<WindowExprSpec>,
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowSortSpec {
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub expr: LogicalExprNode,
    pub ascending: bool,
    pub nulls_first: bool,
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowExprSpec {
    pub name: String,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub expr: LogicalExprNode,
}

#[derive(Clone, Debug, Default)]
pub struct Window {
    partition_by: Vec<Expr>,
    order_by: Vec<Sort>,
    exprs: Vec<(String, Expr)>,
}

impl Window {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn partition_by<I, E>(mut self, exprs: I) -> Self
    where
        I: IntoIterator<Item = E>,
        E: IntoExpr,
    {
        self.partition_by
            .extend(exprs.into_iter().map(IntoExpr::into_expr));
        self
    }

    pub fn order_by<I>(mut self, sort_by: I) -> Self
    where
        I: IntoIterator<Item = Sort>,
    {
        self.order_by.extend(sort_by);
        self
    }

    pub fn expr(mut self, name: impl Into<String>, expr: impl IntoExpr) -> Self {
        let name = name.into();
        let expr = expr.into_expr();
        if let Some((_, existing)) = self
            .exprs
            .iter_mut()
            .find(|(existing_name, _)| existing_name == &name)
        {
            *existing = expr;
        } else {
            self.exprs.push((name, expr));
        }
        self
    }
}

impl DataTransform for Window {
    type Output = ();

    fn into_compiled_and_output(
        self,
        _ctx: DataTransformCompileContext,
    ) -> Result<(Box<dyn CompiledDataTransform>, Self::Output), AvengerChartError> {
        validate_unique_generated_names(self.exprs.iter().map(|(name, _)| name.as_str()))?;
        let transform = CompiledWindowTransform {
            partition_by: self
                .partition_by
                .into_iter()
                .map(|expr| expr_node(expr, "window partition_by expression"))
                .collect(),
            order_by: self
                .order_by
                .into_iter()
                .map(|sort| WindowSortSpec {
                    expr: expr_node(sort.expr, "window order_by expression"),
                    ascending: sort.asc,
                    nulls_first: sort.nulls_first,
                })
                .collect(),
            exprs: self
                .exprs
                .into_iter()
                .map(|(name, expr)| WindowExprSpec {
                    name,
                    expr: expr_node(expr, "window expression"),
                })
                .collect(),
        };
        Ok((Box::new(transform), ()))
    }
}

#[typetag::serde(name = "window")]
#[async_trait]
impl CompiledDataTransform for CompiledWindowTransform {
    fn clone_box(&self) -> Box<dyn CompiledDataTransform> {
        Box::new(self.clone())
    }

    fn map_exprs(
        &self,
        f: &mut dyn FnMut(Expr) -> Result<Expr, AvengerChartError>,
    ) -> Result<Box<dyn CompiledDataTransform>, AvengerChartError> {
        Ok(Box::new(Self {
            partition_by: map_expr_nodes(&self.partition_by, f)?,
            order_by: self
                .order_by
                .iter()
                .map(|sort| {
                    Ok(WindowSortSpec {
                        expr: map_expr_node(&sort.expr, f)?,
                        ascending: sort.ascending,
                        nulls_first: sort.nulls_first,
                    })
                })
                .collect::<Result<_, AvengerChartError>>()?,
            exprs: self
                .exprs
                .iter()
                .map(|spec| {
                    Ok(WindowExprSpec {
                        name: spec.name.clone(),
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
        validate_output_names(
            dataframe.schema().fields().iter().map(|field| field.name()),
            self.exprs.iter().map(|spec| spec.name.as_str()),
        )?;

        let partition_by = self
            .partition_by
            .iter()
            .map(|expr| expr.to_default_expr(ctx.session_context))
            .collect::<Result<Vec<_>, _>>()?;
        let order_by = self
            .order_by
            .iter()
            .map(|sort| {
                Ok(Sort {
                    expr: sort.expr.to_default_expr(ctx.session_context)?,
                    asc: sort.ascending,
                    nulls_first: sort.nulls_first,
                })
            })
            .collect::<Result<Vec<_>, AvengerChartError>>()?;

        let mut window_exprs = Vec::with_capacity(self.exprs.len());
        for spec in &self.exprs {
            let expr = spec.expr.to_default_expr(ctx.session_context)?;
            let expr = configure_window_expr(expr, &spec.name, &partition_by, &order_by)?;
            window_exprs.push(expr);
        }

        Ok(DataTransformResult::dataframe(
            dataframe
                .window(window_exprs)
                .map_err(AvengerChartError::DataFusionError)?,
        ))
    }
}

fn configure_window_expr(
    expr: Expr,
    name: &str,
    default_partition_by: &[Expr],
    default_order_by: &[Sort],
) -> Result<Expr, AvengerChartError> {
    let Expr::WindowFunction(window) = expr else {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Window transform output '{name}' must be a DataFusion window expression"
        )));
    };

    let mut window: WindowFunction = *window;
    if window.params.partition_by.is_empty() {
        window.params.partition_by = default_partition_by.to_vec();
    }
    if window.params.order_by.is_empty() {
        window.params.order_by = default_order_by.to_vec();
        if !window.params.order_by.is_empty()
            && window.params.window_frame == WindowFrame::new(None)
        {
            window.params.window_frame = WindowFrame::new(Some(true));
        }
    }
    if window.params.order_by.is_empty() {
        if !is_order_independent_aggregate_window(&window) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Window transform output '{name}' requires an explicit order_by(...) for deterministic results"
            )));
        }
    }

    Ok(Expr::from(window).alias(name))
}

fn is_order_independent_aggregate_window(window: &WindowFunction) -> bool {
    matches!(window.fun, WindowFunctionDefinition::AggregateUDF(_))
        && window.params.window_frame == WindowFrame::new(None)
}
