use avenger_common::value::{ColorOrGradient, Gradient, ScalarOrArray};
use avenger_geometry::{lyon_to_geo::IntoGeoType, GeometryInstance};
use geo::{Coord, Geometry, Rect};
use itertools::izip;
use lyon_extra::euclid::Point2D;
use lyon_path::{builder::BorderRadii, geom::Box2D, Path, Winding};
use serde::{Deserialize, Serialize};

use super::mark::SceneMark;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SceneRectMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub gradients: Vec<Gradient>,
    pub x: ScalarOrArray<f32>,
    pub y: ScalarOrArray<f32>,
    pub width: Option<ScalarOrArray<f32>>,
    pub height: Option<ScalarOrArray<f32>>,
    pub x2: Option<ScalarOrArray<f32>>,
    pub y2: Option<ScalarOrArray<f32>>,
    pub fill: ScalarOrArray<ColorOrGradient>,
    pub stroke: ScalarOrArray<ColorOrGradient>,
    pub stroke_width: ScalarOrArray<f32>,
    pub corner_radius: ScalarOrArray<f32>,
    pub indices: Option<Vec<usize>>,
    pub zindex: Option<i32>,
}

impl SceneRectMark {
    pub fn x_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn x_vec(&self) -> Vec<f32> {
        self.x.as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn y_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn y_vec(&self) -> Vec<f32> {
        self.y.as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn width_iter(&self) -> Box<dyn Iterator<Item = f32> + '_> {
        if let Some(width) = self.width.as_ref() {
            // We have width
            width.as_iter_owned(self.len as usize, self.indices.as_ref())
        } else if let Some(x2) = self.x2.as_ref() {
            // Compute width from x2 and x
            Box::new(
                self.x_iter()
                    .zip(x2.as_iter(self.len as usize, self.indices.as_ref()))
                    .map(|(x, x2)| x2 - x),
            )
        } else {
            // Default to width 1
            Box::new(std::iter::repeat(1.0).take(self.len as usize))
        }
    }

    pub fn width_vec(&self) -> Vec<f32> {
        self.width_iter().collect()
    }

    pub fn height_iter(&self) -> Box<dyn Iterator<Item = f32> + '_> {
        if let Some(height) = self.height.as_ref() {
            // We have height
            height.as_iter_owned(self.len as usize, self.indices.as_ref())
        } else if let Some(y2) = self.y2.as_ref() {
            // Compute width from y2 and y
            Box::new(
                self.y_iter()
                    .zip(y2.as_iter(self.len as usize, self.indices.as_ref()))
                    .map(|(y, y2)| y2 - y),
            )
        } else {
            // Default to height 1
            Box::new(std::iter::repeat(1.0).take(self.len as usize))
        }
    }

    pub fn height_vec(&self) -> Vec<f32> {
        self.height_iter().collect()
    }

    pub fn x2_iter(&self) -> Box<dyn Iterator<Item = f32> + '_> {
        if let Some(x2) = self.x2.as_ref() {
            // We have x2
            x2.as_iter_owned(self.len as usize, self.indices.as_ref())
        } else if let Some(width) = self.width.as_ref() {
            // Compute x2 from x and width
            Box::new(
                self.x_iter()
                    .zip(width.as_iter(self.len as usize, self.indices.as_ref()))
                    .map(|(x, width)| x + width),
            )
        } else {
            // Default to x + 1
            Box::new(self.x_iter().map(|x| x + 1.0))
        }
    }

    pub fn x2_vec(&self) -> Vec<f32> {
        self.x2_iter().collect()
    }

    pub fn y2_iter(&self) -> Box<dyn Iterator<Item = f32> + '_> {
        if let Some(y2) = self.y2.as_ref() {
            // We have y2
            y2.as_iter_owned(self.len as usize, self.indices.as_ref())
        } else if let Some(height) = self.height.as_ref() {
            // Compute y2 from y and height
            Box::new(
                self.y_iter()
                    .zip(height.as_iter(self.len as usize, self.indices.as_ref()))
                    .map(|(y, height)| y + height),
            )
        } else {
            // Default to y + 1
            Box::new(self.y_iter().map(|y| y + 1.0))
        }
    }

    pub fn y2_vec(&self) -> Vec<f32> {
        self.y2_iter().collect()
    }

    pub fn fill_iter(&self) -> Box<dyn Iterator<Item = &ColorOrGradient> + '_> {
        self.fill.as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn fill_vec(&self) -> Vec<ColorOrGradient> {
        self.fill.as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn stroke_iter(&self) -> Box<dyn Iterator<Item = &ColorOrGradient> + '_> {
        self.stroke
            .as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn stroke_vec(&self) -> Vec<ColorOrGradient> {
        self.stroke.as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn stroke_width_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.stroke_width
            .as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn stroke_width_vec(&self) -> Vec<f32> {
        self.stroke_width
            .as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn corner_radius_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.corner_radius
            .as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn corner_radius_vec(&self) -> Vec<f32> {
        self.corner_radius
            .as_vec(self.len as usize, self.indices.as_ref())
    }

    pub fn indices_iter(&self) -> Box<dyn Iterator<Item = usize> + '_> {
        if let Some(indices) = self.indices.as_ref() {
            Box::new(indices.iter().cloned())
        } else {
            Box::new((0..self.len as usize).into_iter())
        }
    }

    pub fn transformed_path_iter(&self, origin: [f32; 2]) -> Box<dyn Iterator<Item = Path> + '_> {
        Box::new(
            izip!(
                self.x_iter(),
                self.y_iter(),
                self.x2_iter(),
                self.y2_iter(),
                self.corner_radius_iter()
            )
            .map(move |(x, y, x2, y2, corner_radius)| {
                // Create rect path
                let mut path_builder = Path::builder();
                let x0 = f32::min(*x + origin[0], x2 + origin[0]);
                let x1 = f32::max(*x + origin[0], x2 + origin[0]);
                let y0 = f32::min(*y + origin[1], y2 + origin[1]);
                let y1 = f32::max(*y + origin[1], y2 + origin[1]);

                if *corner_radius > 0.0 {
                    path_builder.add_rounded_rectangle(
                        &Box2D::new(Point2D::new(x0, y0), Point2D::new(x1, y1)),
                        &BorderRadii {
                            top_left: *corner_radius,
                            top_right: *corner_radius,
                            bottom_left: *corner_radius,
                            bottom_right: *corner_radius,
                        },
                        Winding::Positive,
                    );
                } else {
                    path_builder.add_rectangle(
                        &Box2D::new(Point2D::new(x0, y0), Point2D::new(x1, y1)),
                        Winding::Positive,
                    );
                }

                path_builder.build()
            }),
        )
    }

    pub fn geometry_iter(&self) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        if self.corner_radius.equals_scalar(0.0) {
            // Simple case where we don't need to build lyon paths first
            Box::new(
                izip!(
                    self.indices_iter(),
                    self.x_iter(),
                    self.y_iter(),
                    self.x2_iter(),
                    self.y2_iter(),
                    self.stroke_width_iter()
                )
                .map(|(id, x, y, x2, y2, stroke_width)| {
                    // Create rect geometry
                    let x0 = f32::min(*x, x2);
                    let x1 = f32::max(*x, x2);
                    let y0 = f32::min(*y, y2);
                    let y1 = f32::max(*y, y2);

                    let geometry = Geometry::Rect(Rect::<f32>::new(
                        Coord { x: x0, y: y0 },
                        Coord { x: x1, y: y1 },
                    ));
                    GeometryInstance {
                        id,
                        geometry,
                        half_stroke_width: *stroke_width / 2.0,
                    }
                }),
            )
        } else {
            // General case
            Box::new(
                izip!(
                    self.indices_iter(),
                    self.transformed_path_iter([0.0, 0.0]),
                    self.stroke_width_iter()
                )
                .map(move |(id, path, stroke_width)| {
                    let half_stroke_width = stroke_width / 2.0;
                    let geometry = path.as_geo_type(0.1, true);
                    GeometryInstance {
                        id,
                        geometry,
                        half_stroke_width,
                    }
                }),
            )
        }
    }
}

impl Default for SceneRectMark {
    fn default() -> Self {
        Self {
            name: "rule_mark".to_string(),
            clip: true,
            len: 1,
            gradients: vec![],
            x: ScalarOrArray::Scalar(0.0),
            y: ScalarOrArray::Scalar(0.0),
            width: None,
            height: None,
            x2: None,
            y2: None,
            fill: ScalarOrArray::Scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            stroke: ScalarOrArray::Scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            stroke_width: ScalarOrArray::Scalar(0.0),
            corner_radius: ScalarOrArray::Scalar(0.0),
            indices: None,
            zindex: None,
        }
    }
}

impl From<SceneRectMark> for SceneMark {
    fn from(mark: SceneRectMark) -> Self {
        SceneMark::Rect(mark)
    }
}
