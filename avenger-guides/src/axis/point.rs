use avenger_scales::scales::{band::BandScale, ConfiguredScale};
use avenger_scenegraph::marks::group::SceneGroup;

use super::{band::make_band_axis_marks, opts::AxisConfig};
use crate::error::AvengerGuidesError;

pub fn make_point_axis_marks(
    scale: ConfiguredScale,
    title: &str,
    origin: [f32; 2],
    config: &AxisConfig,
) -> Result<SceneGroup, AvengerGuidesError> {
    let band_scale = BandScale::from_point_scale(&scale);
    make_band_axis_marks(&band_scale, title, origin, config)
}

pub fn make_point_axis_marks_with_text_engine(
    scale: ConfiguredScale,
    title: &str,
    origin: [f32; 2],
    config: &AxisConfig,
    text_engine: &avenger_text::TextEngine,
) -> Result<SceneGroup, AvengerGuidesError> {
    let band_scale = BandScale::from_point_scale(&scale);
    super::band::make_band_axis_marks_with_text_engine(
        &band_scale,
        title,
        origin,
        config,
        text_engine,
    )
}
