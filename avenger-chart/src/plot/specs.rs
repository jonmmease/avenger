//! Specification types for scales and axes

use crate::marks::RadiusExpression;
use crate::scales::Scale;
use datafusion::dataframe::DataFrame;
use std::sync::Arc;

/// Type alias for scale domain expressions with optional radius information
pub(crate) type ScaleDomainWithRadius = Vec<(
    Arc<DataFrame>,
    datafusion::logical_expr::Expr,
    Option<RadiusExpression>,
)>;

/// How a scale is defined for a channel
#[derive(Clone)]
pub enum ScaleSpec {
    /// Scale defined locally on this plot with a configuration function
    Local(Arc<dyn Fn(Scale) -> Scale + Send + Sync>),
}

/// How an axis is customized for a channel
#[derive(Clone)]
pub enum AxisSpec<A> {
    /// Axis customized locally with a configuration function
    Local(Arc<dyn Fn(A) -> A + Send + Sync>),
}
