use avenger_common::types::{ColorOrGradient, Gradient};
use avenger_common::value::ScalarOrArray;
use itertools::izip;
use lyon_path::{
    geom::{euclid::Vector2D, Angle, Point, Vector},
    traits::SvgPathBuilder,
    Path,
};
use serde::{Deserialize, Serialize};
use std::ops::{Mul, Neg};
use std::sync::Arc;

use super::{mark::SceneMark, path::PathTransform};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SceneArcMark {
    pub name: String,
    pub clip: bool,
    pub len: u32,
    pub gradients: Vec<Gradient>,
    pub x: ScalarOrArray<f32>,
    pub y: ScalarOrArray<f32>,
    pub start_angle: ScalarOrArray<f32>,
    pub end_angle: ScalarOrArray<f32>,
    pub outer_radius: ScalarOrArray<f32>,
    pub inner_radius: ScalarOrArray<f32>,
    pub pad_angle: ScalarOrArray<f32>,
    pub corner_radius: ScalarOrArray<f32>,
    pub fill: ScalarOrArray<ColorOrGradient>,
    pub stroke: ScalarOrArray<ColorOrGradient>,
    pub stroke_width: ScalarOrArray<f32>,
    pub indices: Option<Arc<Vec<usize>>>,
    pub zindex: Option<i32>,
}

impl SceneArcMark {
    pub fn x_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn y_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn start_angle_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.start_angle
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn end_angle_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.end_angle
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn outer_radius_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.outer_radius
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn inner_radius_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.inner_radius
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn pad_angle_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.pad_angle
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn corner_radius_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.corner_radius
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn fill_iter(&self) -> Box<dyn Iterator<Item = &ColorOrGradient> + '_> {
        self.fill.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn stroke_iter(&self) -> Box<dyn Iterator<Item = &ColorOrGradient> + '_> {
        self.stroke
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn stroke_width_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.stroke_width
            .as_iter(self.len as usize, self.indices.as_ref())
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
                self.start_angle_iter(),
                self.end_angle_iter(),
                self.outer_radius_iter(),
                self.inner_radius_iter()
            )
            .map(
                move |(x, y, start_angle, end_angle, outer_radius, inner_radius)| {
                    // Compute angle
                    let total_angle = end_angle - start_angle;

                    // Normalize inner/outer radius
                    let (inner_radius, outer_radius) = if *inner_radius > *outer_radius {
                        (*outer_radius, *inner_radius)
                    } else {
                        (*inner_radius, *outer_radius)
                    };

                    let mut path_builder = Path::builder().with_svg();

                    // Orient arc starting along vertical y-axis
                    path_builder.move_to(Point::new(0.0, -inner_radius));
                    path_builder.line_to(Point::new(0.0, -outer_radius));

                    // Draw outer arc
                    path_builder.arc(
                        Point::new(0.0, 0.0),
                        Vector::new(outer_radius, outer_radius),
                        Angle::radians(total_angle),
                        Angle::radians(0.0),
                    );

                    if inner_radius != 0.0 {
                        // Compute vector from outer arc corner to arc corner
                        let inner_radius_vec = path_builder
                            .current_position()
                            .to_vector()
                            .neg()
                            .normalize()
                            .mul(outer_radius - inner_radius);
                        path_builder.relative_line_to(inner_radius_vec);

                        // Draw inner
                        path_builder.arc(
                            Point::new(0.0, 0.0),
                            Vector::new(inner_radius, inner_radius),
                            Angle::radians(-total_angle),
                            Angle::radians(0.0),
                        );
                    } else {
                        // Draw line back to origin
                        path_builder.line_to(Point::new(0.0, 0.0));
                    }

                    path_builder.close();

                    // Transform path to account for start angle and position
                    let path = path_builder.build().transformed(
                        &PathTransform::rotation(Angle::radians(*start_angle))
                            .then_translate(Vector2D::new(*x + origin[0], *y + origin[1])),
                    );

                    path
                },
            ),
        )
    }
}

impl Default for SceneArcMark {
    fn default() -> Self {
        Self {
            name: "arc_mark".to_string(),
            clip: true,
            len: 1,
            gradients: vec![],
            x: ScalarOrArray::new_scalar(0.0),
            y: ScalarOrArray::new_scalar(0.0),
            start_angle: ScalarOrArray::new_scalar(0.0),
            end_angle: ScalarOrArray::new_scalar(0.0),
            outer_radius: ScalarOrArray::new_scalar(0.0),
            inner_radius: ScalarOrArray::new_scalar(0.0),
            pad_angle: ScalarOrArray::new_scalar(0.0),
            corner_radius: ScalarOrArray::new_scalar(0.0),
            fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])),
            stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            stroke_width: ScalarOrArray::new_scalar(0.0),
            indices: None,
            zindex: None,
        }
    }
}

impl From<SceneArcMark> for SceneMark {
    fn from(mark: SceneArcMark) -> Self {
        SceneMark::Arc(mark)
    }
}
