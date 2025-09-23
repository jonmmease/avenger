//! Specification types for scales and axes

use crate::scales::{Auto, Scale};

/// How a scale is defined for a channel
#[derive(Clone)]
pub enum ScaleSpec {
    /// Scale defined locally on this plot with configuration
    Local(Scale<Auto>),
}

/// How an axis is customized for a channel
#[derive(Clone)]
pub enum AxisSpec<A: Clone> {
    /// Axis customized locally with configuration
    Local(A),
}
