use crate::common::{
    expr_node, map_expr_node, sanitize_output_name, validate_output_names,
    validate_unique_generated_names,
};
use async_trait::async_trait;
use avenger_chart_core::{
    AvengerChartError, ChannelExpr, CompiledDataTransform, DataTransform,
    DataTransformCompileContext, DataTransformExecutionContext, DataTransformResult,
    DefaultLogicalExprNodeExt, IntoExpr, NestedBandLevelConfig, NestedBandSpec, SerializableExpr,
    TimeContext, contains_aggregate, nested, time,
};
use datafusion::{
    arrow::datatypes::DataType,
    dataframe::DataFrame,
    functions::datetime::{
        expr_fn::{date_part, to_local_time, to_unixtime},
        from_unixtime as from_unixtime_udf,
    },
    logical_expr::{Expr, col, expr_fn::cast, lit},
};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledTimeLevelsTransform {
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub value: LogicalExprNode,
    pub levels: Vec<TimeLevelKey>,
    pub time_context: TimeContext,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TimeLevel {
    Year,
    Quarter,
    Month,
    DayOfMonth,
    DayOfYear,
    Hour,
    Minute,
    Week,
    DayOfWeek,
}

impl TimeLevel {
    fn suffix(self) -> &'static str {
        match self {
            TimeLevel::Year => "year",
            TimeLevel::Quarter => "quarter",
            TimeLevel::Month => "month",
            TimeLevel::DayOfMonth => "day",
            TimeLevel::DayOfYear => "day_of_year",
            TimeLevel::Hour => "hour",
            TimeLevel::Minute => "minute",
            TimeLevel::Week => "week",
            TimeLevel::DayOfWeek => "day_of_week",
        }
    }

    fn date_part_name(self) -> Result<&'static str, AvengerChartError> {
        match self {
            TimeLevel::Year => Ok("year"),
            TimeLevel::Quarter => Ok("quarter"),
            TimeLevel::Month => Ok("month"),
            TimeLevel::DayOfMonth => Ok("day"),
            TimeLevel::DayOfYear => Ok("doy"),
            TimeLevel::Hour => Ok("hour"),
            TimeLevel::Minute => Ok("minute"),
            TimeLevel::Week | TimeLevel::DayOfWeek => Err(AvengerChartError::InvalidArgument(
                "TimeLevels v1 does not support week-number or day-of-week levels; use low-level time::week/time::day_of_week expressions instead"
                    .to_string(),
            )),
        }
    }

    fn default_label(self) -> TimeLevelLabel {
        match self {
            TimeLevel::Year => TimeLevelLabel::Year4,
            TimeLevel::Quarter => TimeLevelLabel::QuarterShort,
            TimeLevel::Month => TimeLevelLabel::MonthAbbrev,
            TimeLevel::DayOfMonth
            | TimeLevel::DayOfYear
            | TimeLevel::Hour
            | TimeLevel::Minute
            | TimeLevel::Week
            | TimeLevel::DayOfWeek => TimeLevelLabel::Key,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TimeLevelLabel {
    Key,
    Year4,
    QuarterShort,
    MonthName,
    MonthAbbrev,
}

impl TimeLevelLabel {
    fn expr(self, key: Expr) -> Expr {
        match self {
            TimeLevelLabel::Key | TimeLevelLabel::Year4 => time::year_label(key),
            TimeLevelLabel::QuarterShort => time::quarter_label(key),
            TimeLevelLabel::MonthName => time::month_name_from_number(key),
            TimeLevelLabel::MonthAbbrev => time::month_abbrev_from_number(key),
        }
    }
}

#[derive(Clone, Debug)]
pub struct TimeLevelConfig {
    level: TimeLevel,
    label: Option<TimeLevelLabel>,
    output_name: Option<String>,
}

impl TimeLevelConfig {
    pub fn new(level: TimeLevel) -> Self {
        Self {
            level,
            label: None,
            output_name: None,
        }
    }

    pub fn label(mut self, label: TimeLevelLabel) -> Self {
        self.label = Some(label);
        self
    }

    pub fn output_name(mut self, output_name: impl Into<String>) -> Self {
        self.output_name = Some(output_name.into());
        self
    }

    fn into_key(self, base_name: &str) -> TimeLevelKey {
        TimeLevelKey {
            level: self.level,
            key_name: self
                .output_name
                .unwrap_or_else(|| format!("{base_name}_{}", self.level.suffix())),
            label: self.label.unwrap_or_else(|| self.level.default_label()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct TimeLevels {
    value: Expr,
    levels: Vec<TimeLevelConfig>,
    time_context: TimeContext,
    name: Option<String>,
}

impl TimeLevels {
    pub fn new(value: impl IntoExpr) -> Self {
        Self {
            value: value.into_expr(),
            levels: Vec::new(),
            time_context: TimeContext::new(),
            name: None,
        }
    }

    pub fn level(mut self, level: TimeLevel) -> Self {
        self.levels.push(TimeLevelConfig::new(level));
        self
    }

    pub fn level_with<F>(mut self, level: TimeLevel, f: F) -> Self
    where
        F: FnOnce(TimeLevelConfig) -> TimeLevelConfig,
    {
        self.levels.push(f(TimeLevelConfig::new(level)));
        self
    }

    pub fn year(self) -> Self {
        self.level(TimeLevel::Year)
    }

    pub fn year_with<F>(self, f: F) -> Self
    where
        F: FnOnce(TimeLevelConfig) -> TimeLevelConfig,
    {
        self.level_with(TimeLevel::Year, f)
    }

    pub fn quarter(self) -> Self {
        self.level(TimeLevel::Quarter)
    }

    pub fn quarter_with<F>(self, f: F) -> Self
    where
        F: FnOnce(TimeLevelConfig) -> TimeLevelConfig,
    {
        self.level_with(TimeLevel::Quarter, f)
    }

    pub fn month(self) -> Self {
        self.level(TimeLevel::Month)
    }

    pub fn month_with<F>(self, f: F) -> Self
    where
        F: FnOnce(TimeLevelConfig) -> TimeLevelConfig,
    {
        self.level_with(TimeLevel::Month, f)
    }

    pub fn day_of_month(self) -> Self {
        self.level(TimeLevel::DayOfMonth)
    }

    pub fn day_of_year(self) -> Self {
        self.level(TimeLevel::DayOfYear)
    }

    pub fn hour(self) -> Self {
        self.level(TimeLevel::Hour)
    }

    pub fn minute(self) -> Self {
        self.level(TimeLevel::Minute)
    }

    pub fn time_context(mut self, time_context: TimeContext) -> Self {
        self.time_context = time_context;
        self
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
}

impl DataTransform for TimeLevels {
    type Output = TimeLevelsOutput;

    fn into_compiled_and_output(
        self,
        ctx: DataTransformCompileContext,
    ) -> Result<(Box<dyn CompiledDataTransform>, Self::Output), AvengerChartError> {
        if contains_aggregate(&self.value) {
            return Err(AvengerChartError::InvalidArgument(
                "TimeLevels::new(...) does not accept aggregate expressions; use Aggregate before TimeLevels"
                    .to_string(),
            ));
        }
        if self.levels.is_empty() {
            return Err(AvengerChartError::InvalidArgument(
                "TimeLevels requires at least one level".to_string(),
            ));
        }
        validate_unique_time_levels(&self.levels)?;

        let base_name = self
            .name
            .unwrap_or_else(|| sanitize_output_name(&format!("{}_timelevels", self.value)));
        let levels = self
            .levels
            .into_iter()
            .map(|level| level.into_key(&base_name))
            .collect::<Vec<_>>();
        validate_time_level_keys(&levels)?;
        let output = TimeLevelsOutput {
            levels: TimeLevelKeys {
                levels: levels.clone(),
            },
            scope: ctx.scope,
        };
        Ok((
            Box::new(CompiledTimeLevelsTransform {
                value: expr_node(self.value, "timelevels value expression"),
                levels,
                time_context: self.time_context,
            }),
            output,
        ))
    }
}

#[derive(Clone, Debug)]
pub struct TimeLevelsOutput {
    levels: TimeLevelKeys,
    scope: avenger_chart_core::CoordinationScope,
}

impl TimeLevelsOutput {
    pub fn key(&self, level: TimeLevel) -> Expr {
        col(self.key_name(level))
    }

    pub fn key_name(&self, level: TimeLevel) -> &str {
        self.levels
            .key(level)
            .unwrap_or_else(|| panic!("TimeLevels output does not include {level:?}"))
            .key_name
            .as_str()
    }

    pub fn keys(&self) -> Vec<Expr> {
        self.levels
            .levels
            .iter()
            .map(|level| col(&level.key_name))
            .collect()
    }

    pub fn keys_with<I, E>(&self, extra: I) -> Vec<Expr>
    where
        I: IntoIterator<Item = E>,
        E: Into<Expr>,
    {
        self.keys()
            .into_iter()
            .chain(extra.into_iter().map(Into::into))
            .collect()
    }

    pub fn levels(&self) -> TimeLevelKeys {
        self.levels.clone()
    }

    pub fn try_nested(&self) -> Result<ChannelExpr, AvengerChartError> {
        validate_nested_hierarchy(&self.levels.levels)?;
        let source_columns = self
            .levels
            .levels
            .iter()
            .map(|level| level.key_name.clone())
            .collect::<Vec<_>>();
        let mut spec = NestedBandSpec::from_source_columns(source_columns.clone());
        for (index, level) in self.levels.levels.iter().enumerate() {
            spec.levels.insert(
                index,
                NestedBandLevelConfig::<()>::new(Default::default())
                    .label_with(level.label.expr(col(&level.key_name)))
                    .into_spec(),
            );
        }

        Ok(nested(source_columns)
            .map_channel_value(|value| value.with_nested_band_config(spec))
            .with_transform_scope(self.scope))
    }

    pub fn nested(&self) -> ChannelExpr {
        self.try_nested()
            .expect("TimeLevels output cannot be represented as nested categorical bands")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeLevelKeys {
    pub levels: Vec<TimeLevelKey>,
}

impl TimeLevelKeys {
    pub fn key(&self, level: TimeLevel) -> Option<&TimeLevelKey> {
        self.levels.iter().find(|key| key.level == level)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeLevelKey {
    pub level: TimeLevel,
    pub key_name: String,
    pub label: TimeLevelLabel,
}

#[typetag::serde(name = "time_levels")]
#[async_trait]
impl CompiledDataTransform for CompiledTimeLevelsTransform {
    fn clone_box(&self) -> Box<dyn CompiledDataTransform> {
        Box::new(self.clone())
    }

    fn map_exprs(
        &self,
        f: &mut dyn FnMut(Expr) -> Result<Expr, AvengerChartError>,
    ) -> Result<Box<dyn CompiledDataTransform>, AvengerChartError> {
        Ok(Box::new(Self {
            value: map_expr_node(&self.value, f)?,
            levels: self.levels.clone(),
            time_context: self.time_context.clone(),
        }))
    }

    async fn apply(
        &self,
        dataframe: DataFrame,
        ctx: &DataTransformExecutionContext<'_>,
    ) -> Result<DataTransformResult, AvengerChartError> {
        validate_output_names(
            dataframe.schema().fields().iter().map(|field| field.name()),
            self.levels.iter().map(|level| level.key_name.as_str()),
        )?;
        validate_time_level_keys(&self.levels)?;

        let payload = self.with_parent_time_context(&ctx.time_context);
        let value = payload.value.to_default_expr(ctx.session_context)?;
        let value = timezone_cast(value, payload.time_context.resolved_timezone());
        let mut dataframe = dataframe;
        for level in &payload.levels {
            dataframe = dataframe
                .with_column(
                    &level.key_name,
                    time_level_key_expr(level.level, value.clone())?,
                )
                .map_err(AvengerChartError::DataFusionError)?;
        }
        Ok(DataTransformResult::dataframe(dataframe))
    }
}

impl CompiledTimeLevelsTransform {
    fn with_parent_time_context(&self, parent: &TimeContext) -> Self {
        let mut payload = self.clone();
        payload.time_context = payload.time_context.resolved_with_parent(parent);
        payload
    }
}

fn validate_unique_time_levels(levels: &[TimeLevelConfig]) -> Result<(), AvengerChartError> {
    let mut seen = std::collections::BTreeSet::new();
    for level in levels {
        if !seen.insert(level.level.suffix()) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "TimeLevels level {:?} is duplicated",
                level.level
            )));
        }
    }
    Ok(())
}

fn validate_time_level_keys(levels: &[TimeLevelKey]) -> Result<(), AvengerChartError> {
    validate_unique_generated_names(levels.iter().map(|level| level.key_name.as_str()))?;
    for level in levels {
        level.level.date_part_name()?;
    }
    Ok(())
}

fn timezone_cast(value: Expr, timezone: &str) -> Expr {
    // Interpret the input as an instant, then extract calendar fields in the
    // requested local timezone. A direct timestamp timezone cast preserves wall
    // time for naive inputs, which is not useful for chart time bucketing.
    to_local_time(vec![
        from_unixtime_udf().call(vec![to_unixtime(vec![value]), lit(timezone.to_string())]),
    ])
}

fn time_level_key_expr(level: TimeLevel, value: Expr) -> Result<Expr, AvengerChartError> {
    Ok(cast(
        date_part(lit(level.date_part_name()?), value),
        DataType::Int32,
    ))
}

fn validate_nested_hierarchy(levels: &[TimeLevelKey]) -> Result<(), AvengerChartError> {
    let chain = levels.iter().map(|level| level.level).collect::<Vec<_>>();
    let valid = match chain.as_slice() {
        [level] => level.date_part_name().is_ok(),
        [TimeLevel::Year, TimeLevel::Quarter]
        | [TimeLevel::Year, TimeLevel::Quarter, TimeLevel::Month]
        | [
            TimeLevel::Year,
            TimeLevel::Quarter,
            TimeLevel::Month,
            TimeLevel::DayOfMonth,
        ]
        | [TimeLevel::Year, TimeLevel::Month]
        | [TimeLevel::Year, TimeLevel::Month, TimeLevel::DayOfMonth]
        | [TimeLevel::Year, TimeLevel::DayOfYear] => true,
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(AvengerChartError::InvalidArgument(format!(
            "TimeLevels::nested() requires a supported hierarchical chain; got {chain:?}"
        )))
    }
}
