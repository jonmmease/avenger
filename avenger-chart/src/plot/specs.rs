//! Specification types for scales and axes

use serde::{Deserialize, Serialize};

use crate::{
    axis::Axis,
    scales::{Auto, Scale},
};

/// How a scale is defined for a channel
#[derive(Clone, Serialize, Deserialize)]
pub enum ScaleSpec {
    /// Scale defined locally on this plot with configuration
    Local(Scale<Auto>),
}

/// How an axis is customized for a channel
#[derive(Serialize, Deserialize)]
pub enum AxisSpec {
    /// Axis customized locally with configuration
    Local(Box<dyn Axis>),
}

impl Clone for AxisSpec {
    fn clone(&self) -> Self {
        match self {
            AxisSpec::Local(axis) => AxisSpec::Local(axis.box_clone()),
        }
    }
}
