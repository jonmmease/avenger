use std::collections::HashMap;

use datafusion::{logical_expr::Expr, prelude::SessionContext};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::{
    DefaultLogicalExprNodeExt, Maybe, RadiusExpression, ScaleDefaultDomain, ScaleDomain,
    ScaleRange, ScaleSpec, SerializableExpr,
};

/// Optional categorical scale-domain ordering configuration.
#[serde_as]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScaleOrderingSpec {
    #[serde_as(as = "Option<FromInto<SerializableExpr>>")]
    pub order_expr: Option<LogicalExprNode>,
    pub order_descending: Option<bool>,
}

impl ScaleOrderingSpec {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn has_order_expr(&self) -> bool {
        self.order_expr.is_some()
    }

    pub fn order_descending(&self) -> bool {
        self.order_descending.unwrap_or(false)
    }

    pub fn merge(&mut self, other: ScaleOrderingSpec) {
        if other.order_expr.is_some() {
            self.order_expr = other.order_expr;
        }
        if other.order_descending.is_some() {
            self.order_descending = other.order_descending;
        }
    }

    pub fn all_exprs(&self, ctx: &SessionContext) -> Vec<Expr> {
        self.order_expr
            .as_ref()
            .and_then(|node| node.to_default_expr(ctx).ok())
            .into_iter()
            .collect()
    }
}

/// Owned scale configuration data shared by chart-facing scale wrappers and channels.
#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaleConfigSpec {
    pub scale_spec: Maybe<Box<dyn ScaleSpec>>,
    pub domain: Maybe<ScaleDomain>,
    pub range: Maybe<ScaleRange>,
    pub ordering: Maybe<ScaleOrderingSpec>,
    #[serde_as(as = "HashMap<_, FromInto<SerializableExpr>>")]
    pub options: HashMap<String, LogicalExprNode>,
}

impl ScaleConfigSpec {
    pub fn empty() -> Self {
        Self {
            scale_spec: Maybe::Unset,
            domain: Maybe::Unset,
            range: Maybe::Unset,
            ordering: Maybe::Unset,
            options: HashMap::new(),
        }
    }

    pub fn new(
        scale_spec: Maybe<Box<dyn ScaleSpec>>,
        domain: Maybe<ScaleDomain>,
        range: Maybe<ScaleRange>,
        ordering: Maybe<ScaleOrderingSpec>,
        options: HashMap<String, LogicalExprNode>,
    ) -> Self {
        Self {
            scale_spec,
            domain,
            range,
            ordering,
            options,
        }
    }

    pub fn all_exprs(&self, ctx: &SessionContext) -> Vec<Expr> {
        let mut exprs = Vec::new();

        if let Some(domain) = self.domain.as_option() {
            exprs.extend(scale_domain_exprs(domain, ctx));
        }
        if let Some(range) = self.range.as_option() {
            exprs.extend(scale_range_exprs(range, ctx));
        }
        if let Some(ordering) = self.ordering.as_option() {
            exprs.extend(ordering.all_exprs(ctx));
        }
        for node in self.options.values() {
            if let Ok(expr) = node.to_default_expr(ctx) {
                exprs.push(expr);
            }
        }

        exprs
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

fn scale_range_exprs(range: &ScaleRange, ctx: &SessionContext) -> Vec<Expr> {
    match range {
        ScaleRange::Numeric(start, end) => [start, end.as_ref()]
            .into_iter()
            .filter_map(|node| node.to_default_expr(ctx).ok())
            .collect(),
        ScaleRange::Discrete(_) | ScaleRange::Color(_) | ScaleRange::Pattern(_) => Vec::new(),
    }
}
