use crate::common::{expr_node, sanitize_output_name, validate_output_names};
use async_trait::async_trait;
use avenger_chart_core::{
    AvengerChartError, ChannelValue, CompiledDataTransform, DataTransform,
    DataTransformCompileContext, DataTransformExecutionContext, DataTransformResult,
    DefaultLogicalExprNodeExt, DerivedScalarMap, IntoExpr, ScaleChannelValue, SerializableExpr,
    TimeContext, WeekStart, derived_scalar,
};
use datafusion::{
    arrow::{
        array::Float64Array,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    dataframe::DataFrame,
    functions::{
        datetime::expr_fn::{date_bin, to_timestamp_nanos, to_unixtime},
        expr_fn::{abs, floor, ln},
    },
    functions_aggregate::expr_fn::{max, min},
    logical_expr::{Expr, ExprSchemable, JoinType, col, expr_fn::scalar_subquery, lit, when},
};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};
use std::sync::Arc;

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledTimeUnitTransform {
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub value: LogicalExprNode,
    #[serde_as(as = "Option<FromInto<SerializableExpr>>")]
    pub maxbins: Option<LogicalExprNode>,
    pub units: Option<Vec<TimeUnitPart>>,
    pub time_context: TimeContext,
    pub interval: bool,
    pub start_name: String,
    pub end_name: String,
    pub domain_start_scalar_id: String,
    pub domain_end_scalar_id: String,
    pub tick_spacing_scalar_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeUnitPart {
    Year,
    Quarter,
    Month,
    Week,
    Day,
    Hour,
    Minute,
    Second,
}

#[derive(Clone, Debug)]
pub struct TimeUnit {
    value: Expr,
    maxbins: Option<Expr>,
    units: Option<Vec<TimeUnitPart>>,
    time_context: TimeContext,
    interval: bool,
    name: Option<String>,
}

impl TimeUnit {
    pub fn new(value: Expr) -> Self {
        Self {
            value,
            maxbins: Some(lit(10.0)),
            units: None,
            time_context: TimeContext::new(),
            interval: true,
            name: None,
        }
    }

    pub fn maxbins(mut self, maxbins: impl IntoExpr) -> Self {
        self.maxbins = Some(maxbins.into_expr());
        self.units = None;
        self
    }

    pub fn unit(mut self, unit: TimeUnitPart) -> Self {
        self.units = Some(vec![unit]);
        self.maxbins = None;
        self
    }

    pub fn units<I>(mut self, units: I) -> Self
    where
        I: IntoIterator<Item = TimeUnitPart>,
    {
        self.units = Some(units.into_iter().collect());
        self.maxbins = None;
        self
    }

    pub fn time_context(mut self, time_context: TimeContext) -> Self {
        self.time_context = time_context;
        self
    }

    pub fn interval(mut self, interval: bool) -> Self {
        self.interval = interval;
        self
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
}

impl DataTransform for TimeUnit {
    type Output = TimeUnitOutput;

    fn into_compiled_and_output(
        self,
        ctx: DataTransformCompileContext,
    ) -> Result<(Box<dyn CompiledDataTransform>, Self::Output), AvengerChartError> {
        if let Some(units) = &self.units
            && units.is_empty()
        {
            return Err(AvengerChartError::InvalidArgument(
                "TimeUnit::units(...) requires at least one unit".to_string(),
            ));
        }
        if avenger_chart_core::contains_aggregate(&self.value) {
            return Err(AvengerChartError::InvalidArgument(
                "TimeUnit::new(...) does not accept aggregate expressions; use Aggregate before TimeUnit"
                    .to_string(),
            ));
        }

        let base_name = self
            .name
            .unwrap_or_else(|| sanitize_output_name(&format!("{}_timeunit", self.value)));
        let start_name = format!("{base_name}_start");
        let end_name = format!("{base_name}_end");
        let domain_start_scalar_id = format!("{base_name}_domain_start");
        let domain_end_scalar_id = format!("{base_name}_domain_end");
        let tick_spacing_scalar_id = format!("{base_name}_tick_spacing");
        let output = TimeUnitOutput {
            start_name: start_name.clone(),
            end_name: end_name.clone(),
            domain_start_scalar_id: domain_start_scalar_id.clone(),
            domain_end_scalar_id: domain_end_scalar_id.clone(),
            scope: ctx.scope,
        };
        Ok((
            Box::new(CompiledTimeUnitTransform {
                value: expr_node(self.value, "timeunit value expression"),
                maxbins: self
                    .maxbins
                    .map(|expr| expr_node(expr, "timeunit maxbins expression")),
                units: self.units,
                time_context: self.time_context,
                interval: self.interval,
                start_name,
                end_name,
                domain_start_scalar_id,
                domain_end_scalar_id,
                tick_spacing_scalar_id,
            }),
            output,
        ))
    }
}

#[derive(Clone, Debug)]
pub struct TimeUnitOutput {
    start_name: String,
    end_name: String,
    domain_start_scalar_id: String,
    domain_end_scalar_id: String,
    scope: avenger_chart_core::Sharing,
}

impl TimeUnitOutput {
    pub fn start(&self) -> ChannelValue {
        self.position_channel(&self.start_name)
    }

    pub fn end(&self) -> ChannelValue {
        self.position_channel(&self.end_name)
    }

    fn position_channel(&self, column_name: &str) -> ChannelValue {
        let domain_start_scalar_id = self.domain_start_scalar_id.clone();
        let domain_end_scalar_id = self.domain_end_scalar_id.clone();
        ChannelValue::from(col(column_name))
            .scale(move |scale| {
                scale
                    .domain_interval(
                        derived_scalar(&domain_start_scalar_id, None),
                        derived_scalar(&domain_end_scalar_id, None),
                    )
                    .option("nice", lit(false))
            })
            .with_transform_scope(self.scope)
    }
}

#[typetag::serde(name = "time_unit")]
#[async_trait]
impl CompiledDataTransform for CompiledTimeUnitTransform {
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
            [self.start_name.as_str(), self.end_name.as_str()],
        )?;
        let (prepared, original_columns) =
            prepared_time_value_dataframe(dataframe, self, ctx.session_context)?;
        let plan = time_unit_plan_dataframe(prepared.clone(), self, ctx)?;
        let dataframe = apply_time_unit_with_plan(prepared, plan, self, original_columns)?;
        let derived_scalars = time_unit_derived_scalars(dataframe.clone(), self)?;
        Ok(DataTransformResult {
            dataframe,
            derived_scalars,
        })
    }
}

const TIME_VALUE: &str = "__avenger_timeunit_value";
const TIME_SECONDS: &str = "__avenger_timeunit_seconds";
const TIME_MIN: &str = "__avenger_timeunit_min";
const TIME_MAX: &str = "__avenger_timeunit_max";
const TIME_SPAN_SECONDS: &str = "__avenger_timeunit_span_seconds";
const TIME_MAXBINS: &str = "__avenger_timeunit_maxbins";
const TIME_TARGET_SECONDS: &str = "__avenger_timeunit_target_seconds";
const TIME_SCORE: &str = "__avenger_timeunit_score";

fn prepared_time_value_dataframe(
    dataframe: DataFrame,
    payload: &CompiledTimeUnitTransform,
    ctx: &datafusion::prelude::SessionContext,
) -> Result<(DataFrame, Vec<String>), AvengerChartError> {
    let original_columns = dataframe
        .schema()
        .fields()
        .iter()
        .map(|field| field.name().clone())
        .collect::<Vec<_>>();
    let value_expr = payload.value.to_default_expr(ctx)?;
    let dataframe = dataframe
        .with_column(TIME_VALUE, value_expr)
        .map_err(AvengerChartError::DataFusionError)?;
    Ok((dataframe, original_columns))
}

fn time_unit_plan_dataframe(
    dataframe: DataFrame,
    payload: &CompiledTimeUnitTransform,
    ctx: &DataTransformExecutionContext<'_>,
) -> Result<DataFrame, AvengerChartError> {
    if let Some(units) = &payload.units {
        return candidate_dataframe(
            ctx.session_context,
            vec![explicit_candidate(units, payload)?],
        );
    }

    let Some(maxbins) = &payload.maxbins else {
        return Err(AvengerChartError::InvalidArgument(
            "TimeUnit requires either explicit units or maxbins".to_string(),
        ));
    };
    let maxbins = maxbins.to_default_expr(ctx.session_context)?;
    let extent = dataframe
        .aggregate(
            Vec::<Expr>::new(),
            vec![
                min(col(TIME_VALUE)).alias(TIME_MIN),
                max(col(TIME_VALUE)).alias(TIME_MAX),
            ],
        )
        .map_err(AvengerChartError::DataFusionError)?;
    let span = to_unixtime(vec![col(TIME_MAX)]) - to_unixtime(vec![col(TIME_MIN)]);
    let maxbins = cast_to_f64(maxbins, &extent)?;
    let extent = extent
        .with_column(TIME_SPAN_SECONDS, span)
        .map_err(AvengerChartError::DataFusionError)?
        .with_column(TIME_MAXBINS, floor(maxbins))
        .map_err(AvengerChartError::DataFusionError)?;
    let candidates = candidate_dataframe(
        ctx.session_context,
        auto_candidates(payload.time_context.resolved_week_start()),
    )?;
    cross_join(extent, candidates)?
        .with_column(
            TIME_TARGET_SECONDS,
            when(
                col(TIME_MAXBINS).gt(lit(0.0)),
                col(TIME_SPAN_SECONDS) / col(TIME_MAXBINS),
            )
            .otherwise(lit(1.0))
            .map_err(AvengerChartError::DataFusionError)?,
        )
        .map_err(AvengerChartError::DataFusionError)?
        .with_column(
            TIME_SCORE,
            abs(ln(col(TIME_SECONDS)) - ln(col(TIME_TARGET_SECONDS))),
        )
        .map_err(AvengerChartError::DataFusionError)?
        .sort(vec![
            col(TIME_SCORE).sort(true, false),
            col(TIME_SECONDS).sort(true, false),
        ])
        .map_err(AvengerChartError::DataFusionError)?
        .limit(0, Some(1))
        .map_err(AvengerChartError::DataFusionError)?
        .select(vec![col(TIME_SECONDS)])
        .map_err(AvengerChartError::DataFusionError)
}

fn apply_time_unit_with_plan(
    dataframe: DataFrame,
    plan: DataFrame,
    payload: &CompiledTimeUnitTransform,
    original_columns: Vec<String>,
) -> Result<DataFrame, AvengerChartError> {
    let mut dataframe = cross_join(dataframe, plan)?;
    let candidates = payload_candidates(payload)?;
    let start = candidate_case_expr(&candidates, |candidate| {
        date_bin(
            interval_literal(candidate),
            col(TIME_VALUE),
            to_timestamp_nanos(vec![lit(candidate.origin.clone())]),
        )
    })?;
    dataframe = dataframe
        .with_column(&payload.start_name, start)
        .map_err(AvengerChartError::DataFusionError)?;
    let end = if payload.interval {
        candidate_case_expr(&candidates, |candidate| {
            col(&payload.start_name) + interval_literal(candidate)
        })?
    } else {
        col(&payload.start_name)
    };
    dataframe = dataframe
        .with_column(&payload.end_name, end)
        .map_err(AvengerChartError::DataFusionError)?;

    let mut projection = original_columns
        .iter()
        .map(|name| col(name))
        .collect::<Vec<_>>();
    projection.push(col(&payload.start_name));
    projection.push(col(&payload.end_name));
    dataframe
        .select(projection)
        .map_err(AvengerChartError::DataFusionError)
}

fn time_unit_derived_scalars(
    dataframe: DataFrame,
    payload: &CompiledTimeUnitTransform,
) -> Result<DerivedScalarMap, AvengerChartError> {
    let extent = dataframe
        .aggregate(
            Vec::<Expr>::new(),
            vec![
                min(col(&payload.start_name)).alias(TIME_MIN),
                max(col(&payload.end_name)).alias(TIME_MAX),
            ],
        )
        .map_err(AvengerChartError::DataFusionError)?;
    let mut derived_scalars = DerivedScalarMap::new();
    derived_scalars.insert(
        payload.domain_start_scalar_id.clone(),
        scalar_from_dataframe(extent.clone(), col(TIME_MIN))?,
    );
    derived_scalars.insert(
        payload.domain_end_scalar_id.clone(),
        scalar_from_dataframe(extent, col(TIME_MAX))?,
    );
    Ok(derived_scalars)
}

fn scalar_from_dataframe(dataframe: DataFrame, expr: Expr) -> Result<Expr, AvengerChartError> {
    let dataframe = dataframe
        .select(vec![expr.alias("__avenger_timeunit_scalar")])
        .map_err(AvengerChartError::DataFusionError)?;
    Ok(scalar_subquery(Arc::new(dataframe.into_unoptimized_plan())))
}

fn cast_to_f64(expr: Expr, dataframe: &DataFrame) -> Result<Expr, AvengerChartError> {
    expr.cast_to(&DataType::Float64, dataframe.schema())
        .map_err(AvengerChartError::DataFusionError)
}

fn cross_join(left: DataFrame, right: DataFrame) -> Result<DataFrame, AvengerChartError> {
    left.join_on(right, JoinType::Inner, [lit(true)])
        .map_err(AvengerChartError::DataFusionError)
}

#[derive(Clone)]
struct TimeUnitCandidate {
    months: i32,
    days: i32,
    nanos: i64,
    seconds: f64,
    origin: String,
}

fn candidate_for_unit(unit: TimeUnitPart, week_start: WeekStart) -> TimeUnitCandidate {
    match unit {
        TimeUnitPart::Year => candidate(unit, 12, 0, 0, 365.25 * 24.0 * 60.0 * 60.0, week_start),
        TimeUnitPart::Quarter => candidate(unit, 3, 0, 0, 91.3125 * 24.0 * 60.0 * 60.0, week_start),
        TimeUnitPart::Month => candidate(unit, 1, 0, 0, 30.4375 * 24.0 * 60.0 * 60.0, week_start),
        TimeUnitPart::Week => candidate(unit, 0, 7, 0, 7.0 * 24.0 * 60.0 * 60.0, week_start),
        TimeUnitPart::Day => candidate(unit, 0, 1, 0, 24.0 * 60.0 * 60.0, week_start),
        TimeUnitPart::Hour => candidate(unit, 0, 0, 3_600_000_000_000, 60.0 * 60.0, week_start),
        TimeUnitPart::Minute => candidate(unit, 0, 0, 60_000_000_000, 60.0, week_start),
        TimeUnitPart::Second => candidate(unit, 0, 0, 1_000_000_000, 1.0, week_start),
    }
}

fn explicit_candidate(
    units: &[TimeUnitPart],
    payload: &CompiledTimeUnitTransform,
) -> Result<TimeUnitCandidate, AvengerChartError> {
    Ok(candidate_for_unit(
        finest_unit(units)?,
        payload.time_context.resolved_week_start(),
    ))
}

fn payload_candidates(
    payload: &CompiledTimeUnitTransform,
) -> Result<Vec<TimeUnitCandidate>, AvengerChartError> {
    if let Some(units) = &payload.units {
        Ok(vec![explicit_candidate(units, payload)?])
    } else {
        Ok(auto_candidates(payload.time_context.resolved_week_start()))
    }
}

fn candidate(
    unit: TimeUnitPart,
    months: i32,
    days: i32,
    nanos: i64,
    seconds: f64,
    week_start: WeekStart,
) -> TimeUnitCandidate {
    let origin = if unit == TimeUnitPart::Week {
        week_start.anchor_timestamp()
    } else {
        "1970-01-01T00:00:00Z"
    };
    TimeUnitCandidate {
        months,
        days,
        nanos,
        seconds,
        origin: origin.to_string(),
    }
}

fn auto_candidates(week_start: WeekStart) -> Vec<TimeUnitCandidate> {
    [
        TimeUnitPart::Second,
        TimeUnitPart::Minute,
        TimeUnitPart::Hour,
        TimeUnitPart::Day,
        TimeUnitPart::Week,
        TimeUnitPart::Month,
        TimeUnitPart::Quarter,
        TimeUnitPart::Year,
    ]
    .into_iter()
    .map(|unit| candidate_for_unit(unit, week_start))
    .collect()
}

fn candidate_dataframe(
    ctx: &datafusion::prelude::SessionContext,
    candidates: Vec<TimeUnitCandidate>,
) -> Result<DataFrame, AvengerChartError> {
    let seconds = candidates
        .iter()
        .map(|candidate| candidate.seconds)
        .collect::<Vec<_>>();
    let schema = Arc::new(Schema::new(vec![Field::new(
        TIME_SECONDS,
        DataType::Float64,
        false,
    )]));
    let batch = RecordBatch::try_new(schema, vec![Arc::new(Float64Array::from(seconds)) as _])
        .map_err(AvengerChartError::ArrowError)?;
    ctx.read_batch(batch)
        .map_err(AvengerChartError::DataFusionError)
}

fn interval_literal(candidate: &TimeUnitCandidate) -> Expr {
    lit(ScalarValue::IntervalMonthDayNano(Some(
        datafusion::arrow::array::types::IntervalMonthDayNanoType::make_value(
            candidate.months,
            candidate.days,
            candidate.nanos,
        ),
    )))
}

fn candidate_case_expr(
    candidates: &[TimeUnitCandidate],
    mut branch: impl FnMut(&TimeUnitCandidate) -> Expr,
) -> Result<Expr, AvengerChartError> {
    let mut iter = candidates.iter();
    let Some(first) = iter.next() else {
        return Err(AvengerChartError::InvalidArgument(
            "TimeUnit requires at least one candidate interval".to_string(),
        ));
    };
    let mut builder = when(col(TIME_SECONDS).eq(lit(first.seconds)), branch(first));
    for candidate in iter {
        builder = builder.when(
            col(TIME_SECONDS).eq(lit(candidate.seconds)),
            branch(candidate),
        );
    }
    builder
        .otherwise(lit(ScalarValue::Null))
        .map_err(AvengerChartError::DataFusionError)
}

fn finest_unit(units: &[TimeUnitPart]) -> Result<TimeUnitPart, AvengerChartError> {
    units
        .iter()
        .copied()
        .max_by_key(|unit| unit_rank(*unit))
        .ok_or_else(|| {
            AvengerChartError::InvalidArgument(
                "TimeUnit::units(...) requires at least one unit".to_string(),
            )
        })
}

fn unit_rank(unit: TimeUnitPart) -> u8 {
    match unit {
        TimeUnitPart::Year => 0,
        TimeUnitPart::Quarter => 1,
        TimeUnitPart::Month => 2,
        TimeUnitPart::Week => 3,
        TimeUnitPart::Day => 4,
        TimeUnitPart::Hour => 5,
        TimeUnitPart::Minute => 6,
        TimeUnitPart::Second => 7,
    }
}
