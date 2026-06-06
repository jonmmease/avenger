use crate::common::{
    expr_node, map_expr_node, map_optional_expr_node, simple_column_name,
    validate_unique_generated_names,
};
use async_trait::async_trait;
use avenger_chart_core::{
    AvengerChartError, CompiledDataTransform, DataTransform, DataTransformCompileContext,
    DataTransformExecutionContext, DataTransformResult, DefaultLogicalExprNodeExt, IntoExpr,
    SerializableExpr, eval_to_scalars, params_to_datafusion,
};
use datafusion::{
    arrow::{
        array::{Array, ArrayRef, Float64Array},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    dataframe::DataFrame,
    logical_expr::{Expr, ExprSchemable, col, lit},
};
use datafusion_proto::protobuf::LogicalExprNode;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};
use std::{f64::consts::PI, sync::Arc};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum KdeResolve {
    #[default]
    Independent,
    Shared,
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledKdeTransform {
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub value: LogicalExprNode,
    pub group_by: Vec<String>,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub bandwidth: LogicalExprNode,
    pub counts: bool,
    pub cumulative: bool,
    #[serde_as(as = "Option<FromInto<SerializableExpr>>")]
    pub extent_start: Option<LogicalExprNode>,
    #[serde_as(as = "Option<FromInto<SerializableExpr>>")]
    pub extent_stop: Option<LogicalExprNode>,
    pub resolve: KdeResolve,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub steps: LogicalExprNode,
    pub value_name: String,
    pub density_name: String,
}

#[derive(Clone, Debug)]
pub struct Kde {
    value: Expr,
    group_by: Vec<Expr>,
    bandwidth: Expr,
    counts: bool,
    cumulative: bool,
    extent: Option<(Expr, Expr)>,
    resolve: KdeResolve,
    steps: Expr,
    value_name: String,
    density_name: String,
}

impl Kde {
    pub fn new(value: impl IntoExpr) -> Self {
        Self {
            value: value.into_expr(),
            group_by: Vec::new(),
            bandwidth: lit(0.0),
            counts: false,
            cumulative: false,
            extent: None,
            resolve: KdeResolve::Independent,
            steps: lit(200.0),
            value_name: "value".to_string(),
            density_name: "density".to_string(),
        }
    }

    pub fn group_by<I, E>(mut self, group_by: I) -> Self
    where
        I: IntoIterator<Item = E>,
        E: IntoExpr,
    {
        self.group_by = group_by.into_iter().map(IntoExpr::into_expr).collect();
        self
    }

    pub fn bandwidth(mut self, bandwidth: impl IntoExpr) -> Self {
        self.bandwidth = bandwidth.into_expr();
        self
    }

    pub fn counts(mut self, counts: bool) -> Self {
        self.counts = counts;
        self
    }

    pub fn cumulative(mut self, cumulative: bool) -> Self {
        self.cumulative = cumulative;
        self
    }

    pub fn extent(mut self, start: impl IntoExpr, stop: impl IntoExpr) -> Self {
        self.extent = Some((start.into_expr(), stop.into_expr()));
        self
    }

    pub fn resolve(mut self, resolve: KdeResolve) -> Self {
        self.resolve = resolve;
        self
    }

    pub fn steps(mut self, steps: impl IntoExpr) -> Self {
        self.steps = steps.into_expr();
        self
    }

    pub fn as_fields(
        mut self,
        value_name: impl Into<String>,
        density_name: impl Into<String>,
    ) -> Self {
        self.value_name = value_name.into();
        self.density_name = density_name.into();
        self
    }
}

impl DataTransform for Kde {
    type Output = KdeOutput;

    fn into_compiled_and_output(
        self,
        _ctx: DataTransformCompileContext,
    ) -> Result<(Box<dyn CompiledDataTransform>, Self::Output), AvengerChartError> {
        if avenger_chart_core::contains_aggregate(&self.value) {
            return Err(AvengerChartError::InvalidArgument(
                "Kde::new(...) does not accept aggregate expressions; use Aggregate before Kde"
                    .to_string(),
            ));
        }

        let mut group_by = Vec::new();
        for expr in &self.group_by {
            let Some(name) = simple_column_name(expr) else {
                return Err(AvengerChartError::InvalidArgument(
                    "Kde::group_by(...) only accepts simple column references in v1; use Calculate first for computed grouping"
                        .to_string(),
                ));
            };
            group_by.push(name);
        }

        validate_unique_generated_names([self.value_name.as_str(), self.density_name.as_str()])?;
        for group_name in &group_by {
            if group_name == &self.value_name || group_name == &self.density_name {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Kde output name conflicts with group column '{group_name}'; use Kde::as_fields(...) to choose different output names"
                )));
            }
        }

        let transform = CompiledKdeTransform {
            value: expr_node(self.value, "kde value expression"),
            group_by,
            bandwidth: expr_node(self.bandwidth, "kde bandwidth expression"),
            counts: self.counts,
            cumulative: self.cumulative,
            extent_start: self
                .extent
                .as_ref()
                .map(|(start, _)| expr_node(start.clone(), "kde extent start expression")),
            extent_stop: self
                .extent
                .map(|(_, stop)| expr_node(stop, "kde extent stop expression")),
            resolve: self.resolve,
            steps: expr_node(self.steps, "kde steps expression"),
            value_name: self.value_name.clone(),
            density_name: self.density_name.clone(),
        };

        Ok((
            Box::new(transform),
            KdeOutput {
                value_name: self.value_name,
                density_name: self.density_name,
            },
        ))
    }
}

#[derive(Clone, Debug)]
pub struct KdeOutput {
    value_name: String,
    density_name: String,
}

impl KdeOutput {
    pub fn value(&self) -> Expr {
        col(&self.value_name)
    }

    pub fn density(&self) -> Expr {
        col(&self.density_name)
    }
}

#[typetag::serde(name = "kde")]
#[async_trait]
impl CompiledDataTransform for CompiledKdeTransform {
    fn clone_box(&self) -> Box<dyn CompiledDataTransform> {
        Box::new(self.clone())
    }

    fn map_exprs(
        &self,
        f: &mut dyn FnMut(Expr) -> Result<Expr, AvengerChartError>,
    ) -> Result<Box<dyn CompiledDataTransform>, AvengerChartError> {
        Ok(Box::new(Self {
            value: map_expr_node(&self.value, f)?,
            group_by: self.group_by.clone(),
            bandwidth: map_expr_node(&self.bandwidth, f)?,
            counts: self.counts,
            cumulative: self.cumulative,
            extent_start: map_optional_expr_node(&self.extent_start, f)?,
            extent_stop: map_optional_expr_node(&self.extent_stop, f)?,
            resolve: self.resolve,
            steps: map_expr_node(&self.steps, f)?,
            value_name: self.value_name.clone(),
            density_name: self.density_name.clone(),
        }))
    }

    async fn apply(
        &self,
        dataframe: DataFrame,
        ctx: &DataTransformExecutionContext<'_>,
    ) -> Result<DataTransformResult, AvengerChartError> {
        let steps = eval_steps(&self.steps, ctx).await?;
        let bandwidth = eval_bandwidth(&self.bandwidth, ctx).await?;
        let extent = eval_extent(self, ctx).await?;
        let group_fields = group_fields(&dataframe, &self.group_by)?;
        let projected = projected_kde_dataframe(dataframe, self, ctx)?;
        let batches = projected
            .collect()
            .await
            .map_err(AvengerChartError::DataFusionError)?;
        let groups = collect_groups(&batches, self.group_by.len())?;
        let batch = kde_record_batch(self, &group_fields, groups, steps, bandwidth, extent)?;
        let dataframe = ctx
            .session_context
            .read_batch(batch)
            .map_err(AvengerChartError::DataFusionError)?;
        Ok(DataTransformResult::dataframe(dataframe))
    }
}

const KDE_VALUE: &str = "__avenger_kde_value";

#[derive(Clone, Debug)]
struct KdeGroup {
    key: Vec<ScalarValue>,
    values: Vec<f64>,
}

fn group_fields(
    dataframe: &DataFrame,
    group_by: &[String],
) -> Result<Vec<Field>, AvengerChartError> {
    let mut fields = Vec::new();
    for name in group_by {
        let field = dataframe
            .schema()
            .field_with_unqualified_name(name)
            .map_err(|_| {
                AvengerChartError::InvalidArgument(format!(
                    "Kde::group_by(...) column '{name}' was not found"
                ))
            })?;
        fields.push(Field::new(
            name,
            field.data_type().clone(),
            field.is_nullable(),
        ));
    }
    Ok(fields)
}

fn projected_kde_dataframe(
    dataframe: DataFrame,
    payload: &CompiledKdeTransform,
    ctx: &DataTransformExecutionContext<'_>,
) -> Result<DataFrame, AvengerChartError> {
    let value_expr = payload.value.to_default_expr(ctx.session_context)?;
    let value_expr = value_expr
        .cast_to(&DataType::Float64, dataframe.schema())
        .map_err(AvengerChartError::DataFusionError)?;
    let mut exprs = payload
        .group_by
        .iter()
        .map(|name| col(name).alias(name))
        .collect::<Vec<_>>();
    exprs.push(value_expr.alias(KDE_VALUE));
    let dataframe = dataframe
        .select(exprs)
        .map_err(AvengerChartError::DataFusionError)?;
    if let Some(params) = params_to_datafusion(ctx.params) {
        dataframe
            .with_param_values(params)
            .map_err(AvengerChartError::DataFusionError)
    } else {
        Ok(dataframe)
    }
}

fn collect_groups(
    batches: &[RecordBatch],
    group_column_count: usize,
) -> Result<Vec<KdeGroup>, AvengerChartError> {
    let mut groups = Vec::<KdeGroup>::new();
    let mut group_index = IndexMap::<Vec<ScalarValue>, usize>::new();
    for batch in batches {
        let value_array = batch
            .column(group_column_count)
            .as_any()
            .downcast_ref::<Float64Array>()
            .ok_or_else(|| {
                AvengerChartError::InternalError(
                    "Kde projected value column was not Float64".to_string(),
                )
            })?;
        for row in 0..batch.num_rows() {
            if value_array.is_null(row) {
                continue;
            }
            let value = value_array.value(row);
            if !value.is_finite() {
                continue;
            }
            let mut key = Vec::with_capacity(group_column_count);
            for column_index in 0..group_column_count {
                key.push(
                    ScalarValue::try_from_array(batch.column(column_index), row)
                        .map_err(AvengerChartError::DataFusionError)?,
                );
            }
            let group_id = if let Some(group_id) = group_index.get(&key) {
                *group_id
            } else {
                let group_id = groups.len();
                group_index.insert(key.clone(), group_id);
                groups.push(KdeGroup {
                    key,
                    values: Vec::new(),
                });
                group_id
            };
            groups[group_id].values.push(value);
        }
    }
    Ok(groups)
}

fn kde_record_batch(
    payload: &CompiledKdeTransform,
    group_fields: &[Field],
    groups: Vec<KdeGroup>,
    steps: usize,
    bandwidth: Option<f64>,
    extent: Option<(f64, f64)>,
) -> Result<RecordBatch, AvengerChartError> {
    let output_groups = groups
        .into_iter()
        .filter(|group| !group.values.is_empty())
        .collect::<Vec<_>>();
    let global_values = output_groups
        .iter()
        .flat_map(|group| group.values.iter().copied())
        .collect::<Vec<_>>();
    let shared_extent = match (payload.resolve, extent, global_values.is_empty()) {
        (_, _, true) => None,
        (KdeResolve::Shared, Some(extent), false) => Some(extent),
        (KdeResolve::Shared, None, false) => Some(data_extent(
            &global_values,
            bandwidth.unwrap_or_else(|| auto_bandwidth(&global_values)),
        )),
        _ => None,
    };

    let mut group_columns = (0..group_fields.len())
        .map(|_| Vec::<ScalarValue>::new())
        .collect::<Vec<_>>();
    let mut sample_values = Vec::<f64>::new();
    let mut density_values = Vec::<f64>::new();

    for group in &output_groups {
        let h = bandwidth.unwrap_or_else(|| auto_bandwidth(&group.values));
        let h = positive_finite_or_one(h);
        let group_extent = match (payload.resolve, shared_extent, extent) {
            (KdeResolve::Shared, Some(extent), _) => extent,
            (KdeResolve::Independent, _, Some(extent)) => extent,
            _ => data_extent(&group.values, h),
        };
        for index in 0..=steps {
            let t = index as f64 / steps as f64;
            let sample = group_extent.0 + (group_extent.1 - group_extent.0) * t;
            for (column_index, scalar) in group.key.iter().enumerate() {
                group_columns[column_index].push(scalar.clone());
            }
            sample_values.push(sample);
            density_values.push(kde_value(
                sample,
                &group.values,
                h,
                payload.counts,
                payload.cumulative,
            ));
        }
    }

    let mut fields = group_fields.to_vec();
    fields.push(Field::new(&payload.value_name, DataType::Float64, false));
    fields.push(Field::new(&payload.density_name, DataType::Float64, false));
    let schema = Arc::new(Schema::new(fields));

    let mut columns = Vec::<ArrayRef>::new();
    for (column_index, field) in group_fields.iter().enumerate() {
        columns.push(scalars_to_array(
            group_columns[column_index].clone(),
            field.data_type(),
        )?);
    }
    columns.push(Arc::new(Float64Array::from(sample_values)) as ArrayRef);
    columns.push(Arc::new(Float64Array::from(density_values)) as ArrayRef);

    RecordBatch::try_new(schema, columns).map_err(AvengerChartError::ArrowError)
}

fn scalars_to_array(
    scalars: Vec<ScalarValue>,
    data_type: &DataType,
) -> Result<ArrayRef, AvengerChartError> {
    if scalars.is_empty() {
        return ScalarValue::try_from(data_type)
            .and_then(|scalar| scalar.to_array_of_size(0))
            .map_err(AvengerChartError::DataFusionError);
    }
    ScalarValue::iter_to_array(scalars.into_iter()).map_err(AvengerChartError::DataFusionError)
}

async fn eval_steps(
    expr: &LogicalExprNode,
    ctx: &DataTransformExecutionContext<'_>,
) -> Result<usize, AvengerChartError> {
    let scalar = eval_config_scalar(expr, "steps", ctx).await?;
    let value = scalar_to_f64(&scalar, "Kde::steps(...)")?;
    if !value.is_finite() || value <= 0.0 || value.fract() != 0.0 {
        return Err(AvengerChartError::InvalidArgument(
            "Kde::steps(...) must evaluate to a positive integer".to_string(),
        ));
    }
    Ok(value as usize)
}

async fn eval_bandwidth(
    expr: &LogicalExprNode,
    ctx: &DataTransformExecutionContext<'_>,
) -> Result<Option<f64>, AvengerChartError> {
    let scalar = eval_config_scalar(expr, "bandwidth", ctx).await?;
    let value = scalar_to_f64(&scalar, "Kde::bandwidth(...)")?;
    if !value.is_finite() || value < 0.0 {
        return Err(AvengerChartError::InvalidArgument(
            "Kde::bandwidth(...) must evaluate to a non-negative finite number".to_string(),
        ));
    }
    Ok((value > 0.0).then_some(value))
}

async fn eval_extent(
    payload: &CompiledKdeTransform,
    ctx: &DataTransformExecutionContext<'_>,
) -> Result<Option<(f64, f64)>, AvengerChartError> {
    let Some(start) = &payload.extent_start else {
        return Ok(None);
    };
    let stop = payload.extent_stop.as_ref().ok_or_else(|| {
        AvengerChartError::InternalError("Kde extent stop expression is missing".to_string())
    })?;
    let start = scalar_to_f64(
        &eval_config_scalar(start, "extent start", ctx).await?,
        "Kde::extent(...) start",
    )?;
    let stop = scalar_to_f64(
        &eval_config_scalar(stop, "extent stop", ctx).await?,
        "Kde::extent(...) stop",
    )?;
    if !start.is_finite() || !stop.is_finite() || stop <= start {
        return Err(AvengerChartError::InvalidArgument(
            "Kde::extent(...) must evaluate to finite start/stop values with stop > start"
                .to_string(),
        ));
    }
    Ok(Some((start, stop)))
}

async fn eval_config_scalar(
    expr: &LogicalExprNode,
    label: &str,
    ctx: &DataTransformExecutionContext<'_>,
) -> Result<ScalarValue, AvengerChartError> {
    let expr = expr.to_default_expr(ctx.session_context)?;
    let params = params_to_datafusion(ctx.params);
    let mut values = eval_to_scalars(vec![expr], Some(ctx.session_context), params.as_ref())
        .await
        .map_err(|err| {
            AvengerChartError::InvalidArgument(format!(
                "Kde::{label}(...) must be a literal or parameter expression; column references are not supported in KDE configuration expressions: {err}"
            ))
        })?;
    values.pop().ok_or_else(|| {
        AvengerChartError::InternalError(format!("Kde::{label}(...) did not produce a scalar"))
    })
}

fn scalar_to_f64(scalar: &ScalarValue, label: &str) -> Result<f64, AvengerChartError> {
    match scalar {
        ScalarValue::Float64(Some(value)) => Ok(*value),
        ScalarValue::Float32(Some(value)) => Ok(*value as f64),
        ScalarValue::Int64(Some(value)) => Ok(*value as f64),
        ScalarValue::Int32(Some(value)) => Ok(*value as f64),
        ScalarValue::Int16(Some(value)) => Ok(*value as f64),
        ScalarValue::Int8(Some(value)) => Ok(*value as f64),
        ScalarValue::UInt64(Some(value)) => Ok(*value as f64),
        ScalarValue::UInt32(Some(value)) => Ok(*value as f64),
        ScalarValue::UInt16(Some(value)) => Ok(*value as f64),
        ScalarValue::UInt8(Some(value)) => Ok(*value as f64),
        _ => Err(AvengerChartError::InvalidArgument(format!(
            "{label} must evaluate to a numeric scalar"
        ))),
    }
}

fn data_extent(values: &[f64], bandwidth: f64) -> (f64, f64) {
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if min.is_finite() && max.is_finite() && max > min {
        (min, max)
    } else {
        let center = if min.is_finite() { min } else { 0.0 };
        let half_span = (bandwidth * 3.0).max(0.5);
        (center - half_span, center + half_span)
    }
}

pub(crate) fn auto_bandwidth(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 1.0;
    }
    let mut sorted = values
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .collect::<Vec<_>>();
    if sorted.is_empty() {
        return 1.0;
    }
    sorted.sort_by(|a, b| a.total_cmp(b));
    let n = sorted.len();
    let stddev = sample_stddev(&sorted);
    let iqr = quantile_sorted(&sorted, 0.75) - quantile_sorted(&sorted, 0.25);
    let robust = iqr / 1.34;
    let scale = positive_min(stddev, robust)
        .or_else(|| positive_value(stddev))
        .or_else(|| positive_value(robust))
        .or_else(|| positive_value(sorted[n - 1] - sorted[0]))
        .unwrap_or(1.0);
    positive_finite_or_one(1.06 * scale * (n as f64).powf(-0.2))
}

fn sample_stddev(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / (values.len() - 1) as f64;
    variance.sqrt()
}

fn quantile_sorted(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    if values.len() == 1 {
        return values[0];
    }
    let index = p.clamp(0.0, 1.0) * (values.len() - 1) as f64;
    let lower = index.floor() as usize;
    let upper = index.ceil() as usize;
    if lower == upper {
        values[lower]
    } else {
        values[lower] + (values[upper] - values[lower]) * (index - lower as f64)
    }
}

fn positive_min(left: f64, right: f64) -> Option<f64> {
    match (positive_value(left), positive_value(right)) {
        (Some(left), Some(right)) => Some(left.min(right)),
        _ => None,
    }
}

fn positive_value(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
}

fn positive_finite_or_one(value: f64) -> f64 {
    positive_value(value).unwrap_or(1.0)
}

fn kde_value(sample: f64, values: &[f64], bandwidth: f64, counts: bool, cumulative: bool) -> f64 {
    let mut total = 0.0;
    for value in values {
        let z = (sample - value) / bandwidth;
        total += if cumulative {
            normal_cdf(z)
        } else {
            (-0.5 * z * z).exp() / (2.0 * PI).sqrt() / bandwidth
        };
    }
    if counts {
        total
    } else {
        total / values.len() as f64
    }
}

fn normal_cdf(value: f64) -> f64 {
    0.5 * (1.0 + erf(value / 2.0_f64.sqrt()))
}

// Abramowitz and Stegun 7.1.26, accurate enough for smooth CDF samples.
fn erf(value: f64) -> f64 {
    let sign = if value < 0.0 { -1.0 } else { 1.0 };
    let x = value.abs();
    let t = 1.0 / (1.0 + 0.3275911 * x);
    let y = 1.0
        - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t
            + 0.254829592)
            * t
            * (-x * x).exp();
    sign * y
}
