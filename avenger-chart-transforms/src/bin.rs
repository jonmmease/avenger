use crate::common::{
    expr_node, map_expr_node, map_optional_expr_node, sanitize_output_name, validate_output_names,
};
use async_trait::async_trait;
use avenger_chart_cartesian::CartesianAxis;
use avenger_chart_core::{
    AvengerChartError, ChannelExpr, CompiledDataTransform, DataTransform,
    DataTransformCompileContext, DataTransformExecutionContext, DataTransformResult,
    DefaultLogicalExprNodeExt, DerivedScalarMap, IntoExpr, ScaleChannelValue, SerializableExpr,
    derived_scalar,
};
use datafusion::{
    arrow::{
        array::Float64Array,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    dataframe::DataFrame,
    functions::expr_fn::{abs, ceil, floor, ln, power, round},
    functions_aggregate::expr_fn::{count, max, min},
    logical_expr::{Expr, ExprSchemable, JoinType, col, expr_fn::scalar_subquery, lit, when},
    prelude::named_struct,
};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};
use std::sync::Arc;

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledBinTransform {
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub value: LogicalExprNode,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub maxbins: LogicalExprNode,
    pub nice: bool,
    pub base: usize,
    pub divide: Vec<f64>,
    pub steps: Option<Vec<f64>>,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub minstep: LogicalExprNode,
    #[serde_as(as = "Option<FromInto<SerializableExpr>>")]
    pub step: Option<LogicalExprNode>,
    pub extent: Option<BinExtentSpec>,
    #[serde_as(as = "Option<FromInto<SerializableExpr>>")]
    pub span: Option<LogicalExprNode>,
    #[serde_as(as = "Option<FromInto<SerializableExpr>>")]
    pub anchor: Option<LogicalExprNode>,
    pub start_name: String,
    pub end_name: String,
    pub index_name: String,
    pub domain_start_scalar_id: String,
    pub domain_end_scalar_id: String,
    pub tick_spacing_scalar_id: String,
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BinExtentSpec {
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub start: LogicalExprNode,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub stop: LogicalExprNode,
}

#[derive(Clone, Debug)]
pub struct Bin {
    value: Expr,
    maxbins: Expr,
    nice: bool,
    base: usize,
    divide: Vec<f64>,
    steps: Option<Vec<f64>>,
    minstep: Expr,
    step: Option<Expr>,
    extent: Option<(Expr, Expr)>,
    span: Option<Expr>,
    anchor: Option<Expr>,
    name: Option<String>,
}

impl Bin {
    pub fn new(value: impl IntoExpr) -> Self {
        Self {
            value: value.into_expr(),
            maxbins: lit(10.0),
            nice: true,
            base: 10,
            divide: vec![5.0, 2.0],
            steps: None,
            minstep: lit(0.0),
            step: None,
            extent: None,
            span: None,
            anchor: None,
            name: None,
        }
    }

    pub fn maxbins(mut self, maxbins: impl IntoExpr) -> Self {
        self.maxbins = maxbins.into_expr();
        self
    }

    pub fn nice(mut self) -> Self {
        self.nice = true;
        self
    }

    pub fn exact(mut self) -> Self {
        self.nice = false;
        self
    }

    pub fn base(mut self, base: usize) -> Self {
        self.base = base;
        self
    }

    pub fn divide<I>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = usize>,
    {
        self.divide = values.into_iter().map(|value| value as f64).collect();
        self
    }

    pub fn steps<I>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = f64>,
    {
        let mut values = values.into_iter().collect::<Vec<_>>();
        values.sort_by(|a, b| a.total_cmp(b));
        self.steps = Some(values);
        self
    }

    pub fn minstep(mut self, minstep: impl IntoExpr) -> Self {
        self.minstep = minstep.into_expr();
        self
    }

    pub fn step(mut self, step: impl IntoExpr) -> Self {
        self.step = Some(step.into_expr());
        self
    }

    pub fn extent(mut self, start: impl IntoExpr, stop: impl IntoExpr) -> Self {
        self.extent = Some((start.into_expr(), stop.into_expr()));
        self
    }

    pub fn span(mut self, span: impl IntoExpr) -> Self {
        self.span = Some(span.into_expr());
        self
    }

    pub fn anchor(mut self, anchor: impl IntoExpr) -> Self {
        self.anchor = Some(anchor.into_expr());
        self
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
}

impl DataTransform for Bin {
    type Output = BinOutput;

    fn into_compiled_and_output(
        self,
        ctx: DataTransformCompileContext,
    ) -> Result<(Box<dyn CompiledDataTransform>, Self::Output), AvengerChartError> {
        if self.base <= 1 {
            return Err(AvengerChartError::InvalidArgument(
                "Bin::base(...) must be greater than one".to_string(),
            ));
        }
        if self
            .divide
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
        {
            return Err(AvengerChartError::InvalidArgument(
                "Bin::divide(...) values must be positive finite numbers".to_string(),
            ));
        }
        if let Some(steps) = &self.steps
            && steps
                .iter()
                .any(|value| !value.is_finite() || *value <= 0.0)
        {
            return Err(AvengerChartError::InvalidArgument(
                "Bin::steps(...) values must be positive finite numbers".to_string(),
            ));
        }
        if let Some(maxbins) = literal_numeric_value(&self.maxbins)
            && maxbins < 1.0
        {
            return Err(AvengerChartError::InvalidArgument(
                "Bin::maxbins(...) must be at least one".to_string(),
            ));
        }
        if let Some(minstep) = literal_numeric_value(&self.minstep)
            && (!minstep.is_finite() || minstep < 0.0)
        {
            return Err(AvengerChartError::InvalidArgument(
                "Bin::minstep(...) must be a non-negative finite number".to_string(),
            ));
        }
        if let Some(step) = &self.step
            && let Some(step) = literal_numeric_value(step)
            && (!step.is_finite() || step <= 0.0)
        {
            return Err(AvengerChartError::InvalidArgument(
                "Bin::step(...) must be a positive finite number".to_string(),
            ));
        }
        if avenger_chart_core::contains_aggregate(&self.value) {
            return Err(AvengerChartError::InvalidArgument(
                "Bin::new(...) does not accept aggregate expressions; use Aggregate before Bin"
                    .to_string(),
            ));
        }

        let base_name = self
            .name
            .unwrap_or_else(|| sanitize_output_name(&format!("{}_bin", self.value)));
        let start_name = format!("{base_name}_start");
        let end_name = format!("{base_name}_end");
        let index_name = format!("{base_name}_index");
        let domain_start_scalar_id = format!("{base_name}_domain_start");
        let domain_end_scalar_id = format!("{base_name}_domain_end");
        let tick_spacing_scalar_id = format!("{base_name}_tick_spacing");
        let transform = CompiledBinTransform {
            value: expr_node(self.value, "bin value expression"),
            maxbins: expr_node(self.maxbins, "bin maxbins expression"),
            nice: self.nice,
            base: self.base,
            divide: self.divide,
            steps: self.steps,
            minstep: expr_node(self.minstep, "bin minstep expression"),
            step: self.step.map(|expr| expr_node(expr, "bin step expression")),
            extent: self.extent.map(|(start, stop)| BinExtentSpec {
                start: expr_node(start, "bin extent start expression"),
                stop: expr_node(stop, "bin extent stop expression"),
            }),
            span: self.span.map(|expr| expr_node(expr, "bin span expression")),
            anchor: self
                .anchor
                .map(|expr| expr_node(expr, "bin anchor expression")),
            start_name: start_name.clone(),
            end_name: end_name.clone(),
            index_name: index_name.clone(),
            domain_start_scalar_id: domain_start_scalar_id.clone(),
            domain_end_scalar_id: domain_end_scalar_id.clone(),
            tick_spacing_scalar_id: tick_spacing_scalar_id.clone(),
        };

        Ok((
            Box::new(transform),
            BinOutput {
                start_name,
                end_name,
                index_name,
                domain_start_scalar_id,
                domain_end_scalar_id,
                tick_spacing_scalar_id,
                scope: ctx.scope,
            },
        ))
    }
}

#[derive(Clone, Debug)]
pub struct BinOutput {
    start_name: String,
    end_name: String,
    index_name: String,
    domain_start_scalar_id: String,
    domain_end_scalar_id: String,
    tick_spacing_scalar_id: String,
    scope: avenger_chart_core::CoordinationScope,
}

impl BinOutput {
    pub fn start(&self) -> ChannelExpr {
        self.position_channel(&self.start_name)
    }

    pub fn end(&self) -> ChannelExpr {
        self.position_channel(&self.end_name)
    }

    fn position_channel(&self, column_name: &str) -> ChannelExpr {
        let domain_start_scalar_id = self.domain_start_scalar_id.clone();
        let domain_end_scalar_id = self.domain_end_scalar_id.clone();
        let tick_spacing_scalar_id = self.tick_spacing_scalar_id.clone();
        ChannelExpr::scaled(col(column_name))
            .scale(move |scale| {
                scale
                    .domain_interval(
                        derived_scalar(&domain_start_scalar_id, Some(DataType::Float64)),
                        derived_scalar(&domain_end_scalar_id, Some(DataType::Float64)),
                    )
                    .option("nice", lit(false))
                    .option("zero", lit(false))
            })
            .with_axis_config(
                CartesianAxis::new().tick_spacing(derived_scalar(&tick_spacing_scalar_id, None)),
            )
            .with_transform_scope(self.scope)
    }

    pub fn index(&self) -> Expr {
        col(&self.index_name)
    }
}

#[typetag::serde(name = "bin")]
#[async_trait]
impl CompiledDataTransform for CompiledBinTransform {
    fn clone_box(&self) -> Box<dyn CompiledDataTransform> {
        Box::new(self.clone())
    }

    fn map_exprs(
        &self,
        f: &mut dyn FnMut(Expr) -> Result<Expr, AvengerChartError>,
    ) -> Result<Box<dyn CompiledDataTransform>, AvengerChartError> {
        Ok(Box::new(Self {
            value: map_expr_node(&self.value, f)?,
            maxbins: map_expr_node(&self.maxbins, f)?,
            nice: self.nice,
            base: self.base,
            divide: self.divide.clone(),
            steps: self.steps.clone(),
            minstep: map_expr_node(&self.minstep, f)?,
            step: map_optional_expr_node(&self.step, f)?,
            extent: self
                .extent
                .as_ref()
                .map(|extent| {
                    Ok::<_, AvengerChartError>(BinExtentSpec {
                        start: map_expr_node(&extent.start, f)?,
                        stop: map_expr_node(&extent.stop, f)?,
                    })
                })
                .transpose()?,
            span: map_optional_expr_node(&self.span, f)?,
            anchor: map_optional_expr_node(&self.anchor, f)?,
            start_name: self.start_name.clone(),
            end_name: self.end_name.clone(),
            index_name: self.index_name.clone(),
            domain_start_scalar_id: self.domain_start_scalar_id.clone(),
            domain_end_scalar_id: self.domain_end_scalar_id.clone(),
            tick_spacing_scalar_id: self.tick_spacing_scalar_id.clone(),
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
                self.start_name.as_str(),
                self.end_name.as_str(),
                self.index_name.as_str(),
            ],
        )?;

        let (prepared, original_columns) =
            prepared_bin_value_dataframe(dataframe, self, ctx.session_context)?;
        let plan = bin_plan_dataframe(prepared.clone(), self, ctx.session_context)?;
        let derived_scalars = bin_derived_scalars_from_plan(plan.clone(), self)?;
        let dataframe = apply_bin_with_plan(prepared, plan, self, original_columns)?;
        Ok(DataTransformResult {
            dataframe,
            derived_scalars,
        })
    }
}

const BIN_VALUE: &str = "__avenger_bin_value";
const BIN_MIN: &str = "__avenger_bin_min";
const BIN_MAX: &str = "__avenger_bin_max";
const BIN_RAW_SPAN: &str = "__avenger_bin_raw_span";
const BIN_SPAN: &str = "__avenger_bin_span";
const BIN_MAXBINS: &str = "__avenger_bin_maxbins";
const BIN_MINSTEP: &str = "__avenger_bin_minstep";
const BIN_STEP: &str = "__avenger_bin_step";
const BIN_START: &str = "__avenger_bin_start";
const BIN_STOP: &str = "__avenger_bin_stop";
const BIN_COUNT: &str = "__avenger_bin_plan_count";
const BIN_EPSILON: &str = "__avenger_bin_epsilon";
const BIN_FACTOR: &str = "__avenger_bin_factor";
const BIN_DIVIDE: &str = "__avenger_bin_divide";
const BIN_CANDIDATE_STEP: &str = "__avenger_bin_candidate_step";
const BIN_CANDIDATE_COUNT: &str = "__avenger_bin_candidate_count";
const BIN_VALID_ORDER: &str = "__avenger_bin_valid_order";
const BIN_SORT_STEP: &str = "__avenger_bin_sort_step";
const BIN_INDEX_FLOAT: &str = "__avenger_bin_index_float";

fn literal_numeric_value(expr: &Expr) -> Option<f64> {
    let Expr::Literal(value, _) = expr else {
        return None;
    };
    match value {
        ScalarValue::Float64(Some(value)) => Some(*value),
        ScalarValue::Float32(Some(value)) => Some(*value as f64),
        ScalarValue::Int64(Some(value)) => Some(*value as f64),
        ScalarValue::Int32(Some(value)) => Some(*value as f64),
        ScalarValue::Int16(Some(value)) => Some(*value as f64),
        ScalarValue::Int8(Some(value)) => Some(*value as f64),
        ScalarValue::UInt64(Some(value)) => Some(*value as f64),
        ScalarValue::UInt32(Some(value)) => Some(*value as f64),
        ScalarValue::UInt16(Some(value)) => Some(*value as f64),
        ScalarValue::UInt8(Some(value)) => Some(*value as f64),
        _ => None,
    }
}

fn scalar_from_dataframe(dataframe: DataFrame, expr: Expr) -> Result<Expr, AvengerChartError> {
    let dataframe = dataframe
        .select(vec![expr.alias("__avenger_bin_scalar")])
        .map_err(AvengerChartError::DataFusionError)?;
    Ok(scalar_subquery(Arc::new(dataframe.into_unoptimized_plan())))
}

fn cast_to_f64(expr: Expr, dataframe: &DataFrame) -> Result<Expr, AvengerChartError> {
    expr.cast_to(&DataType::Float64, dataframe.schema())
        .map_err(AvengerChartError::DataFusionError)
}

fn max_expr(left: Expr, right: Expr) -> Result<Expr, AvengerChartError> {
    when(left.clone().gt(right.clone()), left)
        .otherwise(right)
        .map_err(AvengerChartError::DataFusionError)
}

fn min_expr(left: Expr, right: Expr) -> Result<Expr, AvengerChartError> {
    when(left.clone().lt(right.clone()), left)
        .otherwise(right)
        .map_err(AvengerChartError::DataFusionError)
}

fn positive_or_one(expr: Expr) -> Result<Expr, AvengerChartError> {
    when(expr.clone().gt(lit(0.0)), expr)
        .otherwise(lit(1.0))
        .map_err(AvengerChartError::DataFusionError)
}

fn prepared_bin_value_dataframe(
    dataframe: DataFrame,
    payload: &CompiledBinTransform,
    ctx: &datafusion::prelude::SessionContext,
) -> Result<(DataFrame, Vec<String>), AvengerChartError> {
    let original_columns = dataframe
        .schema()
        .fields()
        .iter()
        .map(|field| field.name().clone())
        .collect::<Vec<_>>();
    let value_expr = payload.value.to_default_expr(ctx)?;
    let value_expr = cast_to_f64(value_expr, &dataframe)?;
    let dataframe = dataframe
        .with_column(BIN_VALUE, value_expr)
        .map_err(AvengerChartError::DataFusionError)?;
    Ok((dataframe, original_columns))
}

fn one_row_dataframe_from_input(dataframe: DataFrame) -> Result<DataFrame, AvengerChartError> {
    dataframe
        .aggregate(
            Vec::<Expr>::new(),
            vec![count(lit(1)).alias("__avenger_bin_rows")],
        )
        .map_err(AvengerChartError::DataFusionError)
}

fn bin_extent_dataframe(
    dataframe: DataFrame,
    payload: &CompiledBinTransform,
    ctx: &datafusion::prelude::SessionContext,
) -> Result<DataFrame, AvengerChartError> {
    if let Some(extent) = &payload.extent {
        let dataframe = one_row_dataframe_from_input(dataframe)?;
        let start = extent.start.to_default_expr(ctx)?;
        let stop = extent.stop.to_default_expr(ctx)?;
        let start = cast_to_f64(start, &dataframe)?;
        let stop = cast_to_f64(stop, &dataframe)?;
        dataframe
            .select(vec![start.alias(BIN_MIN), stop.alias(BIN_MAX)])
            .map_err(AvengerChartError::DataFusionError)
    } else {
        dataframe
            .aggregate(
                Vec::<Expr>::new(),
                vec![
                    min(col(BIN_VALUE)).alias(BIN_MIN),
                    max(col(BIN_VALUE)).alias(BIN_MAX),
                ],
            )
            .map_err(AvengerChartError::DataFusionError)
    }
}

fn cross_join(left: DataFrame, right: DataFrame) -> Result<DataFrame, AvengerChartError> {
    left.join_on(right, JoinType::Inner, [lit(true)])
        .map_err(AvengerChartError::DataFusionError)
}

fn inline_f64_dataframe(
    ctx: &datafusion::prelude::SessionContext,
    name: &str,
    values: Vec<f64>,
) -> Result<DataFrame, AvengerChartError> {
    let schema = Arc::new(Schema::new(vec![Field::new(
        name,
        DataType::Float64,
        false,
    )]));
    let batch = RecordBatch::try_new(schema, vec![Arc::new(Float64Array::from(values))])
        .map_err(AvengerChartError::ArrowError)?;
    ctx.read_batch(batch)
        .map_err(AvengerChartError::DataFusionError)
}

fn bin_base_constants_dataframe(
    dataframe: DataFrame,
    payload: &CompiledBinTransform,
    ctx: &datafusion::prelude::SessionContext,
) -> Result<DataFrame, AvengerChartError> {
    let extent_df = bin_extent_dataframe(dataframe, payload, ctx)?;
    let maxbins = cast_to_f64(payload.maxbins.to_default_expr(ctx)?, &extent_df)?;
    let maxbins = floor(maxbins);
    let minstep = cast_to_f64(payload.minstep.to_default_expr(ctx)?, &extent_df)?;
    let raw_span = col(BIN_MAX) - col(BIN_MIN);
    let span = if let Some(span) = &payload.span {
        cast_to_f64(span.to_default_expr(ctx)?, &extent_df)?
    } else {
        let abs_raw_span = abs(raw_span.clone());
        when(abs_raw_span.clone().gt(lit(0.0)), abs_raw_span)
            .otherwise(positive_or_one(abs(col(BIN_MIN)))?)
            .map_err(AvengerChartError::DataFusionError)?
    };

    extent_df
        .select(vec![
            col(BIN_MIN),
            col(BIN_MAX),
            raw_span.alias(BIN_RAW_SPAN),
            span.alias(BIN_SPAN),
            maxbins.alias(BIN_MAXBINS),
            minstep.alias(BIN_MINSTEP),
        ])
        .map_err(AvengerChartError::DataFusionError)
}

fn bin_initial_step_dataframe(dataframe: DataFrame) -> Result<DataFrame, AvengerChartError> {
    let base = lit(10.0);
    let logb = ln(base.clone());
    let level = ceil(ln(col(BIN_MAXBINS)) / logb.clone());
    let power_step = power(base, round(vec![ln(col(BIN_SPAN)) / logb - level]));
    let initial_step = max_expr(col(BIN_MINSTEP), power_step)?;
    dataframe
        .with_column(BIN_STEP, initial_step)
        .map_err(AvengerChartError::DataFusionError)
}

fn bin_factor_refined_dataframe(
    dataframe: DataFrame,
    payload: &CompiledBinTransform,
    ctx: &datafusion::prelude::SessionContext,
) -> Result<DataFrame, AvengerChartError> {
    let factor_df =
        inline_f64_dataframe(ctx, BIN_FACTOR, (0..10).map(|value| value as f64).collect())?;
    let base = lit(payload.base as f64);
    let dataframe = cross_join(dataframe, factor_df)?
        .with_column(
            BIN_CANDIDATE_STEP,
            col(BIN_STEP) * power(base, col(BIN_FACTOR)),
        )
        .map_err(AvengerChartError::DataFusionError)?
        .with_column(
            BIN_CANDIDATE_COUNT,
            ceil(col(BIN_SPAN) / col(BIN_CANDIDATE_STEP)),
        )
        .map_err(AvengerChartError::DataFusionError)?
        .filter(col(BIN_CANDIDATE_COUNT).lt_eq(col(BIN_MAXBINS)))
        .map_err(AvengerChartError::DataFusionError)?
        .sort(vec![col(BIN_CANDIDATE_STEP).sort(true, false)])
        .map_err(AvengerChartError::DataFusionError)?
        .limit(0, Some(1))
        .map_err(AvengerChartError::DataFusionError)?;

    dataframe
        .select(vec![
            col(BIN_MIN),
            col(BIN_MAX),
            col(BIN_RAW_SPAN),
            col(BIN_SPAN),
            col(BIN_MAXBINS),
            col(BIN_MINSTEP),
            col(BIN_CANDIDATE_STEP).alias(BIN_STEP),
        ])
        .map_err(AvengerChartError::DataFusionError)
}

fn bin_divide_refined_dataframe(
    dataframe: DataFrame,
    payload: &CompiledBinTransform,
    ctx: &datafusion::prelude::SessionContext,
) -> Result<DataFrame, AvengerChartError> {
    let mut divide_values = payload.divide.clone();
    if !divide_values.iter().any(|value| *value == 1.0) {
        divide_values.push(1.0);
    }
    let divide_df = inline_f64_dataframe(ctx, BIN_DIVIDE, divide_values)?;
    let dataframe = cross_join(dataframe, divide_df)?
        .with_column(BIN_CANDIDATE_STEP, col(BIN_STEP) / col(BIN_DIVIDE))
        .map_err(AvengerChartError::DataFusionError)?
        .with_column(
            BIN_CANDIDATE_COUNT,
            ceil(col(BIN_SPAN) / col(BIN_CANDIDATE_STEP)),
        )
        .map_err(AvengerChartError::DataFusionError)?
        .filter(
            col(BIN_CANDIDATE_STEP)
                .gt_eq(col(BIN_MINSTEP))
                .and(col(BIN_CANDIDATE_COUNT).lt_eq(col(BIN_MAXBINS))),
        )
        .map_err(AvengerChartError::DataFusionError)?
        .sort(vec![col(BIN_CANDIDATE_STEP).sort(true, false)])
        .map_err(AvengerChartError::DataFusionError)?
        .limit(0, Some(1))
        .map_err(AvengerChartError::DataFusionError)?;

    dataframe
        .select(vec![
            col(BIN_MIN),
            col(BIN_MAX),
            col(BIN_RAW_SPAN),
            col(BIN_SPAN),
            col(BIN_MAXBINS),
            col(BIN_MINSTEP),
            col(BIN_CANDIDATE_STEP).alias(BIN_STEP),
        ])
        .map_err(AvengerChartError::DataFusionError)
}

fn bin_steps_dataframe(
    dataframe: DataFrame,
    steps: &[f64],
    ctx: &datafusion::prelude::SessionContext,
) -> Result<DataFrame, AvengerChartError> {
    let steps_df = inline_f64_dataframe(ctx, BIN_CANDIDATE_STEP, steps.to_vec())?;
    let valid = col(BIN_CANDIDATE_COUNT).lt_eq(col(BIN_MAXBINS));
    let dataframe = cross_join(dataframe, steps_df)?
        .with_column(
            BIN_CANDIDATE_COUNT,
            ceil(col(BIN_SPAN) / col(BIN_CANDIDATE_STEP)),
        )
        .map_err(AvengerChartError::DataFusionError)?
        .with_column(
            BIN_VALID_ORDER,
            when(valid.clone(), lit(0.0))
                .otherwise(lit(1.0))
                .map_err(AvengerChartError::DataFusionError)?,
        )
        .map_err(AvengerChartError::DataFusionError)?
        .with_column(
            BIN_SORT_STEP,
            when(valid, col(BIN_CANDIDATE_STEP))
                .otherwise(lit(0.0) - col(BIN_CANDIDATE_STEP))
                .map_err(AvengerChartError::DataFusionError)?,
        )
        .map_err(AvengerChartError::DataFusionError)?
        .sort(vec![
            col(BIN_VALID_ORDER).sort(true, false),
            col(BIN_SORT_STEP).sort(true, false),
        ])
        .map_err(AvengerChartError::DataFusionError)?
        .limit(0, Some(1))
        .map_err(AvengerChartError::DataFusionError)?;

    dataframe
        .select(vec![
            col(BIN_MIN),
            col(BIN_MAX),
            col(BIN_RAW_SPAN),
            col(BIN_SPAN),
            col(BIN_MAXBINS),
            col(BIN_MINSTEP),
            col(BIN_CANDIDATE_STEP).alias(BIN_STEP),
        ])
        .map_err(AvengerChartError::DataFusionError)
}

fn bin_exact_step_dataframe(
    dataframe: DataFrame,
    payload: &CompiledBinTransform,
    ctx: &datafusion::prelude::SessionContext,
) -> Result<DataFrame, AvengerChartError> {
    let step = if let Some(step) = &payload.step {
        cast_to_f64(step.to_default_expr(ctx)?, &dataframe)?
    } else {
        when(col(BIN_RAW_SPAN).eq(lit(0.0)), lit(1.0))
            .otherwise(col(BIN_RAW_SPAN) / col(BIN_MAXBINS))
            .map_err(AvengerChartError::DataFusionError)?
    };
    dataframe
        .with_column(BIN_STEP, step)
        .map_err(AvengerChartError::DataFusionError)
}

fn bin_step_dataframe(
    dataframe: DataFrame,
    payload: &CompiledBinTransform,
    ctx: &datafusion::prelude::SessionContext,
) -> Result<DataFrame, AvengerChartError> {
    if let Some(step) = &payload.step {
        let step = cast_to_f64(step.to_default_expr(ctx)?, &dataframe)?;
        return dataframe
            .with_column(BIN_STEP, step)
            .map_err(AvengerChartError::DataFusionError);
    }
    if let Some(steps) = &payload.steps {
        let dataframe = bin_steps_dataframe(dataframe, steps, ctx)?;
        let step = when(col(BIN_RAW_SPAN).eq(lit(0.0)), lit(1.0))
            .otherwise(col(BIN_STEP))
            .map_err(AvengerChartError::DataFusionError)?;
        return dataframe
            .with_column(BIN_STEP, step)
            .map_err(AvengerChartError::DataFusionError);
    }
    let dataframe = bin_initial_step_dataframe(dataframe)?;
    let dataframe = bin_factor_refined_dataframe(dataframe, payload, ctx)?;
    let dataframe = bin_divide_refined_dataframe(dataframe, payload, ctx)?;
    let step = when(col(BIN_RAW_SPAN).eq(lit(0.0)), lit(1.0))
        .otherwise(col(BIN_STEP))
        .map_err(AvengerChartError::DataFusionError)?;
    dataframe
        .with_column(BIN_STEP, step)
        .map_err(AvengerChartError::DataFusionError)
}

fn bin_final_plan_dataframe(
    dataframe: DataFrame,
    payload: &CompiledBinTransform,
    ctx: &datafusion::prelude::SessionContext,
) -> Result<DataFrame, AvengerChartError> {
    let logb = ln(lit(payload.base as f64));
    let epsilon = if payload.nice {
        let precision = when(ln(col(BIN_STEP)).gt_eq(lit(0.0)), lit(0.0))
            .otherwise(floor((lit(0.0) - ln(col(BIN_STEP))) / logb) + lit(1.0))
            .map_err(AvengerChartError::DataFusionError)?;
        power(lit(payload.base as f64), lit(0.0) - precision - lit(1.0))
    } else {
        lit(0.0)
    };
    let dataframe = dataframe
        .with_column(BIN_EPSILON, epsilon)
        .map_err(AvengerChartError::DataFusionError)?;

    let (start, stop) = if payload.nice {
        let v1 = floor(col(BIN_MIN) / col(BIN_STEP) + col(BIN_EPSILON)) * col(BIN_STEP);
        let start = when(col(BIN_MIN).lt(v1.clone()), v1.clone() - col(BIN_STEP))
            .otherwise(v1)
            .map_err(AvengerChartError::DataFusionError)?;
        let max1 = ceil(col(BIN_MAX) / col(BIN_STEP)) * col(BIN_STEP);
        let stop = when(
            max1.clone().eq(start.clone()),
            start.clone() + col(BIN_STEP),
        )
        .otherwise(max1)
        .map_err(AvengerChartError::DataFusionError)?;
        let stop = start.clone() + ceil((stop - start.clone()) / col(BIN_STEP)) * col(BIN_STEP);
        (start, stop)
    } else {
        let start = col(BIN_MIN);
        let stop = when(
            col(BIN_RAW_SPAN).eq(lit(0.0)),
            start.clone() + col(BIN_STEP),
        )
        .otherwise(start.clone() + ceil(col(BIN_RAW_SPAN) / col(BIN_STEP)) * col(BIN_STEP))
        .map_err(AvengerChartError::DataFusionError)?;
        (start, stop)
    };

    let dataframe = dataframe
        .with_column(BIN_START, start)
        .map_err(AvengerChartError::DataFusionError)?
        .with_column(BIN_STOP, stop)
        .map_err(AvengerChartError::DataFusionError)?;

    let dataframe = if let Some(anchor) = &payload.anchor {
        let anchor = cast_to_f64(anchor.to_default_expr(ctx)?, &dataframe)?;
        let delta = anchor.clone()
            - (col(BIN_START) + col(BIN_STEP) * floor((anchor - col(BIN_START)) / col(BIN_STEP)));
        dataframe
            .with_column("__avenger_bin_anchor_delta", delta)
            .map_err(AvengerChartError::DataFusionError)?
            .with_column(
                BIN_START,
                col(BIN_START) + col("__avenger_bin_anchor_delta"),
            )
            .map_err(AvengerChartError::DataFusionError)?
            .with_column(BIN_STOP, col(BIN_STOP) + col("__avenger_bin_anchor_delta"))
            .map_err(AvengerChartError::DataFusionError)?
    } else {
        dataframe
    };

    dataframe
        .with_column(
            BIN_COUNT,
            ceil((col(BIN_STOP) - col(BIN_START)) / col(BIN_STEP)),
        )
        .map_err(AvengerChartError::DataFusionError)?
        .select(vec![
            col(BIN_START),
            col(BIN_STOP),
            col(BIN_STEP),
            col(BIN_EPSILON),
            col(BIN_COUNT),
        ])
        .map_err(AvengerChartError::DataFusionError)
}

fn bin_plan_dataframe(
    dataframe: DataFrame,
    payload: &CompiledBinTransform,
    ctx: &datafusion::prelude::SessionContext,
) -> Result<DataFrame, AvengerChartError> {
    let dataframe = bin_base_constants_dataframe(dataframe, payload, ctx)?;
    let dataframe = if payload.nice {
        bin_step_dataframe(dataframe, payload, ctx)?
    } else {
        bin_exact_step_dataframe(dataframe, payload, ctx)?
    };
    bin_final_plan_dataframe(dataframe, payload, ctx)
}

fn bin_derived_scalars_from_plan(
    plan: DataFrame,
    payload: &CompiledBinTransform,
) -> Result<DerivedScalarMap, AvengerChartError> {
    let mut derived_scalars = DerivedScalarMap::new();
    derived_scalars.insert(
        payload.domain_start_scalar_id.clone(),
        scalar_from_dataframe(plan.clone(), col(BIN_START))?,
    );
    derived_scalars.insert(
        payload.domain_end_scalar_id.clone(),
        scalar_from_dataframe(plan.clone(), col(BIN_STOP))?,
    );
    derived_scalars.insert(
        payload.tick_spacing_scalar_id.clone(),
        scalar_from_dataframe(
            plan,
            named_struct(vec![
                lit("start"),
                col(BIN_START),
                lit("step"),
                col(BIN_STEP),
            ]),
        )?,
    );
    Ok(derived_scalars)
}

fn apply_bin_with_plan(
    dataframe: DataFrame,
    plan: DataFrame,
    payload: &CompiledBinTransform,
    original_columns: Vec<String>,
) -> Result<DataFrame, AvengerChartError> {
    let mut dataframe = cross_join(dataframe, plan)?;
    let raw_index = floor(col(BIN_EPSILON) + (col(BIN_VALUE) - col(BIN_START)) / col(BIN_STEP));
    let index_float = min_expr(max_expr(raw_index, lit(0.0))?, col(BIN_COUNT) - lit(1.0))?;
    let bin_start = when(col(BIN_VALUE).is_null(), lit(ScalarValue::Float64(None)))
        .otherwise(
            when(
                col(BIN_VALUE).lt(col(BIN_START)),
                lit(ScalarValue::Float64(None)),
            )
            .otherwise(
                when(
                    col(BIN_VALUE).gt(col(BIN_STOP)),
                    lit(ScalarValue::Float64(None)),
                )
                .otherwise(col(BIN_START) + col(BIN_STEP) * index_float.clone())
                .map_err(AvengerChartError::DataFusionError)?,
            )
            .map_err(AvengerChartError::DataFusionError)?,
        )
        .map_err(AvengerChartError::DataFusionError)?;

    dataframe = dataframe
        .with_column(BIN_INDEX_FLOAT, index_float)
        .map_err(AvengerChartError::DataFusionError)?
        .with_column(&payload.start_name, bin_start)
        .map_err(AvengerChartError::DataFusionError)?;
    let index_expr = when(
        col(&payload.start_name).is_null(),
        lit(ScalarValue::Int64(None)),
    )
    .otherwise(col(BIN_INDEX_FLOAT).cast_to(&DataType::Int64, dataframe.schema())?)
    .map_err(AvengerChartError::DataFusionError)?;
    dataframe = dataframe
        .with_column(&payload.index_name, index_expr)
        .map_err(AvengerChartError::DataFusionError)?
        .with_column(&payload.end_name, col(&payload.start_name) + col(BIN_STEP))
        .map_err(AvengerChartError::DataFusionError)?;

    let mut projection = original_columns
        .iter()
        .map(|name| col(name))
        .collect::<Vec<_>>();
    projection.push(col(&payload.start_name));
    projection.push(col(&payload.end_name));
    projection.push(col(&payload.index_name));
    dataframe
        .select(projection)
        .map_err(AvengerChartError::DataFusionError)
}
