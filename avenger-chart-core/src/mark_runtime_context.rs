use datafusion::common::ScalarValue;

use avenger_scenegraph::marks::group::Clip;

use crate::{BasePlotAreaScene, CoordMeasurement, MarkRenderContext, TextMeasurementService};

/// Core runtime view passed to compiled mark renderers.
///
/// Ordinary marks should use `core_view()` plus the coordinate transform they
/// receive at render time. Top-level layout-owned marks are dispatched by the
/// facade before the generic mark render hook is called.
pub trait MarkRuntimeContext: Send + Sync {
    fn core_view(&self) -> MarkRenderContext<'_>;

    fn coord_measurement(&self) -> &dyn CoordMeasurement;

    fn facet_path(&self) -> &[ScalarValue];

    fn base_plot_area_scene(&self) -> Option<&BasePlotAreaScene> {
        None
    }

    fn text_measurement_service(&self) -> Option<&dyn TextMeasurementService> {
        None
    }

    fn plot_area_clip(&self) -> Option<&Clip> {
        None
    }

    fn plot_area_origin(&self) -> [f32; 2] {
        [0.0, 0.0]
    }

    fn plot_width(&self) -> f32 {
        self.core_view().plot_width()
    }

    fn plot_height(&self) -> f32 {
        self.core_view().plot_height()
    }
}
