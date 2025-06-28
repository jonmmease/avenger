use super::mark::SceneMark;
use avenger_common::types::{ColorOrGradient, Gradient, StrokeCap};
use avenger_common::value::ScalarOrArray;
use itertools::izip;
use lyon_path::{geom::Point, Path};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SceneRuleMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub gradients: Vec<Gradient>,
    pub stroke_dash: Option<ScalarOrArray<Vec<f32>>>,
    pub x: ScalarOrArray<f32>,
    pub y: ScalarOrArray<f32>,
    pub x2: ScalarOrArray<f32>,
    pub y2: ScalarOrArray<f32>,
    pub stroke: ScalarOrArray<ColorOrGradient>,
    pub stroke_width: ScalarOrArray<f32>,
    pub stroke_cap: ScalarOrArray<StrokeCap>,
    pub indices: Option<Arc<Vec<usize>>>,
    pub zindex: Option<i32>,
}

impl SceneRuleMark {
    pub fn x_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn y_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn x2_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x2.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn y2_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y2.as_iter(self.len as usize, self.indices.as_ref())
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
            Box::new(0..self.len as usize)
        }
    }

    pub fn transformed_path_iter(&self, origin: [f32; 2]) -> Box<dyn Iterator<Item = Path> + '_> {
        if let Some(stroke_dash_iter) = self.stroke_dash_iter() {
            Box::new(
                izip!(
                    self.x_iter(),
                    self.y_iter(),
                    self.x2_iter(),
                    self.y2_iter(),
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
                izip!(self.x_iter(), self.y_iter(), self.x2_iter(), self.y2_iter(),).map(
                    move |(x0, y0, x1, y1)| {
                        let mut path_builder = Path::builder().with_svg();
                        path_builder.move_to(Point::new(*x0 + origin[0], *y0 + origin[1]));
                        path_builder.line_to(Point::new(*x1 + origin[0], *y1 + origin[1]));
                        path_builder.build()
                    },
                ),
            )
        }
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
            x: ScalarOrArray::new_scalar(0.0),
            y: ScalarOrArray::new_scalar(0.0),
            x2: ScalarOrArray::new_scalar(0.0),
            y2: ScalarOrArray::new_scalar(0.0),
            stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])),
            stroke_width: ScalarOrArray::new_scalar(1.0),
            stroke_cap: ScalarOrArray::new_scalar(StrokeCap::Butt),
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
