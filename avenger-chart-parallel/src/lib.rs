//! Parallel coordinates for `avenger-chart`.

mod axis;
mod coord;
mod frame;
mod guide;

pub use axis::ParallelAxis;
pub use coord::{
    PARALLEL_DIMENSION_CHANNEL_PREFIX, Parallel, ParallelDimensionConfig, ParallelDimensionSpec,
    ParallelTransform, dimension_id_from_generated_channel, generated_dimension_channel,
};
pub use frame::{
    ParallelAxisSlot, ParallelFrameGeometry, propose_axis_order, resolve_parallel_frame,
};
pub use guide::ParallelGuide;
