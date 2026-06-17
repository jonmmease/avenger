use std::collections::BTreeSet;

use datafusion::{
    arrow::datatypes::DataType,
    common::{Column, ScalarValue},
    logical_expr::{Expr, expr::Placeholder, lit},
    prelude::col,
};
use datafusion_common::tree_node::{Transformed, TreeNode, TreeNodeRecursion};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::{
    AvengerChartError, Axis, ChannelExpr, ChannelValue, ConditionalValue, CoordinationScope,
    DefaultLogicalExprNodeExt, DomainCoordination, IntoExpr, Legend, Maybe, NestedBandSpec,
    RadiusExpression, ScaleConfigSpec, ScaleDefaultDomain, ScaleDomain, ScaleOrderingSpec,
    ScaleRange, SerializableExpr, scale_domain::DomainExpr, simplify_to_scalar_sync,
};

const REPEAT_PLACEHOLDER_PREFIX: &str = "$__repeat_";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RepeatPlaceholderKind {
    RowExpr,
    ColumnExpr,
    ItemExpr,
    RowName,
    ColumnName,
    ItemName,
    RowIndex,
    ColumnIndex,
    ItemIndex,
    CellId,
    RowId,
    ColumnId,
    ItemId,
    RowTitle,
    ColumnTitle,
    ItemTitle,
}

impl RepeatPlaceholderKind {
    fn suffix(self) -> &'static str {
        match self {
            Self::RowExpr => "row_expr",
            Self::ColumnExpr => "column_expr",
            Self::ItemExpr => "item_expr",
            Self::RowName => "row_name",
            Self::ColumnName => "column_name",
            Self::ItemName => "item_name",
            Self::RowIndex => "row_index",
            Self::ColumnIndex => "column_index",
            Self::ItemIndex => "item_index",
            Self::CellId => "cell_id",
            Self::RowId => "row_id",
            Self::ColumnId => "column_id",
            Self::ItemId => "item_id",
            Self::RowTitle => "row_title",
            Self::ColumnTitle => "column_title",
            Self::ItemTitle => "item_title",
        }
    }

    fn from_suffix(suffix: &str) -> Option<Self> {
        Some(match suffix {
            "row_expr" => Self::RowExpr,
            "column_expr" => Self::ColumnExpr,
            "item_expr" => Self::ItemExpr,
            "row_name" => Self::RowName,
            "column_name" => Self::ColumnName,
            "item_name" => Self::ItemName,
            "row_index" => Self::RowIndex,
            "column_index" => Self::ColumnIndex,
            "item_index" => Self::ItemIndex,
            "cell_id" => Self::CellId,
            "row_id" => Self::RowId,
            "column_id" => Self::ColumnId,
            "item_id" => Self::ItemId,
            "row_title" => Self::RowTitle,
            "column_title" => Self::ColumnTitle,
            "item_title" => Self::ItemTitle,
            _ => return None,
        })
    }

    fn data_type(self) -> Option<DataType> {
        match self {
            Self::RowExpr | Self::ColumnExpr | Self::ItemExpr => None,
            Self::RowIndex | Self::ColumnIndex | Self::ItemIndex => Some(DataType::Int64),
            Self::RowName | Self::ColumnName | Self::ItemName => Some(DataType::Utf8),
            Self::CellId
            | Self::RowId
            | Self::ColumnId
            | Self::ItemId
            | Self::RowTitle
            | Self::ColumnTitle
            | Self::ItemTitle => Some(DataType::Utf8),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepeatTypeHint {
    Quantitative,
    Temporal,
    Ordinal,
    Nominal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepeatDomainCoordination {
    Independent,
    ByVariable { scope: CoordinationScope },
}

impl Default for RepeatDomainCoordination {
    fn default() -> Self {
        Self::Independent
    }
}

impl RepeatDomainCoordination {
    pub fn by_variable(scope: CoordinationScope) -> Self {
        Self::ByVariable {
            scope: scope.to_normalized(),
        }
    }
}

#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepeatVariable {
    pub id: String,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub expr: LogicalExprNode,
    pub title: Option<String>,
    pub type_hint: Option<RepeatTypeHint>,
}

impl RepeatVariable {
    pub fn new(id: impl Into<String>, expr: impl IntoExpr) -> Self {
        let id = id.into();
        let expr = expr.into_expr();
        Self {
            title: None,
            id,
            expr: LogicalExprNode::from_default_expr(expr)
                .expect("Failed to serialize repeat variable expression"),
            type_hint: None,
        }
    }

    pub fn field(name: impl Into<String>) -> Self {
        let name = name.into();
        Self::new(name.clone(), datafusion::prelude::col(name))
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn type_hint(mut self, type_hint: RepeatTypeHint) -> Self {
        self.type_hint = Some(type_hint);
        self
    }

    pub fn validate(&self) -> Result<(), AvengerChartError> {
        validate_repeat_id(&self.id)
    }

    pub fn resolve(
        &self,
        ctx: &datafusion::prelude::SessionContext,
    ) -> Result<ResolvedRepeatVariable, AvengerChartError> {
        self.validate()?;
        let expr = self.expr.to_default_expr(ctx)?;
        Ok(ResolvedRepeatVariable {
            id: self.id.clone(),
            title: self.title.clone().unwrap_or_else(|| self.id.clone()),
            expr,
            type_hint: self.type_hint,
        })
    }
}

#[derive(Clone, Debug)]
pub struct ResolvedRepeatVariable {
    pub id: String,
    pub expr: Expr,
    pub title: String,
    pub type_hint: Option<RepeatTypeHint>,
}

#[derive(Clone, Debug, Default)]
pub struct RepeatContext {
    pub row: Option<ResolvedRepeatVariable>,
    pub column: Option<ResolvedRepeatVariable>,
    pub item: Option<ResolvedRepeatVariable>,
    pub row_index: Option<usize>,
    pub column_index: Option<usize>,
    pub item_index: Option<usize>,
    pub row_count: usize,
    pub column_count: usize,
    pub item_count: Option<usize>,
    pub domain_coordination: RepeatDomainCoordination,
    pub matrix_axis_defaults: bool,
}

impl RepeatContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_row(
        mut self,
        variable: ResolvedRepeatVariable,
        index: usize,
        count: usize,
    ) -> Self {
        self.row = Some(variable);
        self.row_index = Some(index);
        self.row_count = count;
        self
    }

    pub fn with_column(
        mut self,
        variable: ResolvedRepeatVariable,
        index: usize,
        count: usize,
    ) -> Self {
        self.column = Some(variable);
        self.column_index = Some(index);
        self.column_count = count;
        self
    }

    pub fn with_item(
        mut self,
        variable: ResolvedRepeatVariable,
        index: usize,
        count: usize,
    ) -> Self {
        self.item = Some(variable);
        self.item_index = Some(index);
        self.item_count = Some(count);
        self
    }

    pub fn with_domain_coordination(mut self, coordination: RepeatDomainCoordination) -> Self {
        self.domain_coordination = coordination;
        self
    }

    pub fn with_matrix_axis_defaults(mut self, enabled: bool) -> Self {
        self.matrix_axis_defaults = enabled;
        self
    }
}

pub fn repeat_placeholder_id(kind: RepeatPlaceholderKind) -> String {
    format!("{REPEAT_PLACEHOLDER_PREFIX}{}", kind.suffix())
}

pub fn repeat_placeholder_kind_from_id(id: &str) -> Option<RepeatPlaceholderKind> {
    id.strip_prefix(REPEAT_PLACEHOLDER_PREFIX)
        .and_then(RepeatPlaceholderKind::from_suffix)
}

fn repeat_placeholder_expr(kind: RepeatPlaceholderKind) -> Expr {
    Expr::Placeholder(Placeholder {
        id: repeat_placeholder_id(kind),
        data_type: kind.data_type(),
    })
}

pub fn row() -> ChannelExpr {
    ChannelExpr::scaled(repeat_placeholder_expr(RepeatPlaceholderKind::RowExpr))
}

/// String placeholder for the current repeated row variable id.
///
/// Use this in field-name contexts such as `col(...)`, `event::datum(...)`,
/// `nested([...])`, aggregate group keys, and source-column selection
/// dimensions. It resolves to the repeat variable `id`, so it is intended for
/// field-backed variables such as `RepeatVariable::field("name")` or
/// `RepeatVariable::new("name", col("name"))`.
pub fn row_name() -> String {
    repeat_placeholder_id(RepeatPlaceholderKind::RowName)
}

pub fn column() -> ChannelExpr {
    ChannelExpr::scaled(repeat_placeholder_expr(RepeatPlaceholderKind::ColumnExpr))
}

/// String placeholder for the current repeated column variable id.
///
/// Use this in field-name contexts such as `col(...)`, `event::datum(...)`,
/// `nested([...])`, aggregate group keys, and source-column selection
/// dimensions. It resolves to the repeat variable `id`, so it is intended for
/// field-backed variables such as `RepeatVariable::field("name")` or
/// `RepeatVariable::new("name", col("name"))`.
pub fn column_name() -> String {
    repeat_placeholder_id(RepeatPlaceholderKind::ColumnName)
}

pub fn item() -> ChannelExpr {
    ChannelExpr::scaled(repeat_placeholder_expr(RepeatPlaceholderKind::ItemExpr))
}

/// String placeholder for the current repeated wrap/item variable id.
///
/// Use this in field-name contexts such as `col(...)`, `event::datum(...)`,
/// `nested([...])`, aggregate group keys, and source-column selection
/// dimensions. It resolves to the repeat variable `id`, so it is intended for
/// field-backed variables such as `RepeatVariable::field("name")` or
/// `RepeatVariable::new("name", col("name"))`.
pub fn item_name() -> String {
    repeat_placeholder_id(RepeatPlaceholderKind::ItemName)
}

pub fn row_index() -> Expr {
    repeat_placeholder_expr(RepeatPlaceholderKind::RowIndex)
}

pub fn column_index() -> Expr {
    repeat_placeholder_expr(RepeatPlaceholderKind::ColumnIndex)
}

pub fn item_index() -> Expr {
    repeat_placeholder_expr(RepeatPlaceholderKind::ItemIndex)
}

pub fn cell_id() -> Expr {
    repeat_placeholder_expr(RepeatPlaceholderKind::CellId)
}

pub fn row_id() -> Expr {
    repeat_placeholder_expr(RepeatPlaceholderKind::RowId)
}

pub fn column_id() -> Expr {
    repeat_placeholder_expr(RepeatPlaceholderKind::ColumnId)
}

pub fn item_id() -> Expr {
    repeat_placeholder_expr(RepeatPlaceholderKind::ItemId)
}

pub fn row_title() -> Expr {
    repeat_placeholder_expr(RepeatPlaceholderKind::RowTitle)
}

pub fn column_title() -> Expr {
    repeat_placeholder_expr(RepeatPlaceholderKind::ColumnTitle)
}

pub fn item_title() -> Expr {
    repeat_placeholder_expr(RepeatPlaceholderKind::ItemTitle)
}

pub fn current_cell_predicate() -> Expr {
    col("cell_id").eq(cell_id())
}

pub fn collect_repeat_placeholder_kinds(
    expr: &Expr,
) -> Result<BTreeSet<RepeatPlaceholderKind>, AvengerChartError> {
    let mut kinds = BTreeSet::new();
    expr.apply(|candidate| {
        if let Expr::Placeholder(placeholder) = candidate
            && let Some(kind) = repeat_placeholder_kind_from_id(&placeholder.id)
        {
            kinds.insert(kind);
        }
        if let Expr::Column(column) = candidate {
            collect_repeat_name_placeholder_kinds(&column.name, &mut kinds);
        }
        if let Expr::Literal(ScalarValue::Utf8(Some(value)), _) = candidate {
            collect_repeat_name_placeholder_kinds(value, &mut kinds);
        }
        Ok(TreeNodeRecursion::Continue)
    })
    .map_err(AvengerChartError::DataFusionError)?;
    Ok(kinds)
}

fn collect_repeat_name_placeholder_kinds(value: &str, kinds: &mut BTreeSet<RepeatPlaceholderKind>) {
    for kind in [
        RepeatPlaceholderKind::RowName,
        RepeatPlaceholderKind::ColumnName,
        RepeatPlaceholderKind::ItemName,
    ] {
        if value.contains(&repeat_placeholder_id(kind)) {
            kinds.insert(kind);
        }
    }
}

pub fn resolve_repeat_placeholders(
    expr: Expr,
    ctx: &RepeatContext,
) -> Result<Expr, AvengerChartError> {
    expr.transform(|candidate| {
        if let Expr::Placeholder(placeholder) = &candidate
            && let Some(kind) = repeat_placeholder_kind_from_id(&placeholder.id)
        {
            let replacement = repeat_placeholder_replacement(kind, ctx)
                .map_err(|err| datafusion::error::DataFusionError::Plan(err.to_string()))?;
            return Ok(Transformed::yes(replacement));
        }
        if let Expr::Column(column) = &candidate
            && let Some(name) = resolve_repeat_name_placeholders(&column.name, ctx)
                .map_err(|err| datafusion::error::DataFusionError::Plan(err.to_string()))?
        {
            return Ok(Transformed::yes(Expr::Column(Column {
                relation: column.relation.clone(),
                name,
                spans: column.spans.clone(),
            })));
        }
        if let Expr::Literal(ScalarValue::Utf8(Some(value)), metadata) = &candidate
            && let Some(value) = resolve_repeat_name_placeholders(value, ctx)
                .map_err(|err| datafusion::error::DataFusionError::Plan(err.to_string()))?
        {
            return Ok(Transformed::yes(Expr::Literal(
                ScalarValue::Utf8(Some(value)),
                metadata.clone(),
            )));
        }

        Ok(Transformed::no(candidate))
    })
    .map(|transformed| transformed.data)
    .map_err(AvengerChartError::DataFusionError)
}

pub fn resolve_repeat_channel_expr(
    value: ChannelExpr,
    ctx: &RepeatContext,
) -> Result<ChannelExpr, AvengerChartError> {
    let origin = repeat_domain_origin(value.data_expr())?;
    let expr = resolve_repeat_placeholders(value.clone().into_data_expr(), ctx)?;
    let channel_value = resolve_repeat_channel_value(value.into_channel_value(), ctx)?;
    let value = ChannelExpr::new(expr, channel_value);
    apply_repeat_domain_coordination(value, origin, ctx)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum RepeatDomainOrigin {
    Row,
    Column,
    Item,
}

fn repeat_domain_origin(expr: &Expr) -> Result<Option<RepeatDomainOrigin>, AvengerChartError> {
    let kinds = collect_repeat_placeholder_kinds(expr)?;
    let mut origins = Vec::new();
    if kinds.contains(&RepeatPlaceholderKind::RowExpr)
        || kinds.contains(&RepeatPlaceholderKind::RowName)
    {
        origins.push(RepeatDomainOrigin::Row);
    }
    if kinds.contains(&RepeatPlaceholderKind::ColumnExpr)
        || kinds.contains(&RepeatPlaceholderKind::ColumnName)
    {
        origins.push(RepeatDomainOrigin::Column);
    }
    if kinds.contains(&RepeatPlaceholderKind::ItemExpr)
        || kinds.contains(&RepeatPlaceholderKind::ItemName)
    {
        origins.push(RepeatDomainOrigin::Item);
    }
    Ok(if origins.len() == 1 {
        Some(origins[0])
    } else {
        None
    })
}

fn apply_repeat_domain_coordination(
    value: ChannelExpr,
    origin: Option<RepeatDomainOrigin>,
    ctx: &RepeatContext,
) -> Result<ChannelExpr, AvengerChartError> {
    let Some(origin) = origin else {
        return Ok(value);
    };
    let RepeatDomainCoordination::ByVariable { scope } = ctx.domain_coordination else {
        return Ok(value);
    };
    let group = match origin {
        RepeatDomainOrigin::Row => ctx
            .row
            .as_ref()
            .map(|variable| variable.id.as_str())
            .ok_or_else(|| missing_repeat_variable_error("row"))?,
        RepeatDomainOrigin::Column => ctx
            .column
            .as_ref()
            .map(|variable| variable.id.as_str())
            .ok_or_else(|| missing_repeat_variable_error("column"))?,
        RepeatDomainOrigin::Item => ctx
            .item
            .as_ref()
            .map(|variable| variable.id.as_str())
            .ok_or_else(|| missing_repeat_variable_error("item"))?,
    };
    let generated = DomainCoordination::named(scope, group)?;
    validate_or_apply_generated_domain_coordination(value, generated)
}

fn validate_or_apply_generated_domain_coordination(
    value: ChannelExpr,
    generated: DomainCoordination,
) -> Result<ChannelExpr, AvengerChartError> {
    let existing = value.get_domain_coordination().cloned();
    let Some(existing) = existing else {
        return Ok(value.with_domain_coordination(generated));
    };

    if existing.group == generated.group && existing.scope.to_level() <= generated.scope.to_level()
    {
        return Ok(value);
    }

    Err(AvengerChartError::InvalidArgument(format!(
        "Repeat-generated domain coordination target {:?} conflicts with authored channel domain coordination {:?}",
        generated, existing
    )))
}

fn missing_repeat_variable_error(role: &str) -> AvengerChartError {
    AvengerChartError::InvalidArgument(format!(
        "repeat::{role}() was used without a resolved repeat {role} variable"
    ))
}

pub fn resolve_repeat_channel_value(
    value: ChannelValue,
    ctx: &RepeatContext,
) -> Result<ChannelValue, AvengerChartError> {
    let origin = repeat_domain_origin_channel_value(&value)?;
    let proto = |expr: Expr| LogicalExprNode::from_default_expr(expr);
    let resolved: ChannelValue = match value {
        ChannelValue::Scaled {
            expr,
            scale_name,
            position_boundary,
            scale_config,
            nested_band_config,
            legend_config,
            axis_config,
            domain_coordination,
            transform_scope,
        } => ChannelValue::Scaled {
            expr: resolve_expr_node(expr, ctx)?,
            scale_name,
            position_boundary,
            scale_config: resolve_scale_config(scale_config, ctx)?,
            nested_band_config: resolve_nested_band_config(nested_band_config, ctx)?,
            legend_config: resolve_legend_config(legend_config, ctx)?,
            axis_config: resolve_axis_config(axis_config, ctx)?,
            domain_coordination,
            transform_scope,
        },
        ChannelValue::Value { expr } => ChannelValue::Value {
            expr: resolve_expr_node(expr, ctx)?,
        },
        ChannelValue::Conditional {
            conditions,
            otherwise,
            scale_config,
            nested_band_config,
            legend_config,
            axis_config,
            domain_coordination,
            transform_scope,
        } => {
            let conditions = conditions
                .into_iter()
                .map(|(condition, value)| {
                    Ok((
                        proto(resolve_repeat_placeholders(
                            condition.to_default_expr(&session_context())?,
                            ctx,
                        )?)?,
                        resolve_repeat_conditional_value(value, ctx)?,
                    ))
                })
                .collect::<Result<Vec<_>, AvengerChartError>>()?;
            let otherwise = resolve_repeat_conditional_value(otherwise, ctx)?;
            ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config: resolve_scale_config(scale_config, ctx)?,
                nested_band_config: resolve_nested_band_config(nested_band_config, ctx)?,
                legend_config: resolve_legend_config(legend_config, ctx)?,
                axis_config: resolve_axis_config(axis_config, ctx)?,
                domain_coordination,
                transform_scope,
            }
        }
    };
    apply_repeat_domain_coordination_to_channel_value(resolved, origin, ctx)
}

fn repeat_domain_origin_channel_value(
    value: &ChannelValue,
) -> Result<Option<RepeatDomainOrigin>, AvengerChartError> {
    let ctx = session_context();
    match value {
        ChannelValue::Scaled { expr, .. } | ChannelValue::Value { expr } => {
            repeat_domain_origin(&expr.to_default_expr(&ctx)?)
        }
        ChannelValue::Conditional {
            conditions,
            otherwise,
            ..
        } => {
            let mut origins = BTreeSet::new();
            for (_, value) in conditions {
                if let Some(origin) = repeat_domain_origin_conditional_value(value)? {
                    origins.insert(origin);
                }
            }
            if let Some(origin) = repeat_domain_origin_conditional_value(otherwise)? {
                origins.insert(origin);
            }
            Ok(if origins.len() == 1 {
                origins.iter().next().copied()
            } else {
                None
            })
        }
    }
}

fn repeat_domain_origin_conditional_value(
    value: &ConditionalValue,
) -> Result<Option<RepeatDomainOrigin>, AvengerChartError> {
    let ctx = session_context();
    match value {
        ConditionalValue::Scaled { expr } | ConditionalValue::Value { expr } => {
            repeat_domain_origin(&expr.to_default_expr(&ctx)?)
        }
    }
}

fn apply_repeat_domain_coordination_to_channel_value(
    value: ChannelValue,
    origin: Option<RepeatDomainOrigin>,
    ctx: &RepeatContext,
) -> Result<ChannelValue, AvengerChartError> {
    let value = ChannelExpr::new(lit(0), value);
    apply_repeat_domain_coordination(value, origin, ctx).map(ChannelExpr::into_channel_value)
}

pub fn evaluate_repeat_predicate(
    expr: Expr,
    ctx: &RepeatContext,
) -> Result<bool, AvengerChartError> {
    let expr = resolve_repeat_placeholders(expr, ctx)?;
    let value = simplify_to_scalar_sync(expr).map_err(AvengerChartError::DataFusionError)?;
    match value {
        ScalarValue::Boolean(Some(value)) => Ok(value),
        other => Err(AvengerChartError::InvalidArgument(format!(
            "Repeat predicate must evaluate to a non-null boolean, got {other}"
        ))),
    }
}

fn resolve_repeat_conditional_value(
    value: ConditionalValue,
    ctx: &RepeatContext,
) -> Result<ConditionalValue, AvengerChartError> {
    Ok(match value {
        ConditionalValue::Scaled { expr } => ConditionalValue::Scaled {
            expr: resolve_expr_node(expr, ctx)?,
        },
        ConditionalValue::Value { expr } => ConditionalValue::Value {
            expr: resolve_expr_node(expr, ctx)?,
        },
    })
}

fn resolve_expr_node(
    node: LogicalExprNode,
    ctx: &RepeatContext,
) -> Result<LogicalExprNode, AvengerChartError> {
    LogicalExprNode::from_default_expr(resolve_repeat_placeholders(
        node.to_default_expr(&session_context())?,
        ctx,
    )?)
}

fn resolve_axis_config(
    axis_config: Option<Box<dyn Axis>>,
    ctx: &RepeatContext,
) -> Result<Option<Box<dyn Axis>>, AvengerChartError> {
    axis_config
        .map(|axis| axis.map_exprs(&mut |expr| resolve_repeat_placeholders(expr, ctx)))
        .transpose()
}

fn resolve_scale_config(
    scale_config: Option<Box<ScaleConfigSpec>>,
    ctx: &RepeatContext,
) -> Result<Option<Box<ScaleConfigSpec>>, AvengerChartError> {
    scale_config
        .map(|config| resolve_scale_config_spec(*config, ctx).map(Box::new))
        .transpose()
}

fn resolve_nested_band_config(
    nested_band_config: Option<Box<NestedBandSpec>>,
    ctx: &RepeatContext,
) -> Result<Option<Box<NestedBandSpec>>, AvengerChartError> {
    nested_band_config
        .map(|config| {
            let mut config = *config;
            config.source_columns = config
                .source_columns
                .into_iter()
                .map(|column| Ok(resolve_repeat_name_placeholders(&column, ctx)?.unwrap_or(column)))
                .collect::<Result<_, AvengerChartError>>()?;
            for level in config.levels.values_mut() {
                level.domain = resolve_maybe(std::mem::take(&mut level.domain), |domain| {
                    resolve_scale_domain(domain, ctx)
                })?;
                level.ordering = resolve_maybe(std::mem::take(&mut level.ordering), |ordering| {
                    resolve_scale_ordering(ordering, ctx)
                })?;
                level.axis_config = resolve_axis_config(level.axis_config.take(), ctx)?;
            }
            Ok(Box::new(config))
        })
        .transpose()
}

fn resolve_scale_config_spec(
    mut config: ScaleConfigSpec,
    ctx: &RepeatContext,
) -> Result<ScaleConfigSpec, AvengerChartError> {
    config.domain = resolve_maybe(config.domain, |domain| resolve_scale_domain(domain, ctx))?;
    config.range = resolve_maybe(config.range, |range| resolve_scale_range(range, ctx))?;
    config.ordering = resolve_maybe(config.ordering, |ordering| {
        resolve_scale_ordering(ordering, ctx)
    })?;
    config.options = config
        .options
        .into_iter()
        .map(|(key, node)| Ok((key, resolve_expr_node(node, ctx)?)))
        .collect::<Result<_, AvengerChartError>>()?;
    Ok(config)
}

fn resolve_scale_ordering(
    mut ordering: ScaleOrderingSpec,
    ctx: &RepeatContext,
) -> Result<ScaleOrderingSpec, AvengerChartError> {
    ordering.order_expr = ordering
        .order_expr
        .map(|node| resolve_expr_node(node, ctx))
        .transpose()?;
    Ok(ordering)
}

fn resolve_scale_domain(
    mut domain: ScaleDomain,
    ctx: &RepeatContext,
) -> Result<ScaleDomain, AvengerChartError> {
    domain.default_domain = match domain.default_domain {
        ScaleDefaultDomain::Interval(start, end) => ScaleDefaultDomain::Interval(
            resolve_expr_node(start, ctx)?,
            Box::new(resolve_expr_node(*end, ctx)?),
        ),
        ScaleDefaultDomain::Discrete(values) => ScaleDefaultDomain::Discrete(
            values
                .into_iter()
                .map(|node| resolve_expr_node(node, ctx))
                .collect::<Result<_, AvengerChartError>>()?,
        ),
        ScaleDefaultDomain::DomainExprs(exprs) => ScaleDefaultDomain::DomainExprs(
            exprs
                .into_iter()
                .map(|domain_expr| resolve_domain_expr(domain_expr, ctx))
                .collect::<Result<_, AvengerChartError>>()?,
        ),
        ScaleDefaultDomain::NoDefault => ScaleDefaultDomain::NoDefault,
    };
    domain.raw_domain = domain
        .raw_domain
        .map(|node| resolve_expr_node(node, ctx))
        .transpose()?;
    Ok(domain)
}

fn resolve_domain_expr(
    domain_expr: DomainExpr,
    ctx: &RepeatContext,
) -> Result<DomainExpr, AvengerChartError> {
    Ok(DomainExpr {
        dataframe: domain_expr.dataframe,
        expr: resolve_expr_node(domain_expr.expr, ctx)?,
        radius: domain_expr
            .radius
            .map(|radius| resolve_radius_expression(radius, ctx))
            .transpose()?,
    })
}

fn resolve_radius_expression(
    radius: RadiusExpression,
    ctx: &RepeatContext,
) -> Result<RadiusExpression, AvengerChartError> {
    Ok(match radius {
        RadiusExpression::Symmetric(node) => {
            RadiusExpression::Symmetric(resolve_expr_node(node, ctx)?)
        }
        RadiusExpression::Asymmetric { lower, upper } => RadiusExpression::Asymmetric {
            lower: resolve_expr_node(lower, ctx)?,
            upper: resolve_expr_node(upper, ctx)?,
        },
    })
}

fn resolve_scale_range(
    range: ScaleRange,
    ctx: &RepeatContext,
) -> Result<ScaleRange, AvengerChartError> {
    Ok(match range {
        ScaleRange::Numeric(start, end) => ScaleRange::Numeric(
            resolve_expr_node(start, ctx)?,
            Box::new(resolve_expr_node(*end, ctx)?),
        ),
        ScaleRange::Discrete(values) => ScaleRange::Discrete(values),
        ScaleRange::Color(colors) => ScaleRange::Color(colors),
    })
}

fn resolve_legend_config(
    legend_config: Option<Box<Legend>>,
    ctx: &RepeatContext,
) -> Result<Option<Box<Legend>>, AvengerChartError> {
    legend_config
        .map(|legend| resolve_legend(*legend, ctx).map(Box::new))
        .transpose()
}

fn resolve_legend(mut legend: Legend, ctx: &RepeatContext) -> Result<Legend, AvengerChartError> {
    legend.visible = resolve_maybe_optional_expr(legend.visible, ctx)?;
    legend.title = resolve_maybe_optional_expr(legend.title, ctx)?;
    legend.position = resolve_maybe_optional_expr(legend.position, ctx)?;
    legend.orientation = resolve_maybe_optional_expr(legend.orientation, ctx)?;
    legend.symbol_size = resolve_maybe_optional_expr(legend.symbol_size, ctx)?;
    legend.gradient_thickness = resolve_maybe_optional_expr(legend.gradient_thickness, ctx)?;
    legend.columns = resolve_maybe_optional_expr(legend.columns, ctx)?;
    legend.label_limit = resolve_maybe_optional_expr(legend.label_limit, ctx)?;
    legend.format_number = resolve_maybe_optional_expr(legend.format_number, ctx)?;
    legend.background_fill = resolve_maybe_optional_expr(legend.background_fill, ctx)?;
    legend.background_stroke = resolve_maybe_optional_expr(legend.background_stroke, ctx)?;
    legend.background_stroke_width =
        resolve_maybe_optional_expr(legend.background_stroke_width, ctx)?;
    legend.background_corner_radius =
        resolve_maybe_optional_expr(legend.background_corner_radius, ctx)?;
    legend.background_padding = resolve_maybe_optional_expr(legend.background_padding, ctx)?;
    legend.order = resolve_maybe_optional_expr(legend.order, ctx)?;
    legend.title_color = resolve_maybe_optional_expr(legend.title_color, ctx)?;
    legend.label_color = resolve_maybe_optional_expr(legend.label_color, ctx)?;
    legend.title_font_family = resolve_maybe_optional_expr(legend.title_font_family, ctx)?;
    legend.title_font_size = resolve_maybe_optional_expr(legend.title_font_size, ctx)?;
    legend.title_font_weight = resolve_maybe_optional_expr(legend.title_font_weight, ctx)?;
    legend.label_font_family = resolve_maybe_optional_expr(legend.label_font_family, ctx)?;
    legend.label_font_size = resolve_maybe_optional_expr(legend.label_font_size, ctx)?;
    legend.label_font_weight = resolve_maybe_optional_expr(legend.label_font_weight, ctx)?;
    legend.tick_font_family = resolve_maybe_optional_expr(legend.tick_font_family, ctx)?;
    legend.tick_font_size = resolve_maybe_optional_expr(legend.tick_font_size, ctx)?;
    legend.tick_font_weight = resolve_maybe_optional_expr(legend.tick_font_weight, ctx)?;
    legend.tick_color = resolve_maybe_optional_expr(legend.tick_color, ctx)?;
    Ok(legend)
}

fn resolve_maybe_optional_expr(
    value: Maybe<Option<LogicalExprNode>>,
    ctx: &RepeatContext,
) -> Result<Maybe<Option<LogicalExprNode>>, AvengerChartError> {
    match value {
        Maybe::Unset => Ok(Maybe::Unset),
        Maybe::Set(None) => Ok(Maybe::Set(None)),
        Maybe::Set(Some(node)) => Ok(Maybe::Set(Some(resolve_expr_node(node, ctx)?))),
    }
}

fn resolve_maybe<T>(
    value: Maybe<T>,
    f: impl FnOnce(T) -> Result<T, AvengerChartError>,
) -> Result<Maybe<T>, AvengerChartError> {
    match value {
        Maybe::Unset => Ok(Maybe::Unset),
        Maybe::Set(value) => f(value).map(Maybe::Set),
    }
}

fn repeat_placeholder_replacement(
    kind: RepeatPlaceholderKind,
    ctx: &RepeatContext,
) -> Result<Expr, AvengerChartError> {
    match kind {
        RepeatPlaceholderKind::RowExpr => resolved_expr(ctx.row.as_ref(), "row"),
        RepeatPlaceholderKind::ColumnExpr => resolved_expr(ctx.column.as_ref(), "column"),
        RepeatPlaceholderKind::ItemExpr => resolved_expr(ctx.item.as_ref(), "item"),
        RepeatPlaceholderKind::RowName => resolved_string(ctx.row.as_ref(), "row", |v| &v.id),
        RepeatPlaceholderKind::ColumnName => {
            resolved_string(ctx.column.as_ref(), "column", |v| &v.id)
        }
        RepeatPlaceholderKind::ItemName => resolved_string(ctx.item.as_ref(), "item", |v| &v.id),
        RepeatPlaceholderKind::RowIndex => resolved_index(ctx.row_index, "row"),
        RepeatPlaceholderKind::ColumnIndex => resolved_index(ctx.column_index, "column"),
        RepeatPlaceholderKind::ItemIndex => resolved_index(ctx.item_index, "item"),
        RepeatPlaceholderKind::CellId => resolved_cell_id(ctx),
        RepeatPlaceholderKind::RowId => resolved_string(ctx.row.as_ref(), "row", |v| &v.id),
        RepeatPlaceholderKind::ColumnId => {
            resolved_string(ctx.column.as_ref(), "column", |v| &v.id)
        }
        RepeatPlaceholderKind::ItemId => resolved_string(ctx.item.as_ref(), "item", |v| &v.id),
        RepeatPlaceholderKind::RowTitle => resolved_string(ctx.row.as_ref(), "row", |v| &v.title),
        RepeatPlaceholderKind::ColumnTitle => {
            resolved_string(ctx.column.as_ref(), "column", |v| &v.title)
        }
        RepeatPlaceholderKind::ItemTitle => {
            resolved_string(ctx.item.as_ref(), "item", |v| &v.title)
        }
    }
}

pub fn resolve_repeat_name_placeholders(
    value: &str,
    ctx: &RepeatContext,
) -> Result<Option<String>, AvengerChartError> {
    let mut resolved = value.to_string();
    let mut changed = false;
    for (kind, variable, role) in [
        (RepeatPlaceholderKind::RowName, ctx.row.as_ref(), "row"),
        (
            RepeatPlaceholderKind::ColumnName,
            ctx.column.as_ref(),
            "column",
        ),
        (RepeatPlaceholderKind::ItemName, ctx.item.as_ref(), "item"),
    ] {
        let placeholder = repeat_placeholder_id(kind);
        if resolved.contains(&placeholder) {
            let variable = variable.ok_or_else(|| missing_repeat_variable_error(role))?;
            resolved = resolved.replace(&placeholder, &variable.id);
            changed = true;
        }
    }
    Ok(changed.then_some(resolved))
}

fn resolved_expr(
    variable: Option<&ResolvedRepeatVariable>,
    role: &str,
) -> Result<Expr, AvengerChartError> {
    variable
        .map(|variable| variable.expr.clone())
        .ok_or_else(|| {
            AvengerChartError::InvalidArgument(format!(
                "repeat::{role}() was used without a resolved repeat {role} variable"
            ))
        })
}

fn resolved_index(index: Option<usize>, role: &str) -> Result<Expr, AvengerChartError> {
    index
        .map(|index| lit(ScalarValue::Int64(Some(index as i64))))
        .ok_or_else(|| {
            AvengerChartError::InvalidArgument(format!(
                "repeat::{role}_index() was used without a resolved repeat {role} variable"
            ))
        })
}

fn resolved_string(
    variable: Option<&ResolvedRepeatVariable>,
    role: &str,
    f: impl FnOnce(&ResolvedRepeatVariable) -> &String,
) -> Result<Expr, AvengerChartError> {
    variable
        .map(|variable| lit(f(variable).clone()))
        .ok_or_else(|| {
            AvengerChartError::InvalidArgument(format!(
                "repeat::{role}_id/title() was used without a resolved repeat {role} variable"
            ))
        })
}

fn resolved_cell_id(ctx: &RepeatContext) -> Result<Expr, AvengerChartError> {
    let id = if let Some(item) = &ctx.item {
        format!("repeat_cell:item:{}", item.id)
    } else {
        match (ctx.row.as_ref(), ctx.column.as_ref()) {
            (Some(row), Some(column)) => {
                format!("repeat_cell:{}:{}", row.id, column.id)
            }
            (Some(row), None) => format!("repeat_cell:row:{}", row.id),
            (None, Some(column)) => format!("repeat_cell:column:{}", column.id),
            (None, None) => {
                return Err(AvengerChartError::InvalidArgument(
                    "repeat::cell_id() was used without a resolved repeat cell".to_string(),
                ));
            }
        }
    };
    Ok(lit(id))
}

fn validate_repeat_id(id: &str) -> Result<(), AvengerChartError> {
    if id.is_empty()
        || id.contains('.')
        || !id.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Repeat variable id '{id}' must be a non-empty ASCII identifier without periods"
        )));
    }
    Ok(())
}

fn session_context() -> datafusion::prelude::SessionContext {
    datafusion::prelude::SessionContext::new()
}

#[cfg(test)]
mod tests {
    use datafusion::prelude::{col, lit};
    use datafusion_proto::protobuf::LogicalExprNode;
    use std::collections::HashMap;

    use crate::{event, nested};

    use super::*;

    fn resolved(id: &str, expr: Expr, title: &str) -> ResolvedRepeatVariable {
        ResolvedRepeatVariable {
            id: id.to_string(),
            expr,
            title: title.to_string(),
            type_hint: None,
        }
    }

    fn context() -> RepeatContext {
        RepeatContext::new()
            .with_row(resolved("row_a", col("a"), "Row A"), 1, 3)
            .with_column(resolved("col_b", col("b"), "Column B"), 2, 4)
    }

    fn field_context() -> RepeatContext {
        RepeatContext::new()
            .with_row(resolved("a", col("a"), "A"), 1, 3)
            .with_column(resolved("b", col("b"), "B"), 2, 4)
    }

    #[test]
    fn repeat_variable_validates_ids_and_serializes_expr() {
        let variable = RepeatVariable::new("bill_length_mm", col("bill_length_mm"));
        variable.validate().expect("valid id");
        LogicalExprNode::from_default_expr(
            variable.expr.to_default_expr(&session_context()).unwrap(),
        )
        .expect("round trip expression");

        let invalid = RepeatVariable::new("bad.id", col("x"));
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn placeholder_constructors_serialize_and_are_recognized() {
        let expr = column().into_data_expr();
        LogicalExprNode::from_default_expr(expr.clone()).expect("placeholder serializes");
        let Expr::Placeholder(placeholder) = expr else {
            panic!("expected repeat placeholder");
        };
        assert_eq!(
            repeat_placeholder_kind_from_id(&placeholder.id),
            Some(RepeatPlaceholderKind::ColumnExpr)
        );
    }

    #[test]
    fn collect_repeat_placeholders_finds_nested_kinds() {
        let expr = row_index()
            .eq(column_index())
            .and(column_id().eq(lit("x")))
            .and(col(column_name()).is_not_null())
            .and(current_cell_predicate());
        let kinds = collect_repeat_placeholder_kinds(&expr).expect("collect placeholders");
        assert!(kinds.contains(&RepeatPlaceholderKind::RowIndex));
        assert!(kinds.contains(&RepeatPlaceholderKind::ColumnIndex));
        assert!(kinds.contains(&RepeatPlaceholderKind::ColumnId));
        assert!(kinds.contains(&RepeatPlaceholderKind::ColumnName));
        assert!(kinds.contains(&RepeatPlaceholderKind::CellId));
    }

    #[test]
    fn resolve_repeat_placeholders_replaces_expr_and_metadata() {
        let ctx = context();
        let expr =
            resolve_repeat_placeholders(row().into_data_expr() + column().into_data_expr(), &ctx)
                .expect("resolve repeat expression");
        assert_eq!(expr.to_string(), "a + b");

        let title = resolve_repeat_placeholders(column_title(), &ctx).expect("resolve title");
        assert_eq!(
            simplify_to_scalar_sync(title).expect("scalar"),
            ScalarValue::Utf8(Some("Column B".to_string()))
        );

        let cell = resolve_repeat_placeholders(cell_id(), &ctx).expect("resolve cell id");
        assert_eq!(
            simplify_to_scalar_sync(cell).expect("scalar"),
            ScalarValue::Utf8(Some("repeat_cell:row_a:col_b".to_string()))
        );
    }

    #[test]
    fn resolve_repeat_placeholders_rewrites_field_name_contexts() {
        let ctx = field_context();

        let column_expr =
            resolve_repeat_placeholders(col(column_name()), &ctx).expect("resolve column name");
        assert_eq!(column_expr.to_string(), "b");

        let literal = resolve_repeat_placeholders(lit(format!("field:{}", column_name())), &ctx)
            .expect("resolve string literal");
        assert_eq!(
            simplify_to_scalar_sync(literal).expect("scalar literal"),
            ScalarValue::Utf8(Some("field:b".to_string()))
        );

        let event_datum =
            resolve_repeat_placeholders(event::datum(column_name()), &ctx).expect("resolve datum");
        assert_eq!(event_datum.to_string(), "__event_datum_b");
    }

    #[test]
    fn resolve_repeat_channel_expr_rewrites_nested_source_columns() {
        let value = nested(["quarter".to_string(), column_name()]);
        let resolved =
            resolve_repeat_channel_expr(value, &field_context()).expect("resolve nested channel");
        assert_eq!(
            resolved.data_expr().to_string(),
            "named_struct(Utf8(\"quarter\"), quarter, Utf8(\"b\"), b)"
        );

        let config = resolved
            .channel_value()
            .get_nested_band_config()
            .expect("nested-band metadata");
        assert_eq!(config.source_columns, vec!["quarter", "b"]);
    }

    #[test]
    fn repeat_name_resolves_to_variable_id_not_variable_expr() {
        let ctx = RepeatContext::new().with_column(
            resolved("alias", col("source"), "Aliased Source"),
            0,
            1,
        );

        let expr =
            resolve_repeat_placeholders(column().into_data_expr(), &ctx).expect("resolve expr");
        assert_eq!(expr.to_string(), "source");

        let name_expr = resolve_repeat_placeholders(col(column_name()), &ctx)
            .expect("resolve name placeholder");
        assert_eq!(name_expr.to_string(), "alias");

        let nested_value = nested([column_name()]);
        let resolved_nested =
            resolve_repeat_channel_expr(nested_value, &ctx).expect("resolve nested channel");
        assert_eq!(
            resolved_nested.data_expr().to_string(),
            "named_struct(Utf8(\"alias\"), alias)"
        );
        let config = resolved_nested
            .channel_value()
            .get_nested_band_config()
            .expect("nested-band metadata");
        assert_eq!(config.source_columns, vec!["alias"]);
    }

    #[test]
    fn resolve_repeat_placeholders_errors_on_missing_role() {
        let err = resolve_repeat_placeholders(item().into_data_expr(), &context())
            .expect_err("missing item should error");
        assert!(err.to_string().contains("repeat::item()"), "{err}");

        let err = resolve_repeat_placeholders(cell_id(), &RepeatContext::new())
            .expect_err("missing cell");
        assert!(err.to_string().contains("repeat::cell_id()"), "{err}");
    }

    #[tokio::test]
    async fn current_cell_predicate_filters_store_rows() -> Result<(), AvengerChartError> {
        use std::sync::Arc;

        use datafusion::arrow::{
            array::{Int32Array, StringArray},
            datatypes::{DataType, Field, Schema},
            record_batch::RecordBatch,
        };
        use datafusion::prelude::SessionContext;

        let session = SessionContext::new();
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("cell_id", DataType::Utf8, false),
                Field::new("value", DataType::Int32, false),
            ])),
            vec![
                Arc::new(StringArray::from(vec![
                    "repeat_cell:row_a:col_b",
                    "repeat_cell:row_a:col_c",
                    "repeat_cell:row_d:col_b",
                ])),
                Arc::new(Int32Array::from(vec![1, 2, 3])),
            ],
        )?;
        let predicate = resolve_repeat_placeholders(current_cell_predicate(), &context())?;
        let filtered = session
            .read_batch(batch)?
            .filter(predicate)?
            .collect()
            .await?;
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].num_rows(), 1);
        let values = filtered[0]
            .column_by_name("value")
            .expect("value column")
            .as_any()
            .downcast_ref::<Int32Array>()
            .expect("int values");
        assert_eq!(values.value(0), 1);
        Ok(())
    }

    #[test]
    fn resolve_repeat_channel_expr_preserves_channel_metadata() {
        let value = column().with_scale_name("custom_x").band(0.5);
        let resolved = resolve_repeat_channel_expr(value, &context()).expect("resolve channel");
        assert_eq!(resolved.data_expr().to_string(), "b");
        assert_eq!(
            resolved.channel_value().get_scale_name("x"),
            Some("custom_x".to_string())
        );
    }

    #[test]
    fn resolve_repeat_channel_value_resolves_conditional_branches() {
        let value = ChannelValue::Conditional {
            conditions: vec![(
                LogicalExprNode::from_default_expr(row_index().eq(lit(1_i64)))
                    .expect("condition serializes"),
                ConditionalValue::Value {
                    expr: LogicalExprNode::from_default_expr(column().into_data_expr())
                        .expect("branch serializes"),
                },
            )],
            otherwise: ConditionalValue::Scaled {
                expr: LogicalExprNode::from_default_expr(row().into_data_expr())
                    .expect("otherwise serializes"),
            },
            scale_config: None,
            nested_band_config: None,
            legend_config: None,
            axis_config: None,
            domain_coordination: None,
            transform_scope: None,
        };
        let resolved = resolve_repeat_channel_value(value, &context()).expect("resolve channel");
        let expr = resolved
            .scale_input_expr(&session_context())
            .expect("scale input");
        assert!(expr.to_string().contains("CASE"), "{expr}");
        assert!(expr.to_string().contains("a"), "{expr}");
    }

    #[test]
    fn resolve_repeat_channel_value_resolves_scale_config_exprs() {
        let mut options = HashMap::new();
        options.insert(
            "tick_count".to_string(),
            LogicalExprNode::from_default_expr(row_index()).expect("tick count serializes"),
        );
        let value = ChannelValue::Scaled {
            expr: LogicalExprNode::from_default_expr(column().into_data_expr())
                .expect("expr serializes"),
            scale_name: None,
            position_boundary: None,
            scale_config: Some(Box::new(ScaleConfigSpec {
                scale_spec: Maybe::Unset,
                domain: Maybe::Set(ScaleDomain::new_interval(row_index(), column_index())),
                range: Maybe::Unset,
                ordering: Maybe::Unset,
                options,
            })),
            nested_band_config: None,
            legend_config: None,
            axis_config: None,
            domain_coordination: None,
            transform_scope: None,
        };
        let resolved = resolve_repeat_channel_value(value, &context()).expect("resolve channel");
        let scale_config = resolved.get_scale_config().expect("scale config");
        let exprs = scale_config.all_exprs(&session_context());
        let rendered = exprs.iter().map(ToString::to_string).collect::<Vec<_>>();
        assert_eq!(rendered, vec!["Int64(1)", "Int64(2)", "Int64(1)"]);
    }

    #[test]
    fn resolve_repeat_channel_value_resolves_legend_config_exprs() {
        let value = ChannelValue::Scaled {
            expr: LogicalExprNode::from_default_expr(column().into_data_expr())
                .expect("expr serializes"),
            scale_name: None,
            position_boundary: None,
            scale_config: None,
            nested_band_config: None,
            legend_config: Some(Box::new(Legend::new().title(column_title()))),
            axis_config: None,
            domain_coordination: None,
            transform_scope: None,
        };
        let resolved = resolve_repeat_channel_value(value, &context()).expect("resolve channel");
        let legend = resolved.get_legend_config().expect("legend config");
        let Some(Some(title)) = legend.title.as_option() else {
            panic!("expected legend title");
        };
        let title = title
            .to_default_expr(&session_context())
            .expect("title deserializes");
        assert_eq!(
            simplify_to_scalar_sync(title).expect("scalar title"),
            ScalarValue::Utf8(Some("Column B".to_string()))
        );
    }

    #[test]
    fn evaluate_repeat_predicate_handles_diagonal_logic() {
        let ctx = RepeatContext::new()
            .with_row(resolved("a", col("a"), "a"), 1, 3)
            .with_column(resolved("b", col("b"), "b"), 1, 3);
        assert!(
            evaluate_repeat_predicate(row_index().eq(column_index()), &ctx)
                .expect("diagonal predicate")
        );

        let ctx = RepeatContext::new()
            .with_row(resolved("a", col("a"), "a"), 0, 3)
            .with_column(resolved("b", col("b"), "b"), 1, 3);
        assert!(
            !evaluate_repeat_predicate(row_index().eq(column_index()), &ctx)
                .expect("off-diagonal predicate")
        );
    }

    #[test]
    fn evaluate_repeat_predicate_rejects_data_dependent_exprs() {
        let err = evaluate_repeat_predicate(col("datum_value").eq(lit(1_i64)), &context())
            .expect_err("data-dependent predicate should not evaluate");
        assert!(
            err.to_string().contains("column references")
                || err.to_string().contains("cannot be simplified"),
            "{err}"
        );
    }
}
