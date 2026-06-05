use async_trait::async_trait;
use avenger_chart_cartesian::CartesianAxis;
use avenger_chart_core::{
    AvengerChartError, ChannelValue, CompiledDataTransform, DataTransform,
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
    functions_aggregate::expr_fn::{avg, count, max, min, sum},
    logical_expr::{
        Expr, ExprSchemable, JoinType, WindowFrame, WindowFrameBound, WindowFrameUnits,
        WindowFunctionDefinition, col,
        expr::{Sort, WindowFunction},
        expr_fn::scalar_subquery,
        lit, when,
    },
    prelude::named_struct,
};
use datafusion_functions_aggregate::{min_max::max_udaf, sum::sum_udaf};
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
    pub fn new(value: Expr) -> Self {
        Self {
            value,
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
        bin_dataframe_from_values(
            ctx,
            vec![Some(1.0), Some(2.0), Some(3.0), Some(4.0), Some(5.0), None],
        )
    }

    fn bin_dataframe_from_values(ctx: &SessionContext, values: Vec<Option<f64>>) -> DataFrame {
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new(
                "value",
                DataType::Float64,
                true,
            )])),
            vec![Arc::new(Float64Array::from(values)) as _],
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
        let (compiled_transform, output) =
            compile_transform(Bin::new(col("value")).maxbins(4).exact());
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
    async fn bin_nice_default_uses_friendly_edges() {
        let ctx = SessionContext::new();
        let dataframe =
            bin_dataframe_from_values(&ctx, vec![Some(0.2), Some(1.4), Some(2.8), Some(9.7), None]);
        let (compiled_transform, _) = compile_transform(Bin::new(col("value")).maxbins(5));

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let rows = bin_rows_from_batches(&batches);
        assert_eq!(
            rows,
            vec![
                (Some(0.2), Some(0.0), Some(2.0), Some(0)),
                (Some(1.4), Some(0.0), Some(2.0), Some(0)),
                (Some(2.8), Some(2.0), Some(4.0), Some(1)),
                (Some(9.7), Some(8.0), Some(10.0), Some(4)),
                (None, None, None, None),
            ]
        );
    }

    #[tokio::test]
    async fn bin_exact_preserves_messy_edges() {
        let ctx = SessionContext::new();
        let dataframe =
            bin_dataframe_from_values(&ctx, vec![Some(0.2), Some(1.4), Some(2.8), Some(9.7), None]);
        let (compiled_transform, _) = compile_transform(Bin::new(col("value")).maxbins(5).exact());

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let rows = bin_rows_from_batches(&batches);
        assert_eq!(
            rows,
            vec![
                (Some(0.2), Some(0.2), Some(2.1), Some(0)),
                (Some(1.4), Some(0.2), Some(2.1), Some(0)),
                (Some(2.8), Some(2.1), Some(4.0), Some(1)),
                (Some(9.7), Some(7.8), Some(9.7), Some(4)),
                (None, None, None, None),
            ]
        );
    }

    #[tokio::test]
    async fn bin_steps_choose_smallest_step_under_maxbins() {
        let ctx = SessionContext::new();
        let dataframe = bin_dataframe_from_values(&ctx, vec![Some(0.2), Some(2.8), Some(9.7)]);
        let (compiled_transform, _) =
            compile_transform(Bin::new(col("value")).maxbins(4).steps([1.0, 2.0, 5.0]));

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let rows = bin_rows_from_batches(&batches);
        assert_eq!(
            rows,
            vec![
                (Some(0.2), Some(0.0), Some(5.0), Some(0)),
                (Some(2.8), Some(0.0), Some(5.0), Some(0)),
                (Some(9.7), Some(5.0), Some(10.0), Some(1)),
            ]
        );
    }

    #[tokio::test]
    async fn bin_minstep_prevents_over_refinement() {
        let ctx = SessionContext::new();
        let dataframe = bin_dataframe_from_values(&ctx, vec![Some(0.2), Some(2.8), Some(9.7)]);
        let (compiled_transform, _) =
            compile_transform(Bin::new(col("value")).maxbins(10).minstep(5.0));

        let batches = transformed_batches(&ctx, dataframe, vec![compiled_transform]).await;
        let rows = bin_rows_from_batches(&batches);
        assert_eq!(
            rows,
            vec![
                (Some(0.2), Some(0.0), Some(5.0), Some(0)),
                (Some(2.8), Some(0.0), Some(5.0), Some(0)),
                (Some(9.7), Some(5.0), Some(10.0), Some(1)),
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
