//! Channel metadata for nested categorical position scales.
//!
//! A nested band position is still an ordinary Cartesian position channel at
//! render time: the scale evaluates a structured categorical value to numeric
//! pixels. This module stores the authoring metadata needed to configure that
//! scale per nesting level.

use std::{
    collections::{BTreeMap, BTreeSet},
    marker::PhantomData,
};

use datafusion::{
    logical_expr::Expr,
    prelude::{SessionContext, col, lit, named_struct},
};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::{
    AvengerChartError, Axis, ChannelExpr, ChannelValue, CoordinationScope,
    DefaultLogicalExprNodeExt, DomainCoordination, DomainCoordinationGroup, IntoExpr, Maybe,
    RadiusExpression, ScaleDefaultDomain, ScaleDomain, ScaleOrderingSpec, SerializableExpr,
    validate_domain_group_id,
};

/// How child domains are shared inside a parent nested-band level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NestScope {
    /// Child domains are local to each immediate parent tuple.
    Free,
    /// Every parent tuple reserves the same complete child-domain slots.
    Shared,
}

/// A boundary request within a position scale's band hierarchy.
#[serde_as]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PositionBoundary {
    /// Boundary inside the leaf band.
    Band { band: f64 },
    /// Boundary inside a specific nesting level's band.
    LevelBand { level: usize, band: f64 },
    /// Row-wise boundary inside the leaf band.
    BandExpr {
        #[serde_as(as = "FromInto<SerializableExpr>")]
        band: LogicalExprNode,
    },
    /// Row-wise boundary inside a specific nesting level's band.
    LevelBandExpr {
        level: usize,
        #[serde_as(as = "FromInto<SerializableExpr>")]
        band: LogicalExprNode,
    },
}

impl PositionBoundary {
    /// Construct a leaf-band boundary.
    pub fn band(band: f64) -> Self {
        Self::Band { band }
    }

    /// Construct a boundary inside an explicit nesting level.
    pub fn level_band(level: usize, band: f64) -> Self {
        Self::LevelBand { level, band }
    }

    /// Construct a row-wise leaf-band boundary.
    pub fn band_expr(band: impl IntoExpr) -> Self {
        Self::BandExpr {
            band: LogicalExprNode::from_default_expr(band.into_expr())
                .expect("Failed to serialize band expression"),
        }
    }

    /// Construct a row-wise boundary inside an explicit nesting level.
    pub fn level_band_expr(level: usize, band: impl IntoExpr) -> Self {
        Self::LevelBandExpr {
            level,
            band: LogicalExprNode::from_default_expr(band.into_expr())
                .expect("Failed to serialize level-band expression"),
        }
    }

    /// Return the constant band fraction, when this boundary is not row-wise.
    pub fn band_fraction(&self) -> Option<f64> {
        match self {
            PositionBoundary::Band { band } | PositionBoundary::LevelBand { band, .. } => {
                Some(*band)
            }
            PositionBoundary::BandExpr { .. } | PositionBoundary::LevelBandExpr { .. } => None,
        }
    }

    /// Expressions referenced by this boundary.
    pub fn all_exprs(&self, ctx: &SessionContext) -> Vec<Expr> {
        match self {
            PositionBoundary::Band { .. } | PositionBoundary::LevelBand { .. } => Vec::new(),
            PositionBoundary::BandExpr { band } | PositionBoundary::LevelBandExpr { band, .. } => {
                band.to_expr(ctx).ok().into_iter().collect()
            }
        }
    }
}

/// Nested-band configuration carried by a position channel.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct NestedBandSpec {
    /// Source dataframe columns that define the nested categorical levels.
    #[serde(default)]
    pub source_columns: Vec<String>,
    /// Sparse configuration keyed by struct field / nesting level index.
    #[serde(default)]
    pub levels: BTreeMap<usize, NestedBandLevelSpec>,
}

impl NestedBandSpec {
    /// Create nested-band metadata from explicit source dataframe columns.
    pub fn from_source_columns(source_columns: Vec<String>) -> Self {
        Self {
            source_columns,
            levels: BTreeMap::new(),
        }
    }

    /// Get a level configuration if it exists.
    pub fn level(&self, level: usize) -> Option<&NestedBandLevelSpec> {
        self.levels.get(&level)
    }

    /// Mutably get a level configuration, creating it if necessary.
    pub fn level_mut(&mut self, level: usize) -> &mut NestedBandLevelSpec {
        self.levels.entry(level).or_default()
    }

    /// Return all expressions referenced by nested-band level metadata.
    pub fn all_exprs(&self, ctx: &SessionContext) -> Vec<Expr> {
        self.levels
            .values()
            .flat_map(|level| level.all_exprs(ctx))
            .collect()
    }

    /// Validate the source column contract for public nested-band channels.
    pub fn validate_source_columns(&self) -> Result<(), AvengerChartError> {
        if self.source_columns.is_empty() {
            return Err(AvengerChartError::InvalidArgument(
                "Nested-band position channels must be created with nested([...]); raw struct-valued position columns are not supported"
                    .to_string(),
            ));
        }

        let mut seen = BTreeSet::new();
        for column in &self.source_columns {
            if column.is_empty() {
                return Err(AvengerChartError::InvalidArgument(
                    "nested([...]) source column names must be non-empty".to_string(),
                ));
            }
            if !seen.insert(column) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "nested([...]) source column '{column}' is duplicated"
                )));
            }
        }

        Ok(())
    }
}

impl std::fmt::Debug for NestedBandSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NestedBandSpec")
            .field("source_columns", &self.source_columns)
            .field("levels", &self.levels)
            .finish()
    }
}

/// Build a nested categorical position channel from source dataframe columns.
///
/// This is the public nested-band entry point. The returned channel lowers to a
/// struct expression for scale evaluation, but keeps the original source column
/// names in metadata so selection predicates and event datum requests can stay
/// field-based.
pub fn nested<I, S>(columns: I) -> ChannelExpr
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let source_columns = columns.into_iter().map(Into::into).collect::<Vec<_>>();
    let args = source_columns
        .iter()
        .flat_map(|column| [lit(column.clone()), col(column.clone())])
        .collect::<Vec<_>>();
    let expr = named_struct(args);
    let channel_value = ChannelValue::from(expr.clone())
        .with_nested_band_config(NestedBandSpec::from_source_columns(source_columns));
    ChannelExpr::new(expr, channel_value)
}

/// Configuration for one level of a nested band position channel.
#[serde_as]
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct NestedBandLevelSpec {
    #[serde(default)]
    pub nest_scope: Option<NestScope>,
    #[serde(default)]
    pub domain_coordination: Option<DomainCoordination>,
    #[serde(default)]
    pub domain: Maybe<ScaleDomain>,
    #[serde(default)]
    pub ordering: Maybe<ScaleOrderingSpec>,
    #[serde(default)]
    #[serde_as(as = "Option<FromInto<SerializableExpr>>")]
    pub label_expr: Option<LogicalExprNode>,
    #[serde(default)]
    pub padding_inner: Option<f64>,
    #[serde(default)]
    pub padding_outer: Option<f64>,
    #[serde(default)]
    pub padding_inner_px: Option<f64>,
    #[serde(default)]
    pub padding_outer_px: Option<f64>,
    #[serde(default)]
    pub axis_config: Option<Box<dyn Axis>>,
}

impl NestedBandLevelSpec {
    /// Return all expressions referenced by this level's metadata.
    pub fn all_exprs(&self, ctx: &SessionContext) -> Vec<Expr> {
        let mut exprs = Vec::new();
        if let Some(domain) = self.domain.as_option() {
            exprs.extend(scale_domain_exprs(domain, ctx));
        }
        if let Some(ordering) = self.ordering.as_option() {
            exprs.extend(ordering.all_exprs(ctx));
        }
        if let Some(label_expr) = &self.label_expr
            && let Ok(expr) = label_expr.to_default_expr(ctx)
        {
            exprs.push(expr);
        }
        if let Some(axis) = &self.axis_config {
            exprs.extend(axis.all_exprs(ctx));
        }
        exprs
    }
}

impl std::fmt::Debug for NestedBandLevelSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NestedBandLevelSpec")
            .field("nest_scope", &self.nest_scope)
            .field("domain_coordination", &self.domain_coordination)
            .field("domain", &self.domain)
            .field("ordering", &self.ordering)
            .field("has_label_expr", &self.label_expr.is_some())
            .field("padding_inner", &self.padding_inner)
            .field("padding_outer", &self.padding_outer)
            .field("padding_inner_px", &self.padding_inner_px)
            .field("padding_outer_px", &self.padding_outer_px)
            .field("has_axis_config", &self.axis_config.is_some())
            .finish()
    }
}

/// Fluent builder for one nested-band level.
pub struct NestedBandLevelConfig<A: Axis + Clone + Default + Send + Sync + 'static> {
    spec: NestedBandLevelSpec,
    _axis: PhantomData<A>,
}

impl<A: Axis + Clone + Default + Send + Sync + 'static> NestedBandLevelConfig<A> {
    pub fn new(spec: NestedBandLevelSpec) -> Self {
        Self {
            spec,
            _axis: PhantomData,
        }
    }

    pub fn into_spec(self) -> NestedBandLevelSpec {
        self.spec
    }

    pub fn nest_scope(mut self, scope: NestScope) -> Self {
        self.spec.nest_scope = Some(scope);
        self
    }

    pub fn domain_scope(mut self, scope: CoordinationScope) -> Self {
        self.spec.domain_coordination = Some(
            self.spec
                .domain_coordination
                .unwrap_or_default()
                .with_scope(scope),
        );
        self
    }

    pub fn domain_group(mut self, group: impl Into<String>) -> Self {
        let group = group.into();
        validate_domain_group_id(&group).expect("invalid domain coordination group id");
        self.spec.domain_coordination = Some(
            self.spec
                .domain_coordination
                .unwrap_or_default()
                .with_group(DomainCoordinationGroup::Named(group)),
        );
        self
    }

    pub fn domain_coordination(mut self, coordination: DomainCoordination) -> Self {
        self.spec.domain_coordination = Some(coordination);
        self
    }

    pub fn domain<D: Into<ScaleDomain>>(mut self, domain: D) -> Self {
        self.spec.domain = Maybe::Set(domain.into());
        self
    }

    pub fn domain_values(mut self, values: Vec<impl Into<Expr>>) -> Self {
        self.spec.domain = Maybe::Set(ScaleDomain::new_discrete(
            values.into_iter().map(|value| value.into()).collect(),
        ));
        self
    }

    pub fn order_by(mut self, expr: impl IntoExpr) -> Self {
        let mut ordering = self.spec.ordering.unwrap_or_else(ScaleOrderingSpec::empty);
        ordering.order_expr = Some(
            LogicalExprNode::from_default_expr(expr.into_expr())
                .expect("Failed to serialize nested-band order expression"),
        );
        self.spec.ordering = Maybe::Set(ordering);
        self
    }

    pub fn order_asc(mut self) -> Self {
        let mut ordering = self.spec.ordering.unwrap_or_else(ScaleOrderingSpec::empty);
        ordering.order_descending = Some(false);
        self.spec.ordering = Maybe::Set(ordering);
        self
    }

    pub fn order_desc(mut self) -> Self {
        let mut ordering = self.spec.ordering.unwrap_or_else(ScaleOrderingSpec::empty);
        ordering.order_descending = Some(true);
        self.spec.ordering = Maybe::Set(ordering);
        self
    }

    pub fn label_with(mut self, expr: impl IntoExpr) -> Self {
        self.spec.label_expr = Some(
            LogicalExprNode::from_default_expr(expr.into_expr())
                .expect("Failed to serialize nested-band label expression"),
        );
        self
    }

    pub fn padding_inner(mut self, padding: f64) -> Self {
        self.spec.padding_inner = Some(padding);
        self
    }

    pub fn padding_outer(mut self, padding: f64) -> Self {
        self.spec.padding_outer = Some(padding);
        self
    }

    pub fn padding_inner_px(mut self, padding: f64) -> Self {
        self.spec.padding_inner_px = Some(padding);
        self
    }

    pub fn padding_outer_px(mut self, padding: f64) -> Self {
        self.spec.padding_outer_px = Some(padding);
        self
    }

    pub fn axis<F>(mut self, f: F) -> Self
    where
        F: FnOnce(A) -> A,
    {
        let axis = self
            .spec
            .axis_config
            .take()
            .and_then(|axis| axis.as_any().downcast_ref::<A>().cloned())
            .unwrap_or_default();
        self.spec.axis_config = Some(Box::new(f(axis)));
        self
    }
}

fn scale_domain_exprs(domain: &ScaleDomain, ctx: &SessionContext) -> Vec<Expr> {
    let mut exprs = Vec::new();
    match &domain.default_domain {
        ScaleDefaultDomain::Interval(start, end) => {
            if let Ok(expr) = start.to_default_expr(ctx) {
                exprs.push(expr);
            }
            if let Ok(expr) = end.to_default_expr(ctx) {
                exprs.push(expr);
            }
        }
        ScaleDefaultDomain::Discrete(values) => {
            for node in values {
                if let Ok(expr) = node.to_default_expr(ctx) {
                    exprs.push(expr);
                }
            }
        }
        ScaleDefaultDomain::DomainExprs(domain_exprs) => {
            for domain_expr in domain_exprs {
                if let Ok(expr) = domain_expr.expr.to_default_expr(ctx) {
                    exprs.push(expr);
                }
                if let Some(radius) = &domain_expr.radius {
                    match radius {
                        RadiusExpression::Symmetric(node) => {
                            if let Ok(expr) = node.to_default_expr(ctx) {
                                exprs.push(expr);
                            }
                        }
                        RadiusExpression::Asymmetric { lower, upper } => {
                            if let Ok(expr) = lower.to_default_expr(ctx) {
                                exprs.push(expr);
                            }
                            if let Ok(expr) = upper.to_default_expr(ctx) {
                                exprs.push(expr);
                            }
                        }
                    }
                }
            }
        }
        ScaleDefaultDomain::NoDefault => {}
    }
    if let Some(raw_domain) = &domain.raw_domain
        && let Ok(expr) = raw_domain.to_default_expr(ctx)
    {
        exprs.push(expr);
    }
    exprs
}

#[cfg(test)]
mod tests {
    use datafusion::prelude::{col, lit};

    use super::*;

    #[test]
    fn nest_scope_round_trips() {
        let json = serde_json::to_string(&NestScope::Shared).expect("serialize");
        assert_eq!(json, "\"shared\"");
        let restored: NestScope = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored, NestScope::Shared);
    }

    #[test]
    fn position_boundary_round_trips() {
        let boundary = PositionBoundary::band(0.5);
        let json = serde_json::to_string(&boundary).expect("serialize");
        let restored: PositionBoundary = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored, boundary);

        let boundary = PositionBoundary::level_band(1, 0.25);
        let json = serde_json::to_string(&boundary).expect("serialize");
        let restored: PositionBoundary = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored, boundary);

        let boundary = PositionBoundary::band_expr(col("half_width"));
        let json = serde_json::to_string(&boundary).expect("serialize");
        let restored: PositionBoundary = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored, boundary);

        let boundary = PositionBoundary::level_band_expr(1, col("level_width"));
        let json = serde_json::to_string(&boundary).expect("serialize");
        let restored: PositionBoundary = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored, boundary);
    }

    #[test]
    fn position_boundary_exprs_are_collected() {
        let ctx = SessionContext::new();
        let boundary = PositionBoundary::level_band_expr(1, col("dynamic_band"));
        let rendered = boundary
            .all_exprs(&ctx)
            .into_iter()
            .map(|expr| expr.to_string())
            .collect::<Vec<_>>();
        assert_eq!(rendered, vec!["dynamic_band"]);
    }

    #[test]
    fn nested_band_label_with_participates_in_all_exprs() {
        let ctx = SessionContext::new();
        let spec = NestedBandLevelConfig::<()>::new(NestedBandLevelSpec::default())
            .domain_values(vec![lit("a"), lit("b")])
            .order_by(col("sort_key"))
            .label_with(col("display_label"))
            .into_spec();
        let rendered = spec
            .all_exprs(&ctx)
            .into_iter()
            .map(|expr| expr.to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            rendered,
            vec!["Utf8(\"a\")", "Utf8(\"b\")", "sort_key", "display_label"]
        );
    }

    #[test]
    fn nested_band_level_label_with_is_stored() {
        let ctx = SessionContext::new();
        let spec = NestedBandLevelConfig::<()>::new(NestedBandLevelSpec::default())
            .label_with(col("month_label"))
            .into_spec();

        let label_expr = spec.label_expr.as_ref().expect("label expression");
        assert_eq!(
            label_expr
                .to_default_expr(&ctx)
                .expect("deserialize label")
                .to_string(),
            "month_label"
        );
    }

    #[test]
    fn nested_band_label_with_survives_json_and_bincode() {
        let ctx = SessionContext::new();
        let spec = NestedBandSpec {
            source_columns: vec!["month".to_string()],
            levels: BTreeMap::from([(
                0,
                NestedBandLevelConfig::<()>::new(NestedBandLevelSpec::default())
                    .label_with(col("month_label"))
                    .into_spec(),
            )]),
        };

        let json = serde_json::to_string(&spec).expect("json serialize");
        assert!(json.contains("label_expr"));
        let restored_json: NestedBandSpec = serde_json::from_str(&json).expect("json deserialize");
        assert_eq!(
            restored_json
                .level(0)
                .unwrap()
                .label_expr
                .as_ref()
                .unwrap()
                .to_default_expr(&ctx)
                .expect("json label expr")
                .to_string(),
            "month_label"
        );

        let serialized = bincode::serialize(&spec).expect("bincode serialize");
        let restored_bincode: NestedBandSpec =
            bincode::deserialize(&serialized).expect("bincode deserialize");
        assert_eq!(
            restored_bincode
                .level(0)
                .unwrap()
                .label_expr
                .as_ref()
                .unwrap()
                .to_default_expr(&ctx)
                .expect("bincode label expr")
                .to_string(),
            "month_label"
        );
    }

    #[test]
    fn nested_builds_struct_channel_with_source_columns() {
        let ctx = SessionContext::new();
        let value = nested(["quarter", "team"]);
        assert!(value.data_expr().to_string().starts_with("named_struct("));

        let config = value
            .channel_value()
            .get_nested_band_config()
            .expect("nested-band metadata");
        assert_eq!(config.source_columns, vec!["quarter", "team"]);
        let rendered = value
            .channel_value()
            .all_exprs(&ctx)
            .into_iter()
            .map(|expr| expr.to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            rendered,
            vec!["named_struct(Utf8(\"quarter\"), quarter, Utf8(\"team\"), team)"]
        );
    }

    #[test]
    fn nested_source_columns_validate_non_empty_and_unique() {
        let empty = NestedBandSpec::from_source_columns(Vec::new());
        assert!(matches!(
            empty.validate_source_columns(),
            Err(AvengerChartError::InvalidArgument(_))
        ));

        let duplicate =
            NestedBandSpec::from_source_columns(vec!["quarter".to_string(), "quarter".to_string()]);
        assert!(matches!(
            duplicate.validate_source_columns(),
            Err(AvengerChartError::InvalidArgument(_))
        ));
    }

    #[test]
    fn nested_band_spec_bincode_round_trips_without_level_axis_config() {
        let spec = NestedBandSpec {
            source_columns: vec!["group".to_string(), "series".to_string()],
            levels: BTreeMap::from([(
                1,
                NestedBandLevelSpec {
                    nest_scope: Some(NestScope::Shared),
                    padding_inner: Some(0.05),
                    ..Default::default()
                },
            )]),
        };

        let serialized = bincode::serialize(&spec).expect("serialize");
        let restored: NestedBandSpec = bincode::deserialize(&serialized).expect("deserialize");

        assert_eq!(
            restored.level(1).unwrap().nest_scope,
            Some(NestScope::Shared)
        );
        assert_eq!(restored.source_columns, vec!["group", "series"]);
        assert_eq!(restored.level(1).unwrap().padding_inner, Some(0.05));
    }

    #[test]
    fn nested_band_spec_bincode_round_trips_with_level_axis_config() {
        let spec = NestedBandSpec {
            source_columns: vec!["group".to_string(), "series".to_string()],
            levels: BTreeMap::from([(
                1,
                NestedBandLevelConfig::<()>::new(NestedBandLevelSpec::default())
                    .axis(|axis| axis)
                    .into_spec(),
            )]),
        };

        let serialized = bincode::serialize(&spec).expect("serialize");
        let restored: NestedBandSpec = bincode::deserialize(&serialized).expect("deserialize");

        assert!(
            restored
                .level(1)
                .unwrap()
                .axis_config
                .as_ref()
                .is_some_and(|axis| axis.as_any().is::<()>())
        );
    }
}
