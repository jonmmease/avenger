use crate::error::AvengerGuidesError;

use super::{band::make_band_axis_marks, opts::AxisConfig};
use avenger_scales3::scales::{band::BandScale, ConfiguredScale};
use avenger_scenegraph::marks::group::SceneGroup;

pub fn make_point_axis_marks(
    scale: ConfiguredScale,
    title: &str,
    origin: [f32; 2],
    config: &AxisConfig,
) -> Result<SceneGroup, AvengerGuidesError> {
    let band_scale = BandScale::from_point_scale(&scale);
    make_band_axis_marks(&band_scale, title, origin, config)
}
