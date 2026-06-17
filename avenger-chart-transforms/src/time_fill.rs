use crate::common::{
    expr_node, map_expr_node, map_expr_nodes, sanitize_output_name, simple_column_name,
    validate_generated_name, validate_unique_generated_names,
};
use crate::{TimeLevel, TimeLevelKeys};
use async_trait::async_trait;
use avenger_chart_core::{
    AvengerChartError, CompiledDataTransform, DataTransform, DataTransformCompileContext,
    DataTransformExecutionContext, DataTransformResult, DefaultLogicalExprNodeExt, IntoExpr,
    SerializableExpr, eval_to_scalars, params_to_datafusion,
};
use chrono::{Datelike, NaiveDate};
use datafusion::{
    arrow::{
        array::{Array, ArrayRef, Int32Array},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    dataframe::DataFrame,
    functions::datetime::expr_fn::{date_part, make_date},
    functions_aggregate::expr_fn::{max, min},
    logical_expr::{Expr, JoinType, Operator, binary_expr, col, expr_fn::cast, lit, when},
};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};
use std::sync::Arc;

const INPUT_VALUE: &str = "__avenger_timefill_input_value";
const INPUT_ORDINAL: &str = "__avenger_timefill_input_ordinal";
const SPINE_ORDINAL: &str = "__avenger_timefill_spine_ordinal";
const PRESENT: &str = "__avenger_timefill_present";
const MIN_ORDINAL: &str = "__avenger_timefill_min_ordinal";
const MAX_ORDINAL: &str = "__avenger_timefill_max_ordinal";

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledTimeFillTransform {
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub field: LogicalExprNode,
    pub levels: TimeLevelKeys,
    #[serde_as(as = "Vec<FromInto<SerializableExpr>>")]
    pub group_by: Vec<LogicalExprNode>,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub fill_value: LogicalExprNode,
    pub extent: Option<TimeFillExtentSpec>,
    pub value_name: String,
    pub flag_name: Option<String>,
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimeFillExtentSpec {
    #[serde_as(as = "Vec<FromInto<SerializableExpr>>")]
    pub start: Vec<LogicalExprNode>,
    #[serde_as(as = "Vec<FromInto<SerializableExpr>>")]
    pub end: Vec<LogicalExprNode>,
}

#[derive(Clone, Debug)]
pub struct TimeFill {
    field: Expr,
    levels: Option<TimeLevelKeys>,
    group_by: Vec<Expr>,
    fill_value: Option<Expr>,
    extent: Option<(Vec<Expr>, Vec<Expr>)>,
    value_name: Option<String>,
    flag_name: Option<String>,
}

impl TimeFill {
    pub fn new(field: impl IntoExpr) -> Self {
        Self {
            field: field.into_expr(),
            levels: None,
            group_by: Vec::new(),
            fill_value: None,
            extent: None,
            value_name: None,
            flag_name: None,
        }
    }

    pub fn levels(mut self, levels: TimeLevelKeys) -> Self {
        self.levels = Some(levels);
        self
    }

    pub fn group_by<I, E>(mut self, exprs: I) -> Self
    where
        I: IntoIterator<Item = E>,
        E: IntoExpr,
    {
        self.group_by
            .extend(exprs.into_iter().map(IntoExpr::into_expr));
        self
    }

    pub fn fill_value(mut self, expr: impl IntoExpr) -> Self {
        self.fill_value = Some(expr.into_expr());
        self
    }

    pub fn extent<I, J, E, F>(mut self, start: I, end: J) -> Self
    where
        I: IntoIterator<Item = E>,
        J: IntoIterator<Item = F>,
        E: IntoExpr,
        F: IntoExpr,
    {
        self.extent = Some((
            start.into_iter().map(IntoExpr::into_expr).collect(),
            end.into_iter().map(IntoExpr::into_expr).collect(),
        ));
        self
    }

    pub fn as_value(mut self, name: impl Into<String>) -> Self {
        self.value_name = Some(name.into());
        self
    }

    pub fn flag(mut self, name: impl Into<String>) -> Self {
        self.flag_name = Some(name.into());
        self
    }
}

impl DataTransform for TimeFill {
    type Output = TimeFillOutput;

    fn into_compiled_and_output(
        self,
        _ctx: DataTransformCompileContext,
    ) -> Result<(Box<dyn CompiledDataTransform>, Self::Output), AvengerChartError> {
        let Some(levels) = self.levels else {
            return Err(AvengerChartError::InvalidArgument(
                "TimeFill transform requires levels(...) from TimeLevelsOutput::levels()"
                    .to_string(),
            ));
        };
        let Some(fill_value) = self.fill_value else {
            return Err(AvengerChartError::InvalidArgument(
                "TimeFill transform requires fill_value(...)".to_string(),
            ));
        };
        let hierarchy = TimeFillHierarchy::from_levels(&levels)?;
        if let Some((start, end)) = &self.extent {
            hierarchy.validate_component_count(start.len())?;
            hierarchy.validate_component_count(end.len())?;
        }

        let value_name = self.value_name.unwrap_or_else(|| {
            simple_column_name(&self.field)
                .unwrap_or_else(|| sanitize_output_name(&format!("{}_timefill", self.field)))
        });
        validate_generated_name(&value_name)?;
        if let Some(flag_name) = &self.flag_name {
            validate_generated_name(flag_name)?;
            if flag_name == &value_name {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Data transform output name '{flag_name}' is duplicated"
                )));
            }
        }

        let output = TimeFillOutput {
            value_name: value_name.clone(),
            flag_name: self.flag_name.clone(),
        };
        let extent = self.extent.map(|(start, end)| TimeFillExtentSpec {
            start: start
                .into_iter()
                .map(|expr| expr_node(expr, "timefill extent start expression"))
                .collect(),
            end: end
                .into_iter()
                .map(|expr| expr_node(expr, "timefill extent end expression"))
                .collect(),
        });
        Ok((
            Box::new(CompiledTimeFillTransform {
                field: expr_node(self.field, "timefill field expression"),
                levels,
                group_by: self
                    .group_by
                    .into_iter()
                    .map(|expr| expr_node(expr, "timefill group_by expression"))
                    .collect(),
                fill_value: expr_node(fill_value, "timefill fill value expression"),
                extent,
                value_name,
                flag_name: self.flag_name,
            }),
            output,
        ))
    }
}

#[derive(Clone, Debug)]
pub struct TimeFillOutput {
    value_name: String,
    flag_name: Option<String>,
}

impl TimeFillOutput {
    pub fn value(&self) -> Expr {
        col(&self.value_name)
    }

    pub fn flag(&self) -> Expr {
        let Some(flag_name) = &self.flag_name else {
            panic!("TimeFill flag output was requested, but no flag column was configured");
        };
        col(flag_name)
    }
}

#[typetag::serde(name = "time_fill")]
#[async_trait]
impl CompiledDataTransform for CompiledTimeFillTransform {
    fn clone_box(&self) -> Box<dyn CompiledDataTransform> {
        Box::new(self.clone())
    }

    fn map_exprs(
        &self,
        f: &mut dyn FnMut(Expr) -> Result<Expr, AvengerChartError>,
    ) -> Result<Box<dyn CompiledDataTransform>, AvengerChartError> {
        Ok(Box::new(Self {
            field: map_expr_node(&self.field, f)?,
            levels: self.levels.clone(),
            group_by: map_expr_nodes(&self.group_by, f)?,
            fill_value: map_expr_node(&self.fill_value, f)?,
            extent: self
                .extent
                .as_ref()
                .map(|extent| {
                    Ok::<_, AvengerChartError>(TimeFillExtentSpec {
                        start: map_expr_nodes(&extent.start, f)?,
                        end: map_expr_nodes(&extent.end, f)?,
                    })
                })
                .transpose()?,
            value_name: self.value_name.clone(),
            flag_name: self.flag_name.clone(),
        }))
    }

    async fn apply(
        &self,
        dataframe: DataFrame,
        ctx: &DataTransformExecutionContext<'_>,
    ) -> Result<DataTransformResult, AvengerChartError> {
        validate_generated_names(self)?;
        let hierarchy = TimeFillHierarchy::from_levels(&self.levels)?;
        let original_names = dataframe
            .schema()
            .fields()
            .iter()
            .map(|field| field.name().clone())
            .collect::<Vec<_>>();
        let domain_level_names = domain_level_names(self.levels.levels.len());
        let group_names = group_hidden_names(self.group_by.len());
        let domain_group_names = group_domain_names(self.group_by.len());

        let keyed = add_hidden_columns(dataframe, self, &hierarchy, &group_names, ctx)?;
        validate_observed_components(keyed.clone(), self, &hierarchy).await?;
        let Some((start, end)) = extent_ordinals(keyed.clone(), self, &hierarchy, ctx).await?
        else {
            return Ok(DataTransformResult::dataframe(
                keyed
                    .select(original_names.iter().map(col).collect::<Vec<_>>())
                    .map_err(AvengerChartError::DataFusionError)?,
            ));
        };
        let spine = spine_dataframe(
            ctx.session_context,
            &hierarchy,
            start,
            end,
            &domain_level_names,
        )?;

        let domain = if group_names.is_empty() {
            spine
        } else {
            let group_domain = keyed
                .clone()
                .select(
                    group_names
                        .iter()
                        .zip(domain_group_names.iter())
                        .map(|(input, domain)| col(input).alias(domain))
                        .collect::<Vec<_>>(),
                )
                .map_err(AvengerChartError::DataFusionError)?
                .distinct()
                .map_err(AvengerChartError::DataFusionError)?;
            group_domain
                .join_on(spine, JoinType::Inner, [lit(true)])
                .map_err(AvengerChartError::DataFusionError)?
        };

        let joined = domain
            .join_on(
                keyed,
                JoinType::Left,
                join_predicates(&group_names, &domain_group_names),
            )
            .map_err(AvengerChartError::DataFusionError)?;

        let projection = output_projection(
            &original_names,
            self,
            &domain_level_names,
            &domain_group_names,
            ctx,
        )?;
        Ok(DataTransformResult::dataframe(
            joined
                .select(projection)
                .map_err(AvengerChartError::DataFusionError)?,
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TimeFillHierarchy {
    Year,
    YearQuarter,
    YearMonth { has_quarter: bool },
    YearMonthDay { has_quarter: bool },
    YearDayOfYear,
}

impl TimeFillHierarchy {
    fn from_levels(levels: &TimeLevelKeys) -> Result<Self, AvengerChartError> {
        let chain = levels
            .levels
            .iter()
            .map(|level| level.level)
            .collect::<Vec<_>>();
        match chain.as_slice() {
            [TimeLevel::Year] => Ok(Self::Year),
            [TimeLevel::Year, TimeLevel::Quarter] => Ok(Self::YearQuarter),
            [TimeLevel::Year, TimeLevel::Month] => Ok(Self::YearMonth { has_quarter: false }),
            [TimeLevel::Year, TimeLevel::Quarter, TimeLevel::Month] => {
                Ok(Self::YearMonth { has_quarter: true })
            }
            [TimeLevel::Year, TimeLevel::Month, TimeLevel::DayOfMonth] => {
                Ok(Self::YearMonthDay { has_quarter: false })
            }
            [
                TimeLevel::Year,
                TimeLevel::Quarter,
                TimeLevel::Month,
                TimeLevel::DayOfMonth,
            ] => Ok(Self::YearMonthDay { has_quarter: true }),
            [TimeLevel::Year, TimeLevel::DayOfYear] => Ok(Self::YearDayOfYear),
            _ if chain
                .iter()
                .any(|level| matches!(level, TimeLevel::Week | TimeLevel::DayOfWeek)) =>
            {
                Err(AvengerChartError::InvalidArgument(
                    "TimeFill v1 does not support week-number or day-of-week levels".to_string(),
                ))
            }
            _ => Err(AvengerChartError::InvalidArgument(format!(
                "TimeFill requires a supported hierarchical TimeLevels chain; got {chain:?}"
            ))),
        }
    }

    fn validate_component_count(self, len: usize) -> Result<(), AvengerChartError> {
        if len == self.level_count() {
            Ok(())
        } else {
            Err(AvengerChartError::InvalidArgument(format!(
                "TimeFill extent component count {len} does not match configured time level count {}",
                self.level_count()
            )))
        }
    }

    fn level_count(self) -> usize {
        match self {
            Self::Year => 1,
            Self::YearQuarter | Self::YearMonth { has_quarter: false } | Self::YearDayOfYear => 2,
            Self::YearMonth { has_quarter: true } | Self::YearMonthDay { has_quarter: false } => 3,
            Self::YearMonthDay { has_quarter: true } => 4,
        }
    }

    fn ordinal_expr(self, levels: &TimeLevelKeys) -> Expr {
        let year = level_col(levels, TimeLevel::Year);
        match self {
            Self::Year => year,
            Self::YearQuarter => {
                year * lit(4_i32) + (level_col(levels, TimeLevel::Quarter) - lit(1_i32))
            }
            Self::YearMonth { .. } => {
                year * lit(12_i32) + (level_col(levels, TimeLevel::Month) - lit(1_i32))
            }
            Self::YearMonthDay { .. } => epoch_day_expr(
                year,
                level_col(levels, TimeLevel::Month),
                level_col(levels, TimeLevel::DayOfMonth),
            ),
            Self::YearDayOfYear => {
                epoch_day_expr(year, lit(1_i32), lit(1_i32))
                    + (level_col(levels, TimeLevel::DayOfYear) - lit(1_i32))
            }
        }
    }

    fn ordinal_from_components(self, components: &[i32]) -> Result<i32, AvengerChartError> {
        self.validate_components(components)?;
        Ok(match self {
            Self::Year => components[0],
            Self::YearQuarter => components[0] * 4 + (components[1] - 1),
            Self::YearMonth { has_quarter: false } => components[0] * 12 + (components[1] - 1),
            Self::YearMonth { has_quarter: true } => components[0] * 12 + (components[2] - 1),
            Self::YearMonthDay { has_quarter: false } => {
                epoch_day(components[0], components[1], components[2])?
            }
            Self::YearMonthDay { has_quarter: true } => {
                epoch_day(components[0], components[2], components[3])?
            }
            Self::YearDayOfYear => epoch_day_from_year_day(components[0], components[1])?,
        })
    }

    fn components_for_ordinal(self, ordinal: i32) -> Vec<i32> {
        match self {
            Self::Year => vec![ordinal],
            Self::YearQuarter => vec![ordinal.div_euclid(4), ordinal.rem_euclid(4) + 1],
            Self::YearMonth { has_quarter: false } => {
                vec![ordinal.div_euclid(12), ordinal.rem_euclid(12) + 1]
            }
            Self::YearMonth { has_quarter: true } => {
                let year = ordinal.div_euclid(12);
                let month = ordinal.rem_euclid(12) + 1;
                vec![year, quarter_for_month(month), month]
            }
            Self::YearMonthDay { has_quarter: false } => {
                let date = date_from_epoch_day(ordinal);
                vec![date.year(), date.month() as i32, date.day() as i32]
            }
            Self::YearMonthDay { has_quarter: true } => {
                let date = date_from_epoch_day(ordinal);
                let month = date.month() as i32;
                vec![
                    date.year(),
                    quarter_for_month(month),
                    month,
                    date.day() as i32,
                ]
            }
            Self::YearDayOfYear => {
                let date = date_from_epoch_day(ordinal);
                vec![date.year(), date.ordinal() as i32]
            }
        }
    }

    fn validate_components(self, components: &[i32]) -> Result<(), AvengerChartError> {
        self.validate_component_count(components.len())?;
        match self {
            Self::Year => Ok(()),
            Self::YearQuarter => validate_quarter(components[1]),
            Self::YearMonth { has_quarter: false } => validate_month(components[1]),
            Self::YearMonth { has_quarter: true } => {
                validate_quarter_month(components[1], components[2])
            }
            Self::YearMonthDay { has_quarter: false } => {
                validate_ymd(components[0], components[1], components[2])
            }
            Self::YearMonthDay { has_quarter: true } => {
                validate_quarter_month(components[1], components[2])?;
                validate_ymd(components[0], components[2], components[3])
            }
            Self::YearDayOfYear => {
                epoch_day_from_year_day(components[0], components[1]).map(|_| ())
            }
        }
    }
}

fn level_col(levels: &TimeLevelKeys, level: TimeLevel) -> Expr {
    col(&levels.key(level).expect("validated time level").key_name)
}

fn epoch_day_expr(year: Expr, month: Expr, day: Expr) -> Expr {
    cast(
        date_part(lit("epoch"), make_date(year, month, day)) / lit(86_400_i32),
        DataType::Int32,
    )
}

fn validate_generated_names(spec: &CompiledTimeFillTransform) -> Result<(), AvengerChartError> {
    validate_generated_name(&spec.value_name)?;
    if let Some(flag_name) = &spec.flag_name {
        validate_generated_name(flag_name)?;
        if flag_name == &spec.value_name {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Data transform output name '{flag_name}' is duplicated"
            )));
        }
    }
    validate_unique_generated_names(
        [spec.value_name.as_str()]
            .into_iter()
            .chain(spec.flag_name.iter().map(String::as_str)),
    )
}

fn add_hidden_columns(
    mut dataframe: DataFrame,
    spec: &CompiledTimeFillTransform,
    hierarchy: &TimeFillHierarchy,
    group_names: &[String],
    ctx: &DataTransformExecutionContext<'_>,
) -> Result<DataFrame, AvengerChartError> {
    dataframe = dataframe
        .with_column(
            INPUT_VALUE,
            spec.field.to_default_expr(ctx.session_context)?,
        )
        .map_err(AvengerChartError::DataFusionError)?;
    dataframe = dataframe
        .with_column(INPUT_ORDINAL, hierarchy.ordinal_expr(&spec.levels))
        .map_err(AvengerChartError::DataFusionError)?;
    dataframe = dataframe
        .with_column(PRESENT, lit(true))
        .map_err(AvengerChartError::DataFusionError)?;
    for (group, name) in spec.group_by.iter().zip(group_names.iter()) {
        dataframe = dataframe
            .with_column(name, group.to_default_expr(ctx.session_context)?)
            .map_err(AvengerChartError::DataFusionError)?;
    }
    Ok(dataframe)
}

async fn validate_observed_components(
    dataframe: DataFrame,
    spec: &CompiledTimeFillTransform,
    hierarchy: &TimeFillHierarchy,
) -> Result<(), AvengerChartError> {
    match hierarchy {
        TimeFillHierarchy::YearMonth { has_quarter: true }
        | TimeFillHierarchy::YearMonthDay { has_quarter: true } => {
            let quarter = level_col(&spec.levels, TimeLevel::Quarter);
            let month = level_col(&spec.levels, TimeLevel::Month);
            let invalid = dataframe
                .filter(
                    quarter
                        .clone()
                        .is_not_null()
                        .and(month.clone().is_not_null())
                        .and(quarter.not_eq(quarter_expr_from_month(month))),
                )
                .map_err(AvengerChartError::DataFusionError)?
                .limit(0, Some(1))
                .map_err(AvengerChartError::DataFusionError)?
                .collect()
                .await
                .map_err(AvengerChartError::DataFusionError)?;
            if invalid.iter().any(|batch| batch.num_rows() > 0) {
                return Err(AvengerChartError::InvalidArgument(
                    "TimeFill found inconsistent quarter/month components".to_string(),
                ));
            }
        }
        _ => {}
    }
    Ok(())
}

fn quarter_expr_from_month(month: Expr) -> Expr {
    cast((month - lit(1_i32)) / lit(3_i32), DataType::Int32) + lit(1_i32)
}

async fn extent_ordinals(
    dataframe: DataFrame,
    spec: &CompiledTimeFillTransform,
    hierarchy: &TimeFillHierarchy,
    ctx: &DataTransformExecutionContext<'_>,
) -> Result<Option<(i32, i32)>, AvengerChartError> {
    if let Some(extent) = &spec.extent {
        let start = extent_components(&extent.start, ctx).await?;
        let end = extent_components(&extent.end, ctx).await?;
        let start = hierarchy.ordinal_from_components(&start)?;
        let end = hierarchy.ordinal_from_components(&end)?;
        if start > end {
            return Err(AvengerChartError::InvalidArgument(
                "TimeFill extent start must be less than or equal to extent end".to_string(),
            ));
        }
        return Ok(Some((start, end)));
    }

    let extent = dataframe
        .aggregate(
            Vec::<Expr>::new(),
            vec![
                min(col(INPUT_ORDINAL)).alias(MIN_ORDINAL),
                max(col(INPUT_ORDINAL)).alias(MAX_ORDINAL),
            ],
        )
        .map_err(AvengerChartError::DataFusionError)?
        .collect()
        .await
        .map_err(AvengerChartError::DataFusionError)?;
    let Some(batch) = extent.first() else {
        return Ok(None);
    };
    let min_values = batch
        .column_by_name(MIN_ORDINAL)
        .and_then(|array| array.as_any().downcast_ref::<Int32Array>())
        .ok_or_else(|| {
            AvengerChartError::InternalError("TimeFill min ordinal was not Int32".to_string())
        })?;
    let max_values = batch
        .column_by_name(MAX_ORDINAL)
        .and_then(|array| array.as_any().downcast_ref::<Int32Array>())
        .ok_or_else(|| {
            AvengerChartError::InternalError("TimeFill max ordinal was not Int32".to_string())
        })?;
    if min_values.is_null(0) || max_values.is_null(0) {
        Ok(None)
    } else {
        Ok(Some((min_values.value(0), max_values.value(0))))
    }
}

async fn extent_components(
    exprs: &[LogicalExprNode],
    ctx: &DataTransformExecutionContext<'_>,
) -> Result<Vec<i32>, AvengerChartError> {
    let params = params_to_datafusion(ctx.params);
    let scalars = eval_to_scalars(
        exprs
            .iter()
            .map(|expr| expr.to_default_expr(ctx.session_context))
            .collect::<Result<Vec<_>, _>>()?,
        Some(ctx.session_context),
        params.as_ref(),
    )
    .await
    .map_err(|err| {
        AvengerChartError::InvalidArgument(format!(
            "TimeFill extent components must evaluate to integer scalars: {err}"
        ))
    })?;
    scalars.into_iter().map(scalar_to_i32).collect()
}

fn scalar_to_i32(value: ScalarValue) -> Result<i32, AvengerChartError> {
    match value {
        ScalarValue::Int8(Some(value)) => Ok(i32::from(value)),
        ScalarValue::Int16(Some(value)) => Ok(i32::from(value)),
        ScalarValue::Int32(Some(value)) => Ok(value),
        ScalarValue::Int64(Some(value)) => i32::try_from(value).map_err(|_| {
            AvengerChartError::InvalidArgument(
                "TimeFill extent integer component does not fit in Int32".to_string(),
            )
        }),
        ScalarValue::UInt8(Some(value)) => Ok(i32::from(value)),
        ScalarValue::UInt16(Some(value)) => Ok(i32::from(value)),
        ScalarValue::UInt32(Some(value)) => i32::try_from(value).map_err(|_| {
            AvengerChartError::InvalidArgument(
                "TimeFill extent integer component does not fit in Int32".to_string(),
            )
        }),
        ScalarValue::UInt64(Some(value)) => i32::try_from(value).map_err(|_| {
            AvengerChartError::InvalidArgument(
                "TimeFill extent integer component does not fit in Int32".to_string(),
            )
        }),
        other if other.is_null() => Err(AvengerChartError::InvalidArgument(
            "TimeFill extent components must not evaluate to null".to_string(),
        )),
        other => Err(AvengerChartError::InvalidArgument(format!(
            "TimeFill extent components must be integer scalars, got {other:?}"
        ))),
    }
}

fn spine_dataframe(
    ctx: &datafusion::prelude::SessionContext,
    hierarchy: &TimeFillHierarchy,
    start: i32,
    end: i32,
    domain_level_names: &[String],
) -> Result<DataFrame, AvengerChartError> {
    let ordinals = (start..=end).collect::<Vec<_>>();
    let mut component_columns = vec![Vec::<i32>::new(); domain_level_names.len()];
    for ordinal in &ordinals {
        for (index, component) in hierarchy
            .components_for_ordinal(*ordinal)
            .into_iter()
            .enumerate()
        {
            component_columns[index].push(component);
        }
    }

    let mut fields = vec![Field::new(SPINE_ORDINAL, DataType::Int32, false)];
    let mut arrays = vec![Arc::new(Int32Array::from(ordinals)) as ArrayRef];
    for (name, values) in domain_level_names.iter().zip(component_columns.into_iter()) {
        fields.push(Field::new(name, DataType::Int32, false));
        arrays.push(Arc::new(Int32Array::from(values)) as ArrayRef);
    }
    let batch = RecordBatch::try_new(Arc::new(Schema::new(fields)), arrays)
        .map_err(AvengerChartError::ArrowError)?;
    ctx.read_batch(batch)
        .map_err(AvengerChartError::DataFusionError)
}

fn output_projection(
    original_names: &[String],
    spec: &CompiledTimeFillTransform,
    domain_level_names: &[String],
    domain_group_names: &[String],
    ctx: &DataTransformExecutionContext<'_>,
) -> Result<Vec<Expr>, AvengerChartError> {
    let mut projection = Vec::new();
    let group_sources = spec
        .group_by
        .iter()
        .map(|expr| {
            Ok(simple_column_name(
                &expr.to_default_expr(ctx.session_context)?,
            ))
        })
        .collect::<Result<Vec<_>, AvengerChartError>>()?;

    for name in original_names {
        if name == &spec.value_name {
            continue;
        }
        if let Some(index) = spec
            .levels
            .levels
            .iter()
            .position(|level| level.key_name == *name)
        {
            projection.push(coalesce_expr(col(name), col(&domain_level_names[index])).alias(name));
            continue;
        }
        if let Some(index) = group_sources
            .iter()
            .position(|source| source.as_deref() == Some(name.as_str()))
        {
            projection.push(coalesce_expr(col(name), col(&domain_group_names[index])).alias(name));
            continue;
        }
        projection.push(col(name).alias(name));
    }

    projection.push(
        coalesce_expr(
            col(INPUT_VALUE),
            spec.fill_value.to_default_expr(ctx.session_context)?,
        )
        .alias(&spec.value_name),
    );
    if let Some(flag_name) = &spec.flag_name {
        projection.push(col(PRESENT).is_null().alias(flag_name));
    }
    Ok(projection)
}

fn join_predicates(group_names: &[String], domain_group_names: &[String]) -> Vec<Expr> {
    let mut predicates = vec![binary_expr(
        col(SPINE_ORDINAL),
        Operator::IsNotDistinctFrom,
        col(INPUT_ORDINAL),
    )];
    predicates.extend(
        domain_group_names
            .iter()
            .zip(group_names.iter())
            .map(|(domain, input)| {
                binary_expr(col(domain), Operator::IsNotDistinctFrom, col(input))
            }),
    );
    predicates
}

fn coalesce_expr(value: Expr, fallback: Expr) -> Expr {
    when(value.clone().is_null(), fallback)
        .otherwise(value)
        .expect("valid timefill coalesce expression")
}

fn domain_level_names(len: usize) -> Vec<String> {
    (0..len)
        .map(|index| format!("__avenger_timefill_domain_level_{index}"))
        .collect()
}

fn group_hidden_names(len: usize) -> Vec<String> {
    (0..len)
        .map(|index| format!("__avenger_timefill_input_group_{index}"))
        .collect()
}

fn group_domain_names(len: usize) -> Vec<String> {
    (0..len)
        .map(|index| format!("__avenger_timefill_domain_group_{index}"))
        .collect()
}

fn validate_quarter(quarter: i32) -> Result<(), AvengerChartError> {
    if (1..=4).contains(&quarter) {
        Ok(())
    } else {
        Err(AvengerChartError::InvalidArgument(format!(
            "TimeFill quarter component {quarter} is outside 1..=4"
        )))
    }
}

fn validate_month(month: i32) -> Result<(), AvengerChartError> {
    if (1..=12).contains(&month) {
        Ok(())
    } else {
        Err(AvengerChartError::InvalidArgument(format!(
            "TimeFill month component {month} is outside 1..=12"
        )))
    }
}

fn validate_quarter_month(quarter: i32, month: i32) -> Result<(), AvengerChartError> {
    validate_quarter(quarter)?;
    validate_month(month)?;
    let expected = quarter_for_month(month);
    if quarter == expected {
        Ok(())
    } else {
        Err(AvengerChartError::InvalidArgument(format!(
            "TimeFill quarter component {quarter} is inconsistent with month component {month}"
        )))
    }
}

fn validate_ymd(year: i32, month: i32, day: i32) -> Result<(), AvengerChartError> {
    validate_month(month)?;
    NaiveDate::from_ymd_opt(year, month as u32, day as u32)
        .map(|_| ())
        .ok_or_else(|| {
            AvengerChartError::InvalidArgument(format!(
                "TimeFill date components year={year}, month={month}, day={day} are invalid"
            ))
        })
}

fn epoch_day(year: i32, month: i32, day: i32) -> Result<i32, AvengerChartError> {
    validate_ymd(year, month, day)?;
    let date = NaiveDate::from_ymd_opt(year, month as u32, day as u32).expect("validated date");
    Ok(date.num_days_from_ce() - epoch_date().num_days_from_ce())
}

fn epoch_day_from_year_day(year: i32, day_of_year: i32) -> Result<i32, AvengerChartError> {
    let date = NaiveDate::from_yo_opt(year, day_of_year as u32).ok_or_else(|| {
        AvengerChartError::InvalidArgument(format!(
            "TimeFill day-of-year component {day_of_year} is invalid for year {year}"
        ))
    })?;
    Ok(date.num_days_from_ce() - epoch_date().num_days_from_ce())
}

fn date_from_epoch_day(day: i32) -> NaiveDate {
    NaiveDate::from_num_days_from_ce_opt(epoch_date().num_days_from_ce() + day)
        .expect("valid generated epoch day")
}

fn epoch_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(1970, 1, 1).expect("valid epoch date")
}

fn quarter_for_month(month: i32) -> i32 {
    (month - 1).div_euclid(3) + 1
}
