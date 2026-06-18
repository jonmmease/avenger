//! Parallel coordinates for `avenger-chart`.

mod axis;
mod axis_overlay;
mod coord;
mod frame;
mod guide;
mod line;

pub use axis::ParallelAxis;
pub use axis_overlay::{CompiledParallelAxisOverlay, ParallelAxisOverlay};
pub use coord::{
    PARALLEL_DIMENSION_CHANNEL_PREFIX, PARALLEL_LOCAL_X_CHANNEL, PARALLEL_LOCAL_Y_CHANNEL,
    Parallel, ParallelDimensionConfig, ParallelDimensionSpec, ParallelTransform,
    dimension_id_from_generated_channel, generated_dimension_channel,
};
pub use frame::{
    ParallelAxisSlot, ParallelFrameGeometry, propose_axis_order, resolve_parallel_frame,
};
pub use guide::{CompiledParallelGuide, ParallelAxisGuideDatum, ParallelGuide};
pub use line::{CompiledParallelLine, ParallelLine};
