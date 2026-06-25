use avenger_scales::scales::{band::BandScale, ConfiguredScale};
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_text::measurement::{default_text_measurer, TextMeasurer};

use super::{band::make_band_axis_marks_with_text_measurer, opts::AxisConfig};
use crate::error::AvengerGuidesError;

pub fn make_point_axis_marks(
    scale: ConfiguredScale,
    title: &str,
    origin: [f32; 2],
    config: &AxisConfig,
) -> Result<SceneGroup, AvengerGuidesError> {
    let text_measurer = default_text_measurer();
    make_point_axis_marks_with_text_measurer(scale, title, origin, config, &text_measurer)
}

pub fn make_point_axis_marks_with_text_measurer(
    scale: ConfiguredScale,
    title: &str,
    origin: [f32; 2],
    config: &AxisConfig,
    text_measurer: &dyn TextMeasurer,
) -> Result<SceneGroup, AvengerGuidesError> {
    let band_scale = BandScale::from_point_scale(&scale);
    make_band_axis_marks_with_text_measurer(&band_scale, title, origin, config, text_measurer)
}
