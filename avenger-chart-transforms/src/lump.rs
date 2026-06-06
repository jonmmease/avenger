use crate::common::{expr_node, map_expr_node, sanitize_output_name, validate_output_names};
use async_trait::async_trait;
use avenger_chart_core::{
    AvengerChartError, ChannelExpr, CompiledDataTransform, DataTransform,
    DataTransformCompileContext, DataTransformExecutionContext, DataTransformResult,
    DefaultLogicalExprNodeExt, IntoExpr, ScaleChannelValue, SerializableExpr, eval_to_scalars,
    params_to_datafusion,
};
use datafusion::{
    arrow::datatypes::DataType,
    common::{ScalarValue, tree_node::Transformed},
    dataframe::DataFrame,
    functions_aggregate::expr_fn::{count, min},
    functions_window::expr_fn::row_number,
    logical_expr::{
        Expr, ExprSchemable, JoinType, Operator, WindowFrame,
        expr::{Placeholder, WindowFunction},
        expr_fn::binary_expr,
        lit, when,
    },
    prelude::col,
};
use datafusion_common::tree_node::TreeNode;
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

const LUMP_VALUE_PLACEHOLDER_ID: &str = "$__avenger_lump_original_value";
const LUMP_MEASURE_PLACEHOLDER_ID: &str = "$__avenger_lump_measure_value";
const LUMP_WINDOW_PLACEHOLDER_ID: &str = "$__avenger_lump_window_value";

const LUMP_VALUE_INTERNAL: &str = "__avenger_lump_value";
const LUMP_ORIGINAL: &str = "__avenger_lump_original";
const LUMP_MEASURE: &str = "__avenger_lump_measure";
const LUMP_WINDOW: &str = "__avenger_lump_window";
const LUMP_KEEP: &str = "__avenger_lump_keep";

/// Placeholder for the computed window value inside `Lump::keep(...)`.
pub fn window_value() -> Expr {
    lump_placeholder(LUMP_WINDOW_PLACEHOLDER_ID, Some(DataType::Float64))
}

/// Placeholder for the original grouped value inside `Lump::keep(...)`.
pub fn original_value() -> Expr {
    lump_placeholder(LUMP_VALUE_PLACEHOLDER_ID, None)
}

/// Placeholder for the aggregate measure value inside `Lump::keep(...)`.
pub fn measure_value() -> Expr {
    lump_placeholder(LUMP_MEASURE_PLACEHOLDER_ID, Some(DataType::Float64))
}

fn lump_placeholder(id: &str, data_type: Option<DataType>) -> Expr {
    Expr::Placeholder(Placeholder {
        id: id.to_string(),
        data_type,
    })
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledLumpTransform {
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub value: LogicalExprNode,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub top_n: LogicalExprNode,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub order_by: LogicalExprNode,
    pub order_descending: bool,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub window: LogicalExprNode,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub keep: LogicalExprNode,
    pub other_mode: LumpOtherMode,
    pub value_name: String,
    pub rank_name: String,
    pub measure_name: String,
    pub is_other_name: String,
    pub order_name: String,
    pub order_is_other_name: String,
    pub order_rank_name: String,
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LumpOtherMode {
    DefaultStringOther,
    Value {
        #[serde_as(as = "FromInto<SerializableExpr>")]
        expr: LogicalExprNode,
    },
    Drop,
}

#[derive(Clone, Debug)]
pub struct Lump {
    value: Expr,
    order_by: Expr,
    order_descending: bool,
    window: Expr,
    keep: Expr,
    other_mode: LumpOtherModeAuthoring,
    name: Option<String>,
    top_n: Expr,
}

#[derive(Clone, Debug)]
enum LumpOtherModeAuthoring {
    DefaultStringOther,
    Value(Expr),
    Drop,
}

pub trait IntoTopNExpr {
    fn into_top_n_expr(self) -> Expr;
}

impl IntoTopNExpr for Expr {
    fn into_top_n_expr(self) -> Expr {
        self
    }
}

impl IntoTopNExpr for ChannelExpr {
    fn into_top_n_expr(self) -> Expr {
        self.into_expr()
    }
}

macro_rules! impl_into_top_n_expr_for_int {
    ($($ty:ty),* $(,)?) => {
        $(
            impl IntoTopNExpr for $ty {
                fn into_top_n_expr(self) -> Expr {
                    lit(self)
                }
            }
        )*
    };
}

impl_into_top_n_expr_for_int!(i8, i16, i32, i64, u8, u16, u32, u64);

impl IntoTopNExpr for usize {
    fn into_top_n_expr(self) -> Expr {
        lit(self as i64)
    }
}

impl Lump {
    pub fn top_n(value: impl IntoExpr, n: impl IntoTopNExpr) -> Self {
        let n = n.into_top_n_expr();
        let keep = window_value().lt_eq(n.clone());
        Self {
            value: value.into_expr(),
            order_by: count(lit(1)),
            order_descending: true,
            window: row_number(),
            keep,
            other_mode: LumpOtherModeAuthoring::DefaultStringOther,
            name: None,
            top_n: n,
        }
    }

    pub fn order_by(mut self, expr: impl IntoExpr) -> Self {
        self.order_by = expr.into_expr();
        self
    }

    pub fn order_asc(mut self) -> Self {
        self.order_descending = false;
        self
    }

    pub fn order_desc(mut self) -> Self {
        self.order_descending = true;
        self
    }

    pub fn window(mut self, expr: impl IntoExpr) -> Self {
        self.window = expr.into_expr();
        self
    }

    pub fn keep(mut self, expr: impl IntoExpr) -> Self {
        self.keep = expr.into_expr();
        self
    }

    pub fn other_label(mut self, label: impl Into<String>) -> Self {
        self.other_mode = LumpOtherModeAuthoring::Value(lit(label.into()));
        self
    }

    pub fn other_value(mut self, value: impl IntoExpr) -> Self {
        self.other_mode = LumpOtherModeAuthoring::Value(value.into_expr());
        self
    }

    pub fn drop_other(mut self) -> Self {
        self.other_mode = LumpOtherModeAuthoring::Drop;
        self
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
}

impl DataTransform for Lump {
    type Output = LumpOutput;

    fn into_compiled_and_output(
        self,
        ctx: DataTransformCompileContext,
    ) -> Result<(Box<dyn CompiledDataTransform>, Self::Output), AvengerChartError> {
        if avenger_chart_core::contains_aggregate(&self.top_n) || self.top_n.any_column_refs() {
            return Err(AvengerChartError::InvalidArgument(
                "Lump::top_n(..., n) requires a scalar integer expression such as a literal or parameter".to_string(),
            ));
        }
        if avenger_chart_core::contains_aggregate(&self.value) {
            return Err(AvengerChartError::InvalidArgument(
                "Lump::top_n(...) does not accept aggregate value expressions; use Aggregate before Lump"
                    .to_string(),
            ));
        }
        if !avenger_chart_core::contains_aggregate(&self.order_by) {
            return Err(AvengerChartError::InvalidArgument(
                "Lump::order_by(...) requires an aggregate expression".to_string(),
            ));
        }
        validate_window_function_shape(&self.window)?;

        let base_name = self
            .name
            .unwrap_or_else(|| sanitize_output_name(&format!("{}_lump", self.value)));
        let value_name = format!("{base_name}_value");
        let rank_name = format!("{base_name}_rank");
        let measure_name = format!("{base_name}_measure");
        let is_other_name = format!("{base_name}_is_other");
        let order_name = format!("{base_name}_order");
        let order_is_other_name = format!("{base_name}_order_is_other");
        let order_rank_name = format!("{base_name}_order_rank");
        let other_mode = match self.other_mode {
            LumpOtherModeAuthoring::DefaultStringOther => LumpOtherMode::DefaultStringOther,
            LumpOtherModeAuthoring::Value(expr) => LumpOtherMode::Value {
                expr: expr_node(expr, "lump other value expression"),
            },
            LumpOtherModeAuthoring::Drop => LumpOtherMode::Drop,
        };
        let transform = CompiledLumpTransform {
            value: expr_node(self.value, "lump value expression"),
            top_n: expr_node(self.top_n, "lump top_n expression"),
            order_by: expr_node(self.order_by, "lump order_by expression"),
            order_descending: self.order_descending,
            window: expr_node(self.window, "lump window expression"),
            keep: expr_node(self.keep, "lump keep expression"),
            other_mode,
            value_name: value_name.clone(),
            rank_name: rank_name.clone(),
            measure_name: measure_name.clone(),
            is_other_name: is_other_name.clone(),
            order_name: order_name.clone(),
            order_is_other_name: order_is_other_name.clone(),
            order_rank_name: order_rank_name.clone(),
        };

        Ok((
            Box::new(transform),
            LumpOutput {
                value_name,
                rank_name,
                measure_name,
                is_other_name,
                order_name,
                scope: ctx.scope,
            },
        ))
    }
}

#[derive(Clone, Debug)]
pub struct LumpOutput {
    value_name: String,
    rank_name: String,
    measure_name: String,
    is_other_name: String,
    order_name: String,
    scope: avenger_chart_core::CoordinationScope,
}

impl LumpOutput {
    pub fn value(&self) -> ChannelExpr {
        let order_name = self.order_name.clone();
        ChannelExpr::scaled(col(&self.value_name))
            .scale(move |scale| scale.order_by(min(col(&order_name))).order_asc())
            .with_transform_scope(self.scope)
    }

    pub fn rank(&self) -> Expr {
        col(&self.rank_name)
    }

    pub fn measure(&self) -> Expr {
        col(&self.measure_name)
    }

    pub fn is_other(&self) -> Expr {
        col(&self.is_other_name)
    }

    pub fn order(&self) -> Expr {
        col(&self.order_name)
    }
}

#[typetag::serde(name = "lump")]
#[async_trait]
impl CompiledDataTransform for CompiledLumpTransform {
    fn clone_box(&self) -> Box<dyn CompiledDataTransform> {
        Box::new(self.clone())
    }

    fn map_exprs(
        &self,
        f: &mut dyn FnMut(Expr) -> Result<Expr, AvengerChartError>,
    ) -> Result<Box<dyn CompiledDataTransform>, AvengerChartError> {
        Ok(Box::new(Self {
            value: map_expr_node(&self.value, f)?,
            top_n: map_expr_node(&self.top_n, f)?,
            order_by: map_expr_node(&self.order_by, f)?,
            order_descending: self.order_descending,
            window: map_expr_node(&self.window, f)?,
            keep: map_expr_node(&self.keep, f)?,
            other_mode: match &self.other_mode {
                LumpOtherMode::DefaultStringOther => LumpOtherMode::DefaultStringOther,
                LumpOtherMode::Value { expr } => LumpOtherMode::Value {
                    expr: map_expr_node(expr, f)?,
                },
                LumpOtherMode::Drop => LumpOtherMode::Drop,
            },
            value_name: self.value_name.clone(),
            rank_name: self.rank_name.clone(),
            measure_name: self.measure_name.clone(),
            is_other_name: self.is_other_name.clone(),
            order_name: self.order_name.clone(),
            order_is_other_name: self.order_is_other_name.clone(),
            order_rank_name: self.order_rank_name.clone(),
        }))
    }

    async fn apply(
        &self,
        dataframe: DataFrame,
        ctx: &DataTransformExecutionContext<'_>,
    ) -> Result<DataTransformResult, AvengerChartError> {
        validate_output_names(
            dataframe.schema().fields().iter().map(|field| field.name()),
            [
                self.value_name.as_str(),
                self.rank_name.as_str(),
                self.measure_name.as_str(),
                self.is_other_name.as_str(),
                self.order_name.as_str(),
                self.order_is_other_name.as_str(),
                self.order_rank_name.as_str(),
            ],
        )?;
        validate_top_n(self.top_n.to_default_expr(ctx.session_context)?, ctx).await?;
        let dataframe = apply_lump(dataframe, self, ctx.session_context)?;
        Ok(DataTransformResult::dataframe(dataframe))
    }
}

async fn validate_top_n(
    expr: Expr,
    ctx: &DataTransformExecutionContext<'_>,
) -> Result<(), AvengerChartError> {
    let params = params_to_datafusion(ctx.params);
    let mut values = eval_to_scalars(vec![expr], Some(ctx.session_context), params.as_ref())
        .await
        .map_err(AvengerChartError::DataFusionError)?;
    let value = values.pop().ok_or_else(|| {
        AvengerChartError::InternalError("Lump top_n returned no value".to_string())
    })?;
    let n = match value {
        ScalarValue::Int8(Some(value)) => i64::from(value),
        ScalarValue::Int16(Some(value)) => i64::from(value),
        ScalarValue::Int32(Some(value)) => i64::from(value),
        ScalarValue::Int64(Some(value)) => value,
        ScalarValue::UInt8(Some(value)) => i64::from(value),
        ScalarValue::UInt16(Some(value)) => i64::from(value),
        ScalarValue::UInt32(Some(value)) => i64::from(value),
        ScalarValue::UInt64(Some(value)) => i64::try_from(value).map_err(|_| {
            AvengerChartError::InvalidArgument(
                "Lump::top_n(..., n) must evaluate to a positive integer that fits in Int64"
                    .to_string(),
            )
        })?,
        other if other.is_null() => {
            return Err(AvengerChartError::InvalidArgument(
                "Lump::top_n(..., n) must not evaluate to null".to_string(),
            ));
        }
        other => {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Lump::top_n(..., n) must evaluate to an integer scalar, got {other:?}"
            )));
        }
    };
    if n <= 0 {
        return Err(AvengerChartError::InvalidArgument(
            "Lump::top_n(..., n) must evaluate to a positive integer".to_string(),
        ));
    }
    Ok(())
}

fn validate_window_function_shape(expr: &Expr) -> Result<(), AvengerChartError> {
    let Expr::WindowFunction(window) = expr else {
        return Err(AvengerChartError::InvalidArgument(
            "Lump::window(...) expects a DataFusion window function expression such as row_number(), rank(), percent_rank(), or ntile(...)".to_string(),
        ));
    };
    let params = &window.params;
    if !params.partition_by.is_empty()
        || !params.order_by.is_empty()
        || params.window_frame != WindowFrame::new(None)
    {
        return Err(AvengerChartError::InvalidArgument(
            "Lump::window(...) expects only the window function call; Lump owns OVER partitioning, ordering, and frame clauses".to_string(),
        ));
    }
    Ok(())
}

fn replace_lump_placeholders(
    expr: Expr,
    original_expr: Expr,
    measure_expr: Expr,
    window_expr: Expr,
) -> Result<Expr, AvengerChartError> {
    expr.transform(|candidate| {
        if let Expr::Placeholder(placeholder) = &candidate {
            let replacement = match placeholder.id.as_str() {
                LUMP_VALUE_PLACEHOLDER_ID => Some(original_expr.clone()),
                LUMP_MEASURE_PLACEHOLDER_ID => Some(measure_expr.clone()),
                LUMP_WINDOW_PLACEHOLDER_ID => Some(window_expr.clone()),
                _ => None,
            };
            if let Some(replacement) = replacement {
                return Ok(Transformed::yes(replacement));
            }
        }
        Ok(Transformed::no(candidate))
    })
    .map(|transformed| transformed.data)
    .map_err(AvengerChartError::DataFusionError)
}

fn window_with_lump_order(expr: Expr, order_descending: bool) -> Result<Expr, AvengerChartError> {
    let Expr::WindowFunction(window) = expr else {
        return Err(AvengerChartError::InvalidArgument(
            "Lump::window(...) expects a DataFusion window function expression".to_string(),
        ));
    };
    let mut window: WindowFunction = *window;
    if !window.params.partition_by.is_empty()
        || !window.params.order_by.is_empty()
        || window.params.window_frame != WindowFrame::new(None)
    {
        return Err(AvengerChartError::InvalidArgument(
            "Lump::window(...) expects only the window function call; Lump owns OVER partitioning, ordering, and frame clauses".to_string(),
        ));
    }
    window.params.args = window
        .params
        .args
        .into_iter()
        .map(|arg| {
            replace_lump_placeholders(arg, col(LUMP_ORIGINAL), col(LUMP_MEASURE), col(LUMP_WINDOW))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut order_by = vec![col(LUMP_MEASURE).sort(!order_descending, false)];
    if !rank_function_preserves_peers(window.fun.name()) {
        order_by.push(col(LUMP_ORIGINAL).sort(true, false));
    }
    window.params.order_by = order_by;
    window.params.window_frame = WindowFrame::new(Some(true));
    Ok(Expr::from(window))
}

fn rank_function_preserves_peers(name: &str) -> bool {
    matches!(name, "rank" | "dense_rank" | "percent_rank" | "cume_dist")
}

fn cast_to_f64(expr: Expr, dataframe: &DataFrame) -> Result<Expr, AvengerChartError> {
    expr.cast_to(&DataType::Float64, dataframe.schema())
        .map_err(AvengerChartError::DataFusionError)
}

fn is_string_type(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
    )
}

fn other_expr(
    payload: &CompiledLumpTransform,
    value_type: &DataType,
    dataframe: &DataFrame,
    ctx: &datafusion::prelude::SessionContext,
) -> Result<Option<Expr>, AvengerChartError> {
    match &payload.other_mode {
        LumpOtherMode::Drop => Ok(None),
        LumpOtherMode::DefaultStringOther => {
            if !is_string_type(value_type) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Lump on non-string category type {value_type:?} requires .other_value(...) or .drop_other()"
                )));
            }
            Ok(Some(lit("Other")))
        }
        LumpOtherMode::Value { expr } => {
            let expr = expr.to_default_expr(ctx)?;
            let _ = expr
                .get_type(dataframe.schema())
                .map_err(AvengerChartError::DataFusionError)?;
            Ok(Some(expr))
        }
    }
}

fn apply_lump(
    dataframe: DataFrame,
    payload: &CompiledLumpTransform,
    ctx: &datafusion::prelude::SessionContext,
) -> Result<DataFrame, AvengerChartError> {
    let original_columns = dataframe
        .schema()
        .fields()
        .iter()
        .map(|field| field.name().clone())
        .collect::<Vec<_>>();
    let value_expr = payload.value.to_default_expr(ctx)?;
    let value_type = value_expr
        .get_type(dataframe.schema())
        .map_err(AvengerChartError::DataFusionError)?;
    let prepared = dataframe
        .with_column(LUMP_VALUE_INTERNAL, value_expr)
        .map_err(AvengerChartError::DataFusionError)?;
    let order_by_expr = payload.order_by.to_default_expr(ctx)?;

    let mut ranking = prepared
        .clone()
        .aggregate(
            vec![col(LUMP_VALUE_INTERNAL).alias(LUMP_ORIGINAL)],
            vec![order_by_expr.alias(LUMP_MEASURE)],
        )
        .map_err(AvengerChartError::DataFusionError)?;
    let measure_expr = cast_to_f64(col(LUMP_MEASURE), &ranking)?;
    ranking = ranking
        .with_column(LUMP_MEASURE, measure_expr)
        .map_err(AvengerChartError::DataFusionError)?;
    let window_expr = window_with_lump_order(
        payload.window.to_default_expr(ctx)?,
        payload.order_descending,
    )?;
    ranking = ranking
        .with_column(LUMP_WINDOW, window_expr)
        .map_err(AvengerChartError::DataFusionError)?;
    let keep_expr = replace_lump_placeholders(
        payload.keep.to_default_expr(ctx)?,
        col(LUMP_ORIGINAL),
        col(LUMP_MEASURE),
        col(LUMP_WINDOW),
    )?;
    ranking = ranking
        .with_column(LUMP_KEEP, keep_expr)
        .map_err(AvengerChartError::DataFusionError)?;

    let joined = prepared
        .join_on(
            ranking,
            JoinType::Inner,
            [binary_expr(
                col(LUMP_VALUE_INTERNAL),
                Operator::IsNotDistinctFrom,
                col(LUMP_ORIGINAL),
            )],
        )
        .map_err(AvengerChartError::DataFusionError)?;
    let other = other_expr(payload, &value_type, &joined, ctx)?;
    let keep = col(LUMP_KEEP);
    let mut output = if matches!(payload.other_mode, LumpOtherMode::Drop) {
        joined
            .filter(keep.clone())
            .map_err(AvengerChartError::DataFusionError)?
    } else {
        joined
    };

    let value_output = if let Some(other) = other {
        when(keep.clone(), col(LUMP_ORIGINAL))
            .otherwise(other)
            .map_err(AvengerChartError::DataFusionError)?
    } else {
        col(LUMP_ORIGINAL)
    };
    let rank_as_f64 = cast_to_f64(col(LUMP_WINDOW), &output)?;
    let rank_output = when(keep.clone(), rank_as_f64.clone())
        .otherwise(lit(ScalarValue::Float64(None)))
        .map_err(AvengerChartError::DataFusionError)?;
    let measure_output = when(keep.clone(), col(LUMP_MEASURE))
        .otherwise(lit(ScalarValue::Float64(None)))
        .map_err(AvengerChartError::DataFusionError)?;
    let is_other_output = when(keep.clone(), lit(false))
        .otherwise(lit(true))
        .map_err(AvengerChartError::DataFusionError)?;
    let order_is_other_output = when(keep.clone(), lit(0.0))
        .otherwise(lit(1.0))
        .map_err(AvengerChartError::DataFusionError)?;
    let order_rank_output = when(keep.clone(), rank_as_f64.clone())
        .otherwise(lit(ScalarValue::Float64(None)))
        .map_err(AvengerChartError::DataFusionError)?;
    let order_output = when(keep, rank_as_f64)
        .otherwise(lit(1_000_000_000_000_000.0))
        .map_err(AvengerChartError::DataFusionError)?;

    output = output
        .with_column(&payload.value_name, value_output)
        .map_err(AvengerChartError::DataFusionError)?
        .with_column(&payload.rank_name, rank_output)
        .map_err(AvengerChartError::DataFusionError)?
        .with_column(&payload.measure_name, measure_output)
        .map_err(AvengerChartError::DataFusionError)?
        .with_column(&payload.is_other_name, is_other_output)
        .map_err(AvengerChartError::DataFusionError)?
        .with_column(&payload.order_is_other_name, order_is_other_output)
        .map_err(AvengerChartError::DataFusionError)?
        .with_column(&payload.order_rank_name, order_rank_output)
        .map_err(AvengerChartError::DataFusionError)?
        .with_column(&payload.order_name, order_output)
        .map_err(AvengerChartError::DataFusionError)?;

    let mut projection = original_columns
        .iter()
        .map(|name| col(name))
        .collect::<Vec<_>>();
    projection.extend([
        col(&payload.value_name),
        col(&payload.rank_name),
        col(&payload.measure_name),
        col(&payload.is_other_name),
        col(&payload.order_name),
        col(&payload.order_is_other_name),
        col(&payload.order_rank_name),
    ]);
    output
        .select(projection)
        .map_err(AvengerChartError::DataFusionError)
}
