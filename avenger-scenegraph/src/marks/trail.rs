use avenger_common::value::{ColorOrGradient, Gradient, ScalarOrArray};
use avenger_geometry::{lyon_to_geo::IntoGeoType, GeometryInstance};
use itertools::izip;
use lyon_path::{geom::point, Path};
use serde::{Deserialize, Serialize};

use super::mark::SceneMark;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SceneTrailMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub gradients: Vec<Gradient>,
    pub stroke: ColorOrGradient,
    pub x: ScalarOrArray<f32>,
    pub y: ScalarOrArray<f32>,
    pub size: ScalarOrArray<f32>,
    pub defined: ScalarOrArray<bool>,
    pub zindex: Option<i32>,
}

impl SceneTrailMark {
    pub fn x_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x.as_iter(self.len as usize, None)
    }

    pub fn y_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y.as_iter(self.len as usize, None)
    }

    pub fn size_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.size.as_iter(self.len as usize, None)
    }

    pub fn defined_iter(&self) -> Box<dyn Iterator<Item = &bool> + '_> {
        self.defined.as_iter(self.len as usize, None)
    }

    pub fn transformed_path(&self, origin: [f32; 2]) -> Path {
        let mut path_builder = Path::builder_with_attributes(1);
        let mut path_len = 0;
        for (x, y, size, defined) in izip!(
            self.x_iter(),
            self.y_iter(),
            self.size_iter(),
            self.defined_iter()
        ) {
            if *defined {
                if path_len > 0 {
                    // Continue path
                    path_builder.line_to(point(*x + origin[0], *y + origin[1]), &[*size]);
                } else {
                    // New path
                    path_builder.begin(point(*x + origin[0], *y + origin[1]), &[*size]);
                }
                path_len += 1;
            } else {
                if path_len == 1 {
                    // Finishing single point line. Add extra point at the same location
                    // so that stroke caps are drawn
                    path_builder.end(true);
                } else {
                    path_builder.end(false);
                }
                path_len = 0;
            }
        }
        path_builder.end(false);
        path_builder.build()
    }

    pub fn geometry(&self, mark_index: usize) -> GeometryInstance {
        let path = self.transformed_path([0.0, 0.0]);
        let geometry = path.trail_as_geo_type(0.1, 0);
        GeometryInstance {
            mark_index,
            instance_idx: None,
            geometry,
            half_stroke_width: 0.0,
        }
    }
}

impl Default for SceneTrailMark {
    fn default() -> Self {
        Self {
            name: "trail_mark".to_string(),
            clip: true,
            len: 1,
            x: ScalarOrArray::Scalar(0.0),
            y: ScalarOrArray::Scalar(0.0),
            size: ScalarOrArray::Scalar(1.0),
            defined: ScalarOrArray::Scalar(true),
            stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]),
            gradients: vec![],
            zindex: None,
        }
    }
}

impl From<SceneTrailMark> for SceneMark {
    fn from(mark: SceneTrailMark) -> Self {
        SceneMark::Trail(mark)
    }
}
