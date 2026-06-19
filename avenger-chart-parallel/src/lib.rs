//! Parallel coordinates for `avenger-chart`.
//!
//! Parallel marks bind data to dimension ids. The coordinate system owns the
//! frame, guide axes, ordering, drag display state, and axis overlays. Ordinary
//! mark style channels such as fill, stroke, opacity, and shape continue to use
//! the normal chart scale and legend pipeline.

mod axis;
mod axis_overlay;
mod coord;
mod frame;
mod guide;
mod line;
mod symbol;

pub use axis::ParallelAxis;
pub use axis_overlay::{CompiledParallelAxisOverlay, ParallelAxisOverlay};
pub use coord::{
    PARALLEL_DIMENSION_CHANNEL_PREFIX, PARALLEL_LOCAL_X_CHANNEL, PARALLEL_LOCAL_Y_CHANNEL,
    Parallel, ParallelDimensionBinding, ParallelDimensionConfig, ParallelDimensionSpec,
    ParallelFrameDimensionConfig, ParallelTransform, dimension_id_from_generated_channel,
    generated_dimension_channel,
};
pub use frame::{
    ParallelAxisSlot, ParallelDisplayState, ParallelFrameGeometry, ParallelOrderState,
    propose_axis_order, resolve_parallel_frame,
};
pub use guide::{CompiledParallelGuide, ParallelAxisGuideDatum, ParallelGuide};
pub use line::{CompiledParallelLine, ParallelLine};
pub use symbol::{CompiledParallelSymbol, ParallelSymbol};
