use async_trait::async_trait;
use avenger_chart_cartesian::CartesianAxis;
use avenger_chart_core::{
    AvengerChartError, ChannelValue, CompiledDataTransform, DataTransform,
    DataTransformCompileContext, DataTransformExecutionContext, DataTransformResult,
    DefaultLogicalExprNodeExt, DerivedScalarMap, ScaleChannelValue, SerializableExpr,
    derived_scalar,
};
use datafusion::{
    arrow::datatypes::DataType,
    common::ScalarValue,
    dataframe::DataFrame,
    functions::expr_fn::floor,
    functions_aggregate::expr_fn::{avg, count, max, min, sum},
    logical_expr::{
        Expr, ExprSchemable, WindowFrame, WindowFrameBound, WindowFrameUnits,
        WindowFunctionDefinition, col,
        expr::{Sort, WindowFunction},
        expr_fn::scalar_subquery,
        lit, when,
    },
    prelude::named_struct,
};
use datafusion_functions_aggregate::{
    min_max::{max_udaf, min_udaf},
    sum::sum_udaf,
};
use datafusion_proto::protobuf::LogicalExprNode;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledAggregateTransform {
    pub group_by: Vec<AggregateGroupKeySpec>,
    pub measures: Vec<AggregateMeasureSpec>,
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AggregateGroupKeySpec {
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub expr: LogicalExprNode,
    pub alias: Option<String>,
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AggregateMeasureSpec {
    pub name: String,
    pub op: AggregateOp,
    #[serde_as(as = "Option<FromInto<SerializableExpr>>")]
    pub expr: Option<LogicalExprNode>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AggregateOp {
    Sum,
    Count,
    Mean,
    Min,
    Max,
}

#[derive(Clone, Debug, Default)]
pub struct Aggregate {
    group_by: Vec<AggregateGroupKeySpec>,
    measures: Vec<AggregateMeasureSpec>,
}

impl Aggregate {
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
                expr: expr_node(expr, "aggregate group_by expression"),
                alias,
            }
        }));
        self
    }

    pub fn group_by_as(mut self, alias: impl Into<String>, expr: Expr) -> Self {
        self.group_by.push(AggregateGroupKeySpec {
            expr: expr_node(expr, "aggregate group_by expression"),
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
            expr: expr.map(|expr| expr_node(expr, "aggregate measure expression")),
        });
        self
    }
}

impl DataTransform for Aggregate {
    type Output = AggregateOutput;

    fn into_compiled_and_output(
        self,
        _ctx: DataTransformCompileContext,
    ) -> Result<(Box<dyn CompiledDataTransform>, Self::Output), AvengerChartError> {
        let mut names = IndexMap::new();
        for group in &self.group_by {
            if let Some(alias) = &group.alias {
                names.insert(alias.clone(), ());
            }
        }
        for measure in &self.measures {
            names.insert(measure.name.clone(), ());
        }
        let transform = CompiledAggregateTransform {
            group_by: self.group_by,
            measures: self.measures,
        };
        Ok((
            Box::new(transform),
            AggregateOutput {
                names: names.keys().cloned().collect(),
            },
        ))
    }
}

#[derive(Clone, Debug)]
pub struct AggregateOutput {
    names: Vec<String>,
}

impl AggregateOutput {
    pub fn output(&self, name: &str) -> Expr {
        if !self.names.iter().any(|candidate| candidate == name) {
            panic!("Unknown aggregate output '{name}'");
        }
        col(name)
    }
}

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
pub struct CompiledBinTransform {
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub value: LogicalExprNode,
    pub maxbins: usize,
    pub start_name: String,
    pub end_name: String,
    pub index_name: String,
    pub domain_start_scalar_id: String,
    pub domain_end_scalar_id: String,
    pub tick_spacing_scalar_id: String,
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
    pub fn new(value: Expr) -> Self {
        Self {
            value,
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

#[derive(Clone, Debug)]
pub struct Bin {
    value: Expr,
    maxbins: usize,
    name: Option<String>,
}

impl Bin {
    pub fn new(value: Expr) -> Self {
        Self {
            value,
            maxbins: 10,
            name: None,
        }
    }

    pub fn maxbins(mut self, maxbins: usize) -> Self {
        self.maxbins = maxbins;
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
        if self.maxbins == 0 {
            return Err(AvengerChartError::InvalidArgument(
                "Bin::maxbins(...) must be greater than zero".to_string(),
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
            maxbins: self.maxbins,
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
    scope: avenger_chart_core::Sharing,
}

impl BinOutput {
    pub fn start(&self) -> ChannelValue {
        self.position_channel(&self.start_name)
    }

    pub fn end(&self) -> ChannelValue {
        self.position_channel(&self.end_name)
    }

    fn position_channel(&self, column_name: &str) -> ChannelValue {
        let domain_start_scalar_id = self.domain_start_scalar_id.clone();
        let domain_end_scalar_id = self.domain_end_scalar_id.clone();
        let tick_spacing_scalar_id = self.tick_spacing_scalar_id.clone();
        ChannelValue::from(col(column_name))
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

impl StackOutput {
    pub fn start(&self) -> ChannelValue {
        ChannelValue::from(col(&self.start_name))
    }

    pub fn end(&self) -> ChannelValue {
        ChannelValue::from(col(&self.end_name))
    }

    pub fn mid(&self) -> ChannelValue {
        ChannelValue::from((col(&self.start_name) + col(&self.end_name)) / lit(2.0))
    }

    pub fn value(&self) -> Expr {
        col(self.value_name.as_deref().unwrap_or(&self.end_name))
    }
}

#[typetag::serde(name = "aggregate")]
#[async_trait]
impl CompiledDataTransform for CompiledAggregateTransform {
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
            {
                let mut names = Vec::new();
                for group in &self.group_by {
                    if let Some(alias) = &group.alias {
                        if !group_alias_is_identity(group, alias, ctx.session_context)? {
                            names.push(alias.as_str());
                        }
                    }
                }
                for measure in &self.measures {
                    names.push(measure.name.as_str());
                }
                names
            },
        )?;

        let group_exprs = self
            .group_by
            .iter()
            .map(|group| {
                let expr = group.expr.to_default_expr(ctx.session_context)?;
                Ok(match &group.alias {
                    Some(alias) => expr.alias(alias),
                    None => expr,
                })
            })
            .collect::<Result<Vec<_>, AvengerChartError>>()?;
        let agg_exprs = self
            .measures
            .iter()
            .map(|measure| aggregate_expr(measure, ctx.session_context))
            .collect::<Result<Vec<_>, _>>()?;
        let dataframe = dataframe
            .aggregate(group_exprs, agg_exprs)
            .map_err(AvengerChartError::DataFusionError)?;
        Ok(DataTransformResult::dataframe(dataframe))
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

#[typetag::serde(name = "bin")]
#[async_trait]
impl CompiledDataTransform for CompiledBinTransform {
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
                self.index_name.as_str(),
            ],
        )?;

        let derived_scalars = bin_derived_scalars(dataframe.clone(), self, ctx.session_context)?;
        let dataframe = apply_bin(dataframe, self, ctx.session_context)?;
        Ok(DataTransformResult {
            dataframe,
            derived_scalars,
        })
    }
}

fn expr_node(expr: Expr, label: &str) -> LogicalExprNode {
    LogicalExprNode::from_default_expr(expr).expect(label)
}

fn simple_column_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Column(column) => Some(column.name.clone()),
        _ => None,
    }
}

fn sanitize_output_name(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let out = out.trim_matches('_');
    if out.is_empty() {
        "value_stack".to_string()
    } else {
        out.to_string()
    }
}

fn aggregate_expr(
    measure: &AggregateMeasureSpec,
    ctx: &datafusion::prelude::SessionContext,
) -> Result<Expr, AvengerChartError> {
    let expr = match measure.op {
        AggregateOp::Count => count(lit(1)),
        AggregateOp::Sum => sum(required_measure_expr(measure, ctx)?),
        AggregateOp::Mean => avg(required_measure_expr(measure, ctx)?),
        AggregateOp::Min => min(required_measure_expr(measure, ctx)?),
        AggregateOp::Max => max(required_measure_expr(measure, ctx)?),
    };
    Ok(expr.alias(&measure.name))
}

fn required_measure_expr(
    measure: &AggregateMeasureSpec,
    ctx: &datafusion::prelude::SessionContext,
) -> Result<Expr, AvengerChartError> {
    let Some(expr) = &measure.expr else {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Aggregate measure '{}' requires an input expression",
            measure.name
        )));
    };
    expr.to_default_expr(ctx)
}

fn validate_output_names<'a>(
    existing: impl IntoIterator<Item = &'a String>,
    proposed: impl IntoIterator<Item = &'a str>,
) -> Result<(), AvengerChartError> {
    let existing = existing.into_iter().collect::<Vec<_>>();
    let mut seen = IndexMap::<&str, ()>::new();
    for name in proposed {
        if name.is_empty() || name.starts_with("__unused") {
            continue;
        }
        if existing.iter().any(|field| field.as_str() == name) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Data transform output name '{name}' conflicts with an input column; choose a different output name"
            )));
        }
        if seen.insert(name, ()).is_some() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Data transform output name '{name}' is duplicated"
            )));
        }
    }
    Ok(())
}

fn group_alias_is_identity(
    group: &AggregateGroupKeySpec,
    alias: &str,
    ctx: &datafusion::prelude::SessionContext,
) -> Result<bool, AvengerChartError> {
    Ok(simple_column_name(&group.expr.to_default_expr(ctx)?).as_deref() == Some(alias))
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

fn window_min(
    arg: Expr,
    partition_by: Vec<Expr>,
    order_by: Vec<Sort>,
    window_frame: WindowFrame,
) -> Expr {
    aggregate_window_expr(
        WindowFunctionDefinition::AggregateUDF(min_udaf()),
        arg,
        partition_by,
        order_by,
        window_frame,
    )
}

fn bin_step_expr(
    min_expr: Expr,
    max_expr: Expr,
    maxbins: usize,
) -> Result<Expr, AvengerChartError> {
    let span = max_expr - min_expr;
    when(span.clone().eq(lit(0.0)), lit(1.0))
        .otherwise(span / lit(maxbins as f64))
        .map_err(AvengerChartError::DataFusionError)
}

fn bin_stop_expr(
    min_expr: Expr,
    max_expr: Expr,
    maxbins: usize,
) -> Result<Expr, AvengerChartError> {
    Ok(min_expr.clone() + bin_step_expr(min_expr, max_expr, maxbins)? * lit(maxbins as f64))
}

fn bin_extent_dataframe(
    dataframe: DataFrame,
    value_expr: Expr,
) -> Result<DataFrame, AvengerChartError> {
    dataframe
        .aggregate(
            Vec::<Expr>::new(),
            vec![
                min(value_expr.clone()).alias("__avenger_bin_extent_min"),
                max(value_expr).alias("__avenger_bin_extent_max"),
            ],
        )
        .map_err(AvengerChartError::DataFusionError)
}

fn scalar_from_dataframe(dataframe: DataFrame, expr: Expr) -> Result<Expr, AvengerChartError> {
    let dataframe = dataframe
        .select(vec![expr.alias("__avenger_bin_scalar")])
        .map_err(AvengerChartError::DataFusionError)?;
    Ok(scalar_subquery(Arc::new(dataframe.into_unoptimized_plan())))
}

fn bin_derived_scalars(
    dataframe: DataFrame,
    payload: &CompiledBinTransform,
    ctx: &datafusion::prelude::SessionContext,
) -> Result<DerivedScalarMap, AvengerChartError> {
    let value_expr = payload.value.to_default_expr(ctx)?;
    let value_expr = value_expr.cast_to(&DataType::Float64, dataframe.schema())?;
    let extent_df = bin_extent_dataframe(dataframe, value_expr)?;
    let min_expr = col("__avenger_bin_extent_min");
    let max_expr = col("__avenger_bin_extent_max");
    let step_expr = bin_step_expr(min_expr.clone(), max_expr.clone(), payload.maxbins)?;
    let stop_expr = bin_stop_expr(min_expr.clone(), max_expr.clone(), payload.maxbins)?;

    let mut derived_scalars = DerivedScalarMap::new();
    derived_scalars.insert(
        payload.domain_start_scalar_id.clone(),
        scalar_from_dataframe(extent_df.clone(), min_expr.clone())?,
    );
    derived_scalars.insert(
        payload.domain_end_scalar_id.clone(),
        scalar_from_dataframe(extent_df.clone(), stop_expr)?,
    );
    derived_scalars.insert(
        payload.tick_spacing_scalar_id.clone(),
        scalar_from_dataframe(
            extent_df,
            named_struct(vec![lit("start"), min_expr, lit("step"), step_expr]),
        )?,
    );
    Ok(derived_scalars)
}

fn apply_bin(
    dataframe: DataFrame,
    payload: &CompiledBinTransform,
    ctx: &datafusion::prelude::SessionContext,
) -> Result<DataFrame, AvengerChartError> {
    let original_columns = dataframe
        .schema()
        .fields()
        .iter()
        .map(|field| field.name().clone())
        .collect::<Vec<_>>();
    let value_expr = payload.value.to_default_expr(ctx)?;
    let value_expr = value_expr.cast_to(&DataType::Float64, dataframe.schema())?;
    let whole_partition_frame = WindowFrame::new_bounds(
        WindowFrameUnits::Rows,
        WindowFrameBound::Preceding(ScalarValue::UInt64(None)),
        WindowFrameBound::Following(ScalarValue::UInt64(None)),
    );

    let mut df = dataframe
        .with_column("__avenger_bin_value", value_expr)?
        .with_column(
            "__avenger_bin_min",
            window_min(
                col("__avenger_bin_value"),
                Vec::new(),
                Vec::new(),
                whole_partition_frame.clone(),
            ),
        )?
        .with_column(
            "__avenger_bin_max",
            window_max(
                col("__avenger_bin_value"),
                Vec::new(),
                Vec::new(),
                whole_partition_frame,
            ),
        )?
        .with_column(
            "__avenger_bin_step",
            bin_step_expr(
                col("__avenger_bin_min"),
                col("__avenger_bin_max"),
                payload.maxbins,
            )?,
        )?;

    let raw_index =
        floor((col("__avenger_bin_value") - col("__avenger_bin_min")) / col("__avenger_bin_step"));
    let max_index = payload.maxbins.saturating_sub(1) as f64;
    let clamped_index = when(raw_index.clone().lt(lit(0.0)), lit(0.0))
        .otherwise(
            when(raw_index.clone().gt(lit(max_index)), lit(max_index))
                .otherwise(raw_index)
                .map_err(AvengerChartError::DataFusionError)?,
        )
        .map_err(AvengerChartError::DataFusionError)?;

    df = df.with_column("__avenger_bin_index_float", clamped_index)?;
    let index_expr = col("__avenger_bin_index_float").cast_to(&DataType::Int64, df.schema())?;
    df = df
        .with_column(&payload.index_name, index_expr)?
        .with_column(
            &payload.start_name,
            col("__avenger_bin_min") + col("__avenger_bin_index_float") * col("__avenger_bin_step"),
        )?
        .with_column(
            &payload.end_name,
            col(&payload.start_name) + col("__avenger_bin_step"),
        )?;

    let mut projection = original_columns
        .iter()
        .map(|name| col(name))
        .collect::<Vec<_>>();
    projection.push(col(&payload.start_name));
    projection.push(col(&payload.end_name));
    projection.push(col(&payload.index_name));
    df.select(projection)
        .map_err(AvengerChartError::DataFusionError)
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

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::{
        array::{Array, Float64Array, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    };
    use avenger_chart_core::{DataTransformStage, Sharing, collect_derived_scalar_ids};
    use datafusion::prelude::SessionContext;
    use std::sync::Arc;

    fn sample_dataframe(ctx: &SessionContext) -> DataFrame {
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("category", DataType::Utf8, false),
                Field::new("series", DataType::Utf8, false),
                Field::new("value", DataType::Float64, false),
            ])),
            vec![
                Arc::new(StringArray::from(vec!["A", "A", "A", "B"])) as _,
                Arc::new(StringArray::from(vec!["s1", "s2", "s3", "s1"])) as _,
                Arc::new(Float64Array::from(vec![1.0, 2.0, -3.0, 4.0])) as _,
            ],
        )
        .unwrap();
        ctx.read_batch(batch).unwrap()
    }

    fn bin_dataframe(ctx: &SessionContext) -> DataFrame {
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new(
                "value",
                DataType::Float64,
                true,
            )])),
            vec![Arc::new(Float64Array::from(vec![
                Some(1.0),
                Some(2.0),
                Some(3.0),
                Some(4.0),
                Some(5.0),
                None,
            ])) as _],
        )
        .unwrap();
        ctx.read_batch(batch).unwrap()
    }

    async fn transformed_batches(
        ctx: &SessionContext,
        dataframe: DataFrame,
        transforms: Vec<DataTransformStage>,
    ) -> Vec<RecordBatch> {
        avenger_chart_core::apply_compiled_data_transforms(
            dataframe,
            &transforms,
            &DataTransformExecutionContext {
                session_context: ctx,
            },
        )
        .await
        .unwrap()
        .dataframe
        .collect()
        .await
        .unwrap()
    }

    fn compile_transform<T: DataTransform>(transform: T) -> (DataTransformStage, T::Output) {
        compile_transform_with_scope(Sharing::Free, transform)
    }

    fn compile_transform_with_scope<T: DataTransform>(
        scope: Sharing,
        transform: T,
    ) -> (DataTransformStage, T::Output) {
        let (compiled, output) = transform
            .into_compiled_and_output(DataTransformCompileContext::new(scope))
            .unwrap();
        (DataTransformStage::new(scope, compiled), output)
    }

    fn bin_rows(batch: &RecordBatch) -> Vec<(Option<f64>, Option<f64>, Option<f64>, Option<i64>)> {
        let value = batch
            .column_by_name("value")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        let start = batch
            .column_by_name("value_bin_start")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        let end = batch
            .column_by_name("value_bin_end")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        let index = batch
            .column_by_name("value_bin_index")
            .unwrap()
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        (0..batch.num_rows())
            .map(|row| {
                (
                    (!value.is_null(row)).then(|| value.value(row)),
                    (!start.is_null(row)).then(|| start.value(row)),
                    (!end.is_null(row)).then(|| end.value(row)),
                    (!index.is_null(row)).then(|| index.value(row)),
                )
            })
            .collect()
    }

    fn bin_rows_from_batches(
        batches: &[RecordBatch],
    ) -> Vec<(Option<f64>, Option<f64>, Option<f64>, Option<i64>)> {
        batches.iter().flat_map(bin_rows).collect()
    }

    fn stack_rows(batch: &RecordBatch) -> Vec<(String, String, f64, f64)> {
        let category = batch
            .column_by_name("category")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let series = batch
            .column_by_name("series")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let start = batch
            .column_by_name("value_stack_start")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        let end = batch
            .column_by_name("value_stack_end")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        (0..batch.num_rows())
            .map(|index| {
                (
                    category.value(index).to_string(),
                    series.value(index).to_string(),
                    start.value(index),
                    end.value(index),
                )
            })
            .collect()
    }

    fn stack_rows_from_batches(batches: &[RecordBatch]) -> Vec<(String, String, f64, f64)> {
        batches.iter().flat_map(stack_rows).collect()
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() <= 1e-9,
            "expected {expected}, got {actual}"
        );
    }

    fn assert_stack_rows(
        mut actual: Vec<(String, String, f64, f64)>,
        expected: &[(&str, &str, f64, f64)],
    ) {
        actual.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(expected) {
            assert_eq!(actual.0, expected.0);
            assert_eq!(actual.1, expected.1);
            assert_close(actual.2, expected.2);
            assert_close(actual.3, expected.3);
        }
    }

    #[tokio::test]
    async fn aggregate_groups_and_sums() {
        let ctx = SessionContext::new();
        let dataframe = sample_dataframe(&ctx);
        let transform = Aggregate::new()
            .group_by([col("category"), col("series")])
            .sum("total_value", col("value"));
        let (compiled_transform, output) = compile_transform(transform);
        assert_eq!(output.output("total_value").to_string(), "total_value");
        let result = avenger_chart_core::apply_compiled_data_transforms(
            dataframe,
            &[compiled_transform],
            &DataTransformExecutionContext {
                session_context: &ctx,
            },
        )
        .await
        .unwrap();
        let batches = result.dataframe.collect().await.unwrap();
        let rows: usize = batches.iter().map(|batch| batch.num_rows()).sum();
        assert_eq!(rows, 4);
    }

    #[tokio::test]
    async fn bin_output_names_and_derived_scalar_refs() {
        let output = Bin::new(col("value"))
            .maxbins(4)
            .name("custom_bin")
            .into_compiled_and_output(DataTransformCompileContext::new(Sharing::Free))
            .unwrap()
            .1;

        assert_eq!(output.index().to_string(), "custom_bin_index");
        let start = output.start();
        assert!(matches!(start, ChannelValue::Scaled { .. }));
        let scale = start.get_scale_config().expect("scale config");
        let mut ids = Vec::new();
        for expr in scale.all_exprs(&SessionContext::new()) {
            ids.extend(collect_derived_scalar_ids(&expr).unwrap());
        }
        let axis = start.get_axis_config().expect("axis config");
        for expr in axis.all_exprs(&SessionContext::new()) {
            ids.extend(collect_derived_scalar_ids(&expr).unwrap());
        }
        assert!(
            ids.contains(&"custom_bin_domain_start".to_string()),
            "{ids:?}"
        );
        assert!(
            ids.contains(&"custom_bin_domain_end".to_string()),
            "{ids:?}"
        );
        assert!(
            ids.contains(&"custom_bin_tick_spacing".to_string()),
            "{ids:?}"
        );
    }

    #[tokio::test]
    async fn bin_output_uses_stage_scope_as_default_scale_sharing() {
        let output = Bin::new(col("value"))
            .maxbins(4)
            .into_compiled_and_output(DataTransformCompileContext::new(Sharing::Level(1)))
            .unwrap()
            .1;

        let start = output.start();
        let end = output.end();
        assert_eq!(start.get_share_mode(), Some(Sharing::Level(1)));
        assert_eq!(start.get_transform_scope(), Some(Sharing::Level(1)));
        assert_eq!(end.get_share_mode(), Some(Sharing::Level(1)));
        assert_eq!(end.get_transform_scope(), Some(Sharing::Level(1)));
    }

    #[tokio::test]
    async fn bin_maxbins_zero_errors() {
        let err = match Bin::new(col("value"))
            .maxbins(0)
            .into_compiled_and_output(DataTransformCompileContext::new(Sharing::Free))
        {
            Ok(_) => panic!("maxbins zero should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("maxbins"), "{err}");
    }

    #[tokio::test]
    async fn bin_exact_count_clamps_max_and_preserves_nulls() {
        let ctx = SessionContext::new();
        let dataframe = bin_dataframe(&ctx);
        let (compiled_transform, output) = compile_transform(Bin::new(col("value")).maxbins(4));
        assert_eq!(output.index().to_string(), "value_bin_index");

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let rows = bin_rows_from_batches(&batches);
        assert_eq!(
            rows,
            vec![
                (Some(1.0), Some(1.0), Some(2.0), Some(0)),
                (Some(2.0), Some(2.0), Some(3.0), Some(1)),
                (Some(3.0), Some(3.0), Some(4.0), Some(2)),
                (Some(4.0), Some(4.0), Some(5.0), Some(3)),
                (Some(5.0), Some(4.0), Some(5.0), Some(3)),
                (None, None, None, None),
            ]
        );
    }

    #[tokio::test]
    async fn bin_single_value_uses_fallback_step() {
        let ctx = SessionContext::new();
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new(
                "value",
                DataType::Float64,
                false,
            )])),
            vec![Arc::new(Float64Array::from(vec![7.0, 7.0])) as _],
        )
        .unwrap();
        let dataframe = ctx.read_batch(batch).unwrap();
        let (compiled_transform, _) = compile_transform(Bin::new(col("value")).maxbins(3));

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let rows = bin_rows_from_batches(&batches);
        assert_eq!(
            rows,
            vec![
                (Some(7.0), Some(7.0), Some(8.0), Some(0)),
                (Some(7.0), Some(7.0), Some(8.0), Some(0)),
            ]
        );
    }

    #[tokio::test]
    async fn bin_returns_derived_scalars() {
        let ctx = SessionContext::new();
        let dataframe = bin_dataframe(&ctx);
        let (compiled_transform, _) = compile_transform(Bin::new(col("value")).maxbins(4));
        let result = avenger_chart_core::apply_compiled_data_transforms(
            dataframe,
            &[compiled_transform],
            &DataTransformExecutionContext {
                session_context: &ctx,
            },
        )
        .await
        .unwrap();
        assert!(
            result
                .derived_scalars
                .contains_key("value_bin_domain_start")
        );
        assert!(result.derived_scalars.contains_key("value_bin_domain_end"));
        assert!(
            result
                .derived_scalars
                .contains_key("value_bin_tick_spacing")
        );
    }

    #[tokio::test]
    async fn stack_zero_adds_start_and_end_columns() {
        let ctx = SessionContext::new();
        let dataframe = sample_dataframe(&ctx);
        let transform = Stack::new(col("value"))
            .group_by([col("category")])
            .sort_by_exprs([col("series")])
            .name("value_stack");
        let (compiled_transform, output) = compile_transform(transform);
        assert!(matches!(output.start(), ChannelValue::Scaled { .. }));
        let result = avenger_chart_core::apply_compiled_data_transforms(
            dataframe,
            &[compiled_transform],
            &DataTransformExecutionContext {
                session_context: &ctx,
            },
        )
        .await
        .unwrap();
        let schema = result.dataframe.schema();
        assert!(schema.field_with_name(None, "value_stack_start").is_ok());
        assert!(schema.field_with_name(None, "value_stack_end").is_ok());
    }

    #[tokio::test]
    async fn stack_zero_computes_positive_and_negative_extents() {
        let ctx = SessionContext::new();
        let dataframe = sample_dataframe(&ctx);
        let (compiled_transform, _) = compile_transform(
            Stack::new(col("value"))
                .group_by([col("category")])
                .sort_by_exprs([col("series")])
                .name("value_stack"),
        );
        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        assert_stack_rows(
            stack_rows_from_batches(&batches),
            &[
                ("A", "s1", 2.0, 3.0),
                ("A", "s2", 0.0, 2.0),
                ("A", "s3", 0.0, -3.0),
                ("B", "s1", 0.0, 4.0),
            ],
        );
    }

    #[tokio::test]
    async fn stack_normalize_uses_absolute_group_totals() {
        let ctx = SessionContext::new();
        let dataframe = sample_dataframe(&ctx);
        let (compiled_transform, _) = compile_transform(
            Stack::new(col("value"))
                .group_by([col("category")])
                .sort_by_exprs([col("series")])
                .offset(StackOffset::Normalize)
                .name("value_stack"),
        );
        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        assert_stack_rows(
            stack_rows_from_batches(&batches),
            &[
                ("A", "s1", 5.0 / 6.0, 1.0),
                ("A", "s2", 3.0 / 6.0, 5.0 / 6.0),
                ("A", "s3", 0.0, 3.0 / 6.0),
                ("B", "s1", 0.0, 1.0),
            ],
        );
    }

    #[tokio::test]
    async fn stack_center_offsets_smaller_groups() {
        let ctx = SessionContext::new();
        let dataframe = sample_dataframe(&ctx);
        let (compiled_transform, _) = compile_transform(
            Stack::new(col("value"))
                .group_by([col("category")])
                .sort_by_exprs([col("series")])
                .offset(StackOffset::Center)
                .name("value_stack"),
        );
        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        assert_stack_rows(
            stack_rows_from_batches(&batches),
            &[
                ("A", "s1", 5.0, 6.0),
                ("A", "s2", 3.0, 5.0),
                ("A", "s3", 0.0, 3.0),
                ("B", "s1", 1.0, 5.0),
            ],
        );
    }

    #[tokio::test]
    async fn aggregate_output_feeds_stack_transform() {
        let ctx = SessionContext::new();
        let dataframe = sample_dataframe(&ctx);
        let (aggregate_transform, aggregate) = compile_transform(
            Aggregate::new()
                .group_by([col("category"), col("series")])
                .sum("total_value", col("value")),
        );
        let (stack_transform, _) = compile_transform(
            Stack::new(aggregate.output("total_value"))
                .group_by([col("category")])
                .sort_by_exprs([col("series")])
                .name("value_stack"),
        );
        let batches =
            transformed_batches(&ctx, dataframe, vec![aggregate_transform, stack_transform]).await;
        assert_stack_rows(
            stack_rows_from_batches(&batches),
            &[
                ("A", "s1", 2.0, 3.0),
                ("A", "s2", 0.0, 2.0),
                ("A", "s3", 0.0, -3.0),
                ("B", "s1", 0.0, 4.0),
            ],
        );
    }
}
