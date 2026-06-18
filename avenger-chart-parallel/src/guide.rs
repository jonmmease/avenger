use std::{any::Any, collections::HashMap, sync::Arc};

use avenger_chart_core::{
    AvengerChartError, CompiledGuide, CompiledMarkCore, CoordMeasurement, CoordinateGuide,
    GuideSharingContext, LayoutBounds, OverflowSpaceRequirement, Theme,
};
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{group::Clip, mark::SceneMark};
use datafusion::{common::ScalarValue, dataframe::DataFrame, prelude::SessionContext};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::ParallelAxis;

/// Parallel-coordinate guide configuration.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ParallelGuide {
    pub axes: HashMap<String, ParallelAxis>,
}

impl CoordinateGuide for ParallelGuide {
    type Axis = ParallelAxis;

    fn set_axes(&mut self, axes: HashMap<String, Self::Axis>) {
        self.axes = axes;
    }

    fn set_compiled_marks<M>(
        &mut self,
        _compiled_marks: &[Arc<M>],
        _session_context: &SessionContext,
    ) where
        M: CompiledMarkCore + ?Sized,
    {
    }

    fn update(&mut self, other: Self) {
        for (channel, axis) in other.axes {
            match self.axes.get_mut(&channel) {
                Some(existing) => *existing = existing.clone().update(axis),
                None => {
                    self.axes.insert(channel, axis);
                }
            }
        }
    }

    fn build(self) -> Box<dyn CompiledGuide> {
        Box::new(CompiledParallelGuide { axes: self.axes })
    }
}

/// Compiled no-op guide shell for parallel coordinates.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CompiledParallelGuide {
    pub axes: HashMap<String, ParallelAxis>,
}

#[async_trait::async_trait]
#[typetag::serde]
impl CompiledGuide for CompiledParallelGuide {
    async fn measure_overflow(
        &self,
        _scales: &HashMap<String, ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _theme: &Theme,
        _params: &IndexMap<String, ScalarValue>,
        _data_override: Option<&DataFrame>,
        _ctx: &SessionContext,
        _sharing_context: GuideSharingContext<'_>,
        _coord_measurement: Option<&dyn CoordMeasurement>,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        Ok(OverflowSpaceRequirement::default())
    }

    async fn evaluate(
        &self,
        _scales: &HashMap<String, ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _plot_bounds: &LayoutBounds,
        _guide_overflow: &OverflowSpaceRequirement,
        _theme: &Theme,
        _params: &IndexMap<String, ScalarValue>,
        _ctx: &SessionContext,
        _data_override: Option<&DataFrame>,
        _sharing_context: GuideSharingContext<'_>,
        _coord_measurement: &dyn CoordMeasurement,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Ok(Vec::new())
    }

    fn get_clip(
        &self,
        _plot_width: f32,
        _plot_height: f32,
        _scales: &HashMap<String, ConfiguredScale>,
    ) -> Clip {
        Clip::None
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
