use crate::common::{expr_node, sanitize_output_name, validate_output_names};
use async_trait::async_trait;
use avenger_chart_core::{
    AvengerChartError, ChannelExpr, CompiledDataTransform, DataTransform,
    DataTransformCompileContext, DataTransformExecutionContext, DataTransformResult,
    DefaultLogicalExprNodeExt, IntoExpr, SerializableExpr,
};
use datafusion::{
    common::ScalarValue,
    dataframe::DataFrame,
    logical_expr::{
        Expr, ExprSchemable, WindowFrame, WindowFrameBound, WindowFrameUnits,
        WindowFunctionDefinition, col,
        expr::{Sort, WindowFunction},
        lit, when,
    },
};
use datafusion_functions_aggregate::{min_max::max_udaf, sum::sum_udaf};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledStackTransform {
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub value: LogicalExprNode,
    #[serde_as(as = "Vec<FromInto<SerializableExpr>>")]
    pub group_by: Vec<LogicalExprNode>,
    pub sort_by: Vec<TransformSortSpec>,
    pub offset: StackOffset,
    pub start_name: String,
    pub end_name: String,
    pub value_name: Option<String>,
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransformSortSpec {
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub expr: LogicalExprNode,
    pub ascending: bool,
    pub nulls_first: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StackOffset {
    Zero,
    Center,
    Normalize,
}

#[derive(Clone, Debug)]
pub struct Stack {
    value: Expr,
    group_by: Vec<Expr>,
    sort_by: Vec<Sort>,
    offset: StackOffset,
    name: Option<String>,
    value_name: Option<String>,
}

impl Stack {
    pub fn new(value: impl IntoExpr) -> Self {
        Self {
            value: value.into_expr(),
            group_by: Vec::new(),
            sort_by: Vec::new(),
            offset: StackOffset::Zero,
            name: None,
            value_name: None,
        }
    }

    pub fn group_by<I>(mut self, exprs: I) -> Self
    where
        I: IntoIterator<Item = Expr>,
    {
        self.group_by.extend(exprs);
        self
    }

    pub fn sort_by<I>(mut self, sort_by: I) -> Self
    where
        I: IntoIterator<Item = Sort>,
    {
        self.sort_by.extend(sort_by);
        self
    }

    pub fn sort_by_exprs<I>(mut self, exprs: I) -> Self
    where
        I: IntoIterator<Item = Expr>,
    {
        self.sort_by
            .extend(exprs.into_iter().map(|expr| expr.sort(true, false)));
        self
    }

    pub fn offset(mut self, offset: StackOffset) -> Self {
        self.offset = offset;
        self
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn value_name(mut self, name: impl Into<String>) -> Self {
        self.value_name = Some(name.into());
        self
    }
}

impl DataTransform for Stack {
    type Output = StackOutput;

    fn into_compiled_and_output(
        self,
        _ctx: DataTransformCompileContext,
    ) -> Result<(Box<dyn CompiledDataTransform>, Self::Output), AvengerChartError> {
        if avenger_chart_core::contains_aggregate(&self.value) {
            return Err(AvengerChartError::InvalidArgument(
                "Stack::new(...) does not accept aggregate expressions; use Aggregate before Stack"
                    .to_string(),
            ));
        }
        let base_name = self
            .name
            .unwrap_or_else(|| sanitize_output_name(&format!("{}_stack", self.value)));
        let start_name = format!("{base_name}_start");
        let end_name = format!("{base_name}_end");
        let value_name = self.value_name;
        let transform = CompiledStackTransform {
            value: expr_node(self.value, "stack value expression"),
            group_by: self
                .group_by
                .into_iter()
                .map(|expr| expr_node(expr, "stack group_by expression"))
                .collect(),
            sort_by: self
                .sort_by
                .into_iter()
                .map(|sort| TransformSortSpec {
                    expr: expr_node(sort.expr, "stack sort expression"),
                    ascending: sort.asc,
                    nulls_first: sort.nulls_first,
                })
                .collect(),
            offset: self.offset,
            start_name: start_name.clone(),
            end_name: end_name.clone(),
            value_name: value_name.clone(),
        };
        Ok((
            Box::new(transform),
            StackOutput {
                start_name,
                end_name,
                value_name,
            },
        ))
    }
}

#[derive(Clone, Debug)]
pub struct StackOutput {
    start_name: String,
    end_name: String,
    value_name: Option<String>,
}

impl StackOutput {
    pub fn start(&self) -> ChannelExpr {
        ChannelExpr::scaled(col(&self.start_name))
    }

    pub fn end(&self) -> ChannelExpr {
        ChannelExpr::scaled(col(&self.end_name))
    }

    pub fn mid(&self) -> ChannelExpr {
        ChannelExpr::scaled((col(&self.start_name) + col(&self.end_name)) / lit(2.0))
    }

    pub fn value(&self) -> Expr {
        col(self.value_name.as_deref().unwrap_or(&self.end_name))
    }
}

#[typetag::serde(name = "stack")]
#[async_trait]
impl CompiledDataTransform for CompiledStackTransform {
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
            [
                self.start_name.as_str(),
                self.end_name.as_str(),
                self.value_name.as_deref().unwrap_or("__unused_stack_value"),
            ],
        )?;
        let dataframe = apply_stack(dataframe, self, ctx.session_context)?;
        Ok(DataTransformResult::dataframe(dataframe))
    }
}

fn aggregate_window_expr(
    fun: WindowFunctionDefinition,
    arg: Expr,
    partition_by: Vec<Expr>,
    order_by: Vec<Sort>,
    window_frame: WindowFrame,
) -> Expr {
    let mut window = WindowFunction::new(fun, vec![arg]);
    window.params.partition_by = partition_by;
    window.params.order_by = order_by;
    window.params.window_frame = window_frame;
    Expr::from(window)
}

fn window_sum(
    arg: Expr,
    partition_by: Vec<Expr>,
    order_by: Vec<Sort>,
    window_frame: WindowFrame,
) -> Expr {
    aggregate_window_expr(
        WindowFunctionDefinition::AggregateUDF(sum_udaf()),
        arg,
        partition_by,
        order_by,
        window_frame,
    )
}

fn window_max(
    arg: Expr,
    partition_by: Vec<Expr>,
    order_by: Vec<Sort>,
    window_frame: WindowFrame,
) -> Expr {
    aggregate_window_expr(
        WindowFunctionDefinition::AggregateUDF(max_udaf()),
        arg,
        partition_by,
        order_by,
        window_frame,
    )
}

fn apply_stack(
    dataframe: DataFrame,
    payload: &CompiledStackTransform,
    ctx: &datafusion::prelude::SessionContext,
) -> Result<DataFrame, AvengerChartError> {
    let original_columns = dataframe
        .schema()
        .fields()
        .iter()
        .map(|field| field.name().clone())
        .collect::<Vec<_>>();
    let value_expr = payload.value.to_default_expr(ctx)?;
    let group_by = payload
        .group_by
        .iter()
        .map(|expr| expr.to_default_expr(ctx))
        .collect::<Result<Vec<_>, _>>()?;
    let sort_by = payload
        .sort_by
        .iter()
        .map(|sort| {
            Ok(Sort::new(
                sort.expr.to_default_expr(ctx)?,
                sort.ascending,
                sort.nulls_first,
            ))
        })
        .collect::<Result<Vec<_>, AvengerChartError>>()?;

    let value_expr = value_expr.cast_to(
        &datafusion::arrow::datatypes::DataType::Float64,
        dataframe.schema(),
    )?;
    let mut df = dataframe
        .with_column("__avenger_stack_value", value_expr)?
        .with_column(
            "__avenger_stack_abs_value",
            when(
                col("__avenger_stack_value").lt(lit(0.0)),
                lit(0.0) - col("__avenger_stack_value"),
            )
            .otherwise(col("__avenger_stack_value"))?,
        )?;

    let order_by = sort_by;
    let cumulative_frame = WindowFrame::new_bounds(
        WindowFrameUnits::Rows,
        WindowFrameBound::Preceding(ScalarValue::UInt64(None)),
        WindowFrameBound::CurrentRow,
    );
    let whole_partition_frame = WindowFrame::new_bounds(
        WindowFrameUnits::Rows,
        WindowFrameBound::Preceding(ScalarValue::UInt64(None)),
        WindowFrameBound::Following(ScalarValue::UInt64(None)),
    );

    match payload.offset {
        StackOffset::Zero => {
            df = df
                .with_column(
                    "__avenger_stack_pos_value",
                    when(col("__avenger_stack_value").lt(lit(0.0)), lit(0.0))
                        .otherwise(col("__avenger_stack_value"))?,
                )?
                .with_column(
                    "__avenger_stack_neg_value",
                    when(
                        col("__avenger_stack_value").lt(lit(0.0)),
                        col("__avenger_stack_value"),
                    )
                    .otherwise(lit(0.0))?,
                )?
                .with_column(
                    "__avenger_stack_pos_cum",
                    window_sum(
                        col("__avenger_stack_pos_value"),
                        group_by.clone(),
                        order_by.clone(),
                        cumulative_frame.clone(),
                    ),
                )?
                .with_column(
                    "__avenger_stack_pos_group_sum",
                    window_sum(
                        col("__avenger_stack_pos_value"),
                        group_by.clone(),
                        Vec::new(),
                        whole_partition_frame.clone(),
                    ),
                )?
                .with_column(
                    "__avenger_stack_neg_cum",
                    window_sum(
                        col("__avenger_stack_neg_value"),
                        group_by.clone(),
                        order_by.clone(),
                        cumulative_frame.clone(),
                    ),
                )?
                .with_column(
                    &payload.start_name,
                    when(
                        col("__avenger_stack_value").lt(lit(0.0)),
                        col("__avenger_stack_neg_cum") - col("__avenger_stack_value"),
                    )
                    .otherwise(
                        col("__avenger_stack_pos_group_sum") - col("__avenger_stack_pos_cum"),
                    )?,
                )?
                .with_column(
                    &payload.end_name,
                    when(
                        col("__avenger_stack_value").lt(lit(0.0)),
                        col("__avenger_stack_neg_cum"),
                    )
                    .otherwise(
                        col("__avenger_stack_pos_group_sum") - col("__avenger_stack_pos_cum")
                            + col("__avenger_stack_pos_value"),
                    )?,
                )?;
        }
        StackOffset::Normalize | StackOffset::Center => {
            df = df
                .with_column(
                    "__avenger_stack_abs_cum",
                    window_sum(
                        col("__avenger_stack_abs_value"),
                        group_by.clone(),
                        order_by.clone(),
                        cumulative_frame.clone(),
                    ),
                )?
                .with_column(
                    "__avenger_stack_group_sum",
                    window_sum(
                        col("__avenger_stack_abs_value"),
                        group_by.clone(),
                        Vec::new(),
                        whole_partition_frame.clone(),
                    ),
                )?;
            match payload.offset {
                StackOffset::Normalize => {
                    df = df
                        .with_column(
                            &payload.start_name,
                            (col("__avenger_stack_group_sum") - col("__avenger_stack_abs_cum"))
                                / col("__avenger_stack_group_sum"),
                        )?
                        .with_column(
                            &payload.end_name,
                            (col("__avenger_stack_group_sum") - col("__avenger_stack_abs_cum")
                                + col("__avenger_stack_abs_value"))
                                / col("__avenger_stack_group_sum"),
                        )?;
                }
                StackOffset::Center => {
                    df = df
                        .with_column(
                            "__avenger_stack_max_sum",
                            window_max(
                                col("__avenger_stack_group_sum"),
                                Vec::new(),
                                Vec::new(),
                                whole_partition_frame,
                            ),
                        )?
                        .with_column(
                            "__avenger_stack_base",
                            (col("__avenger_stack_max_sum") - col("__avenger_stack_group_sum"))
                                / lit(2.0),
                        )?
                        .with_column(
                            &payload.start_name,
                            col("__avenger_stack_base") + col("__avenger_stack_group_sum")
                                - col("__avenger_stack_abs_cum"),
                        )?
                        .with_column(
                            &payload.end_name,
                            col("__avenger_stack_base") + col("__avenger_stack_group_sum")
                                - col("__avenger_stack_abs_cum")
                                + col("__avenger_stack_abs_value"),
                        )?;
                }
                StackOffset::Zero => unreachable!(),
            }
        }
    }

    if let Some(value_name) = &payload.value_name {
        df = df.with_column(value_name, col("__avenger_stack_value"))?;
    }

    let mut projection = original_columns
        .iter()
        .map(|name| col(name))
        .collect::<Vec<_>>();
    projection.push(col(&payload.start_name));
    projection.push(col(&payload.end_name));
    if let Some(value_name) = &payload.value_name {
        projection.push(col(value_name));
    }
    df.select(projection)
        .map_err(AvengerChartError::DataFusionError)
}
