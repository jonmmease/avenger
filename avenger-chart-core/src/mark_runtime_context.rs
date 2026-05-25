use datafusion::common::ScalarValue;

use crate::{CoordMeasurement, MarkRenderContext};

/// Core runtime view passed to compiled mark renderers.
///
/// Ordinary marks should use `core_view()` plus the coordinate transform they
/// receive at render time. Top-level layout-owned marks are dispatched by the
/// facade before the generic mark render hook is called.
pub trait MarkRuntimeContext: Send + Sync {
    fn core_view(&self) -> MarkRenderContext<'_>;

    fn coord_measurement(&self) -> &dyn CoordMeasurement;

    fn facet_path(&self) -> &[ScalarValue];

    fn plot_width(&self) -> f32 {
        self.core_view().plot_width()
    }

    fn plot_height(&self) -> f32 {
        self.core_view().plot_height()
    }
}
