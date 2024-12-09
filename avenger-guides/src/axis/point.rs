use super::{band::make_band_axis_marks, opts::AxisConfig};
use avenger_scales::point::PointScale;
use avenger_scenegraph::marks::group::SceneGroup;

pub fn make_point_axis_marks(
    scale: &PointScale<String>,
    title: &str,
    origin: [f32; 2],
    config: &AxisConfig,
) -> SceneGroup {
    let band_scale = scale.clone().to_band();
    make_band_axis_marks(&band_scale, title, origin, config)
}
