//! Channel metadata for nested categorical position scales.
//!
//! A nested band position is still an ordinary Cartesian position channel at
//! render time: the scale evaluates a structured categorical value to numeric
//! pixels. This module stores the authoring metadata needed to configure that
//! scale per nesting level.

use std::{collections::BTreeMap, marker::PhantomData};

use datafusion::{logical_expr::Expr, prelude::SessionContext};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};

use crate::{
    Axis, CoordinationScope, DefaultLogicalExprNodeExt, DomainCoordination,
    DomainCoordinationGroup, IntoExpr, Maybe, RadiusExpression, ScaleDefaultDomain, ScaleDomain,
    ScaleOrderingSpec, validate_domain_group_id,
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
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PositionBoundary {
    /// Boundary inside the leaf band.
    Band { band: f64 },
    /// Boundary inside a specific nesting level's band.
    LevelBand { level: usize, band: f64 },
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

    /// Return the band fraction.
    pub fn band_fraction(self) -> f64 {
        match self {
            PositionBoundary::Band { band } | PositionBoundary::LevelBand { band, .. } => band,
        }
    }
}

/// Nested-band configuration carried by a position channel.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct NestedBandSpec {
    /// Sparse configuration keyed by struct field / nesting level index.
    #[serde(default)]
    pub levels: BTreeMap<usize, NestedBandLevelSpec>,
}

impl NestedBandSpec {
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
}

impl std::fmt::Debug for NestedBandSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NestedBandSpec")
            .field("levels", &self.levels)
            .finish()
    }
}

/// Configuration for one level of a nested band position channel.
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
    }

    #[test]
    fn nested_level_config_collects_expressions() {
        let ctx = SessionContext::new();
        let spec = NestedBandLevelConfig::<()>::new(NestedBandLevelSpec::default())
            .domain_values(vec![lit("a"), lit("b")])
            .order_by(col("sort_key"))
            .into_spec();
        let rendered = spec
            .all_exprs(&ctx)
            .into_iter()
            .map(|expr| expr.to_string())
            .collect::<Vec<_>>();
        assert_eq!(rendered, vec!["Utf8(\"a\")", "Utf8(\"b\")", "sort_key"]);
    }
}
