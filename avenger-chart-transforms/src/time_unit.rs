use crate::common::{expr_node, sanitize_output_name, validate_output_names};
use async_trait::async_trait;
use avenger_chart_cartesian::CartesianAxis;
use avenger_chart_core::{
    AvengerChartError, ChannelValue, CompiledDataTransform, DataTransform,
    DataTransformCompileContext, DataTransformExecutionContext, DataTransformResult,
    DefaultLogicalExprNodeExt, DerivedScalarMap, IntoExpr, ScaleChannelValue, SerializableExpr,
    TimeContext, WeekStart, derived_scalar, eval_to_scalars, params_to_datafusion,
};
use datafusion::{
    arrow::{
        array::{Float64Array, Int32Array, Int64Array},
        datatypes::{DataType, Field, Schema, TimeUnit as ArrowTimeUnit},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    dataframe::DataFrame,
    functions::{
        datetime::expr_fn::{date_bin, date_part, make_date, to_timestamp_nanos, to_unixtime},
        expr_fn::{abs, floor, ln},
    },
    functions_aggregate::expr_fn::{max, min},
    logical_expr::{
        Expr, ExprSchemable, JoinType, col,
        expr_fn::{cast, scalar_subquery},
        lit, when,
    },
    prelude::named_struct,
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
            maxbins: Some(lit(10_i64)),
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
            tick_spacing_scalar_id: tick_spacing_scalar_id.clone(),
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
    tick_spacing_scalar_id: String,
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
        let tick_spacing_scalar_id = self.tick_spacing_scalar_id.clone();
        ChannelValue::from(col(column_name))
            .scale(move |scale| {
                scale
                    .domain_interval(
                        derived_scalar(&domain_start_scalar_id, None),
                        derived_scalar(&domain_end_scalar_id, None),
                    )
                    .option("nice", lit(false))
            })
            .with_axis_config(
                CartesianAxis::new().tick_spacing(derived_scalar(&tick_spacing_scalar_id, None)),
            )
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
        let payload = self.with_parent_time_context(&ctx.time_context);
        validate_output_names(
            dataframe.schema().fields().iter().map(|field| field.name()),
            [payload.start_name.as_str(), payload.end_name.as_str()],
        )?;
        let maxbins = validated_maxbins_expr(&payload, ctx).await?;
        let (prepared, original_columns) =
            prepared_time_value_dataframe(dataframe, &payload, ctx.session_context)?;
        let plan = time_unit_plan_dataframe(prepared.clone(), &payload, ctx, maxbins)?;
        let dataframe =
            apply_time_unit_with_plan(prepared, plan.clone(), &payload, original_columns)?;
        let derived_scalars = time_unit_derived_scalars(dataframe.clone(), plan, &payload)?;
        Ok(DataTransformResult {
            dataframe,
            derived_scalars,
        })
    }
}

impl CompiledTimeUnitTransform {
    fn with_parent_time_context(&self, parent: &TimeContext) -> Self {
        let mut payload = self.clone();
        payload.time_context = payload.time_context.resolved_with_parent(parent);
        payload
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
const TIME_MONTHS: &str = "__avenger_timeunit_months";
const TIME_DAYS: &str = "__avenger_timeunit_days";
const TIME_NANOS: &str = "__avenger_timeunit_nanos";

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
    maxbins: Option<Expr>,
) -> Result<DataFrame, AvengerChartError> {
    if let Some(units) = &payload.units {
        return candidate_dataframe(
            ctx.session_context,
            vec![explicit_candidate(units, payload)?],
        );
    }

    let Some(maxbins) = maxbins else {
        return Err(AvengerChartError::InvalidArgument(
            "TimeUnit requires either explicit units or maxbins".to_string(),
        ));
    };
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
        .select(vec![
            col(TIME_SECONDS),
            col(TIME_MONTHS),
            col(TIME_DAYS),
            col(TIME_NANOS),
        ])
        .map_err(AvengerChartError::DataFusionError)
}

async fn validated_maxbins_expr(
    payload: &CompiledTimeUnitTransform,
    ctx: &DataTransformExecutionContext<'_>,
) -> Result<Option<Expr>, AvengerChartError> {
    let Some(maxbins) = &payload.maxbins else {
        return Ok(None);
    };
    let expr = maxbins.to_default_expr(ctx.session_context)?;
    let params = params_to_datafusion(ctx.params);
    let mut values = eval_to_scalars(vec![expr], Some(ctx.session_context), params.as_ref())
        .await
        .map_err(|err| {
            AvengerChartError::InvalidArgument(format!(
                "TimeUnit::maxbins(...) must evaluate to a positive integer scalar: {err}"
            ))
        })?;
    let value = values.pop().ok_or_else(|| {
        AvengerChartError::InternalError("TimeUnit::maxbins(...) returned no value".to_string())
    })?;
    let maxbins = match value {
        ScalarValue::Int8(Some(value)) => i64::from(value),
        ScalarValue::Int16(Some(value)) => i64::from(value),
        ScalarValue::Int32(Some(value)) => i64::from(value),
        ScalarValue::Int64(Some(value)) => value,
        ScalarValue::UInt8(Some(value)) => i64::from(value),
        ScalarValue::UInt16(Some(value)) => i64::from(value),
        ScalarValue::UInt32(Some(value)) => i64::from(value),
        ScalarValue::UInt64(Some(value)) => i64::try_from(value).map_err(|_| {
            AvengerChartError::InvalidArgument(
                "TimeUnit::maxbins(...) must evaluate to a positive integer that fits in Int64"
                    .to_string(),
            )
        })?,
        other if other.is_null() => {
            return Err(AvengerChartError::InvalidArgument(
                "TimeUnit::maxbins(...) must not evaluate to null".to_string(),
            ));
        }
        other => {
            return Err(AvengerChartError::InvalidArgument(format!(
                "TimeUnit::maxbins(...) must evaluate to an integer scalar, got {other:?}"
            )));
        }
    };
    if maxbins <= 0 {
        return Err(AvengerChartError::InvalidArgument(
            "TimeUnit::maxbins(...) must evaluate to a positive integer".to_string(),
        ));
    }
    Ok(Some(lit(maxbins)))
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
        candidate_start_expr(candidate, payload)
    })?;
    dataframe = dataframe
        .with_column(&payload.start_name, start)
        .map_err(AvengerChartError::DataFusionError)?;
    let end = if payload.interval {
        candidate_case_expr(&candidates, |candidate| {
            Ok(col(&payload.start_name) + interval_literal(candidate))
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

fn candidate_start_expr(
    candidate: &TimeUnitCandidate,
    payload: &CompiledTimeUnitTransform,
) -> Result<Expr, AvengerChartError> {
    if let Some(units) = &candidate.explicit_units {
        if let Some(start) =
            cyclic_calendar_start_expr(units, payload.time_context.resolved_week_start())?
        {
            return Ok(start);
        }
    }
    Ok(date_bin(
        interval_literal(candidate),
        col(TIME_VALUE),
        to_timestamp_nanos(vec![lit(candidate.origin.clone())]),
    ))
}

fn cyclic_calendar_start_expr(
    units: &[TimeUnitPart],
    week_start: WeekStart,
) -> Result<Option<Expr>, AvengerChartError> {
    if units.contains(&TimeUnitPart::Year) {
        return Ok(None);
    }
    if units.iter().any(|unit| {
        !matches!(
            unit,
            TimeUnitPart::Quarter | TimeUnitPart::Month | TimeUnitPart::Day
        )
    }) {
        return Ok(None);
    }

    let month = if units.contains(&TimeUnitPart::Month) {
        cast(date_part(lit("month"), col(TIME_VALUE)), DataType::Int32)
    } else if units.contains(&TimeUnitPart::Quarter) {
        (cast(date_part(lit("quarter"), col(TIME_VALUE)), DataType::Int32) - lit(1_i32))
            * lit(3_i32)
            + lit(1_i32)
    } else {
        lit(1_i32)
    };
    let day = if units.contains(&TimeUnitPart::Day) {
        cast(date_part(lit("day"), col(TIME_VALUE)), DataType::Int32)
    } else {
        lit(1_i32)
    };
    let start = cast(
        make_date(lit(week_start.anchor_year()), month, day),
        DataType::Timestamp(ArrowTimeUnit::Millisecond, None),
    );
    let start = when(col(TIME_VALUE).is_not_null(), start)
        .otherwise(lit(ScalarValue::TimestampMillisecond(None, None)))
        .map_err(AvengerChartError::DataFusionError)?;
    Ok(Some(start))
}

fn time_unit_derived_scalars(
    dataframe: DataFrame,
    plan: DataFrame,
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
        scalar_from_dataframe(extent.clone(), col(TIME_MAX))?,
    );
    derived_scalars.insert(
        payload.tick_spacing_scalar_id.clone(),
        scalar_from_dataframe(
            cross_join(extent, plan)?,
            named_struct(vec![
                lit("start"),
                col(TIME_MIN),
                lit("step"),
                candidate_case_expr(&payload_candidates(payload)?, |candidate| {
                    Ok(interval_literal(candidate))
                })?,
            ]),
        )?,
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
    explicit_units: Option<Vec<TimeUnitPart>>,
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
    let mut candidate = candidate_for_unit(
        finest_unit(units)?,
        payload.time_context.resolved_week_start(),
    );
    candidate.explicit_units = Some(units.to_vec());
    Ok(candidate)
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
        explicit_units: None,
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
    let months = candidates
        .iter()
        .map(|candidate| candidate.months)
        .collect::<Vec<_>>();
    let days = candidates
        .iter()
        .map(|candidate| candidate.days)
        .collect::<Vec<_>>();
    let nanos = candidates
        .iter()
        .map(|candidate| candidate.nanos)
        .collect::<Vec<_>>();
    let schema = Arc::new(Schema::new(vec![
        Field::new(TIME_SECONDS, DataType::Float64, false),
        Field::new(TIME_MONTHS, DataType::Int32, false),
        Field::new(TIME_DAYS, DataType::Int32, false),
        Field::new(TIME_NANOS, DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Float64Array::from(seconds)) as _,
            Arc::new(Int32Array::from(months)) as _,
            Arc::new(Int32Array::from(days)) as _,
            Arc::new(Int64Array::from(nanos)) as _,
        ],
    )
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
    mut branch: impl FnMut(&TimeUnitCandidate) -> Result<Expr, AvengerChartError>,
) -> Result<Expr, AvengerChartError> {
    let mut iter = candidates.iter();
    let Some(first) = iter.next() else {
        return Err(AvengerChartError::InvalidArgument(
            "TimeUnit requires at least one candidate interval".to_string(),
        ));
    };
    let mut builder = when(col(TIME_SECONDS).eq(lit(first.seconds)), branch(first)?);
    for candidate in iter {
        builder = builder.when(
            col(TIME_SECONDS).eq(lit(candidate.seconds)),
            branch(candidate)?,
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
