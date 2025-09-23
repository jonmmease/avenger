//! Specification types for scales and axes

use crate::scales::{Auto, Scale};
use std::sync::Arc;

/// How a scale is defined for a channel
#[derive(Clone)]
pub enum ScaleSpec {
    /// Scale defined locally on this plot with configuration
    Local(Scale<Auto>),
}

/// How an axis is customized for a channel
#[derive(Clone)]
pub enum AxisSpec<A> {
    /// Axis customized locally with a configuration function
    Local(Arc<dyn Fn(A) -> A + Send + Sync>),
}
