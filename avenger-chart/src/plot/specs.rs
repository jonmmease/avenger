//! Specification types for scales and axes

use serde::{Deserialize, Serialize};

use crate::chart_core::Axis;

pub use avenger_chart_scales::PlotScaleSpec as ScaleSpec;

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
