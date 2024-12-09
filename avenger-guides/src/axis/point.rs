use super::{band::make_band_axis_marks, opts::AxisConfig};
use avenger_scales::point::PointScale;
use avenger_scenegraph::marks::group::SceneGroup;
use std::{fmt::Debug, hash::Hash};

pub fn make_point_axis_marks<T>(
    scale: &PointScale<T>,
    title: &str,
    origin: [f32; 2],
    config: &AxisConfig,
) -> SceneGroup
where
    T: ToString + Debug + Clone + Hash + Eq + Sync + 'static,
{
    let band_scale = scale.clone().to_band();
    make_band_axis_marks(&band_scale, title, origin, config)
}
