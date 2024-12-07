use avenger_common::value::{ColorOrGradient, Gradient, ScalarOrArray, StrokeCap};
use avenger_geometry::{lyon_to_geo::IntoGeoType, GeometryInstance};
use itertools::izip;
use lyon_path::{geom::Point, Path};
use serde::{Deserialize, Serialize};

use super::mark::SceneMark;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SceneRuleMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub gradients: Vec<Gradient>,
    pub stroke_dash: Option<ScalarOrArray<Vec<f32>>>,
    pub x0: ScalarOrArray<f32>,
    pub y0: ScalarOrArray<f32>,
    pub x1: ScalarOrArray<f32>,
    pub y1: ScalarOrArray<f32>,
    pub stroke: ScalarOrArray<ColorOrGradient>,
    pub stroke_width: ScalarOrArray<f32>,
    pub stroke_cap: ScalarOrArray<StrokeCap>,
    pub indices: Option<Vec<usize>>,
    pub zindex: Option<i32>,
}

impl SceneRuleMark {
    pub fn x0_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x0.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn y0_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y0.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn x1_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x1.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn y1_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y1.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn stroke_iter(&self) -> Box<dyn Iterator<Item = &ColorOrGradient> + '_> {
        self.stroke
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn stroke_width_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.stroke_width
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn stroke_cap_iter(&self) -> Box<dyn Iterator<Item = &StrokeCap> + '_> {
        self.stroke_cap
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn stroke_dash_iter(&self) -> Option<Box<dyn Iterator<Item = &Vec<f32>> + '_>> {
        if let Some(stroke_dash) = &self.stroke_dash {
            Some(stroke_dash.as_iter(self.len as usize, self.indices.as_ref()))
        } else {
            None
        }
    }

    pub fn indices_iter(&self) -> Box<dyn Iterator<Item = usize> + '_> {
        if let Some(indices) = self.indices.as_ref() {
            Box::new(indices.iter().cloned())
        } else {
            Box::new((0..self.len as usize).into_iter())
        }
    }

    pub fn transformed_path_iter(&self, origin: [f32; 2]) -> Box<dyn Iterator<Item = Path> + '_> {
        if let Some(stroke_dash_iter) = self.stroke_dash_iter() {
            Box::new(
                izip!(
                    self.x0_iter(),
                    self.y0_iter(),
                    self.x1_iter(),
                    self.y1_iter(),
                    stroke_dash_iter
                )
                .map(move |(x0, y0, x1, y1, stroke_dash)| {
                    // Next index into stroke_dash array
                    let mut dash_idx = 0;

                    // Distance along line from (x0,y0) to (x1,y1) where the next dash will start
                    let mut start_dash_dist: f32 = 0.0;

                    // Length of the line from (x0,y0) to (x1,y1)
                    let rule_len = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();

                    // Components of unit vector along (x0,y0) to (x1,y1)
                    let xhat = (x1 - x0) / rule_len;
                    let yhat = (y1 - y0) / rule_len;

                    // Whether the next dash length represents a drawn dash (draw == true)
                    // or a gap (draw == false)
                    let mut draw = true;

                    // Init path builder
                    let mut path_builder = Path::builder().with_svg();

                    while start_dash_dist < rule_len {
                        let end_dash_dist = if start_dash_dist + stroke_dash[dash_idx] >= rule_len {
                            // The final dash/gap should be truncated to the end of the rule
                            rule_len
                        } else {
                            // The dash/gap fits entirely in the rule
                            start_dash_dist + stroke_dash[dash_idx]
                        };

                        if draw {
                            let dash_x0 = x0 + xhat * start_dash_dist;
                            let dash_y0 = y0 + yhat * start_dash_dist;
                            let dash_x1 = x0 + xhat * end_dash_dist;
                            let dash_y1 = y0 + yhat * end_dash_dist;

                            path_builder
                                .move_to(Point::new(dash_x0 + origin[0], dash_y0 + origin[1]));
                            path_builder
                                .line_to(Point::new(dash_x1 + origin[0], dash_y1 + origin[1]));
                        }

                        // update start dist for next dash/gap
                        start_dash_dist = end_dash_dist;

                        // increment index and cycle back to start of start of dash array
                        dash_idx = (dash_idx + 1) % stroke_dash.len();

                        // Alternate between drawn dash and gap
                        draw = !draw;
                    }

                    path_builder.build()
                }),
            )
        } else {
            Box::new(
                izip!(
                    self.x0_iter(),
                    self.y0_iter(),
                    self.x1_iter(),
                    self.y1_iter(),
                )
                .map(move |(x0, y0, x1, y1)| {
                    let mut path_builder = Path::builder().with_svg();
                    path_builder.move_to(Point::new(*x0 + origin[0], *y0 + origin[1]));
                    path_builder.line_to(Point::new(*x1 + origin[0], *y1 + origin[1]));
                    path_builder.build()
                }),
            )
        }
    }

    pub fn geometry_iter(
        &self,
        mark_index: usize,
    ) -> Box<dyn Iterator<Item = GeometryInstance> + '_> {
        Box::new(
            izip!(
                self.indices_iter(),
                self.transformed_path_iter([0.0, 0.0]),
                self.stroke_width_iter(),
            )
            .map(move |(id, path, stroke_width)| {
                let half_stroke_width = stroke_width / 2.0;
                let geometry = path.as_geo_type(0.1, false);
                GeometryInstance {
                    mark_index,
                    instance_idx: Some(id),
                    geometry,
                    half_stroke_width,
                }
            }),
        )
    }
}

impl Default for SceneRuleMark {
    fn default() -> Self {
        Self {
            name: "rule_mark".to_string(),
            clip: true,
            len: 1,
            gradients: vec![],
            stroke_dash: None,
            x0: ScalarOrArray::Scalar(0.0),
            y0: ScalarOrArray::Scalar(0.0),
            x1: ScalarOrArray::Scalar(0.0),
            y1: ScalarOrArray::Scalar(0.0),
            stroke: ScalarOrArray::Scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])),
            stroke_width: ScalarOrArray::Scalar(1.0),
            stroke_cap: ScalarOrArray::Scalar(StrokeCap::Butt),
            indices: None,
            zindex: None,
        }
    }
}

impl From<SceneRuleMark> for SceneMark {
    fn from(mark: SceneRuleMark) -> Self {
        SceneMark::Rule(mark)
    }
}
