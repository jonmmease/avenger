use std::{
    ops::{Mul, Neg},
    sync::Arc,
};

use avenger_color::{ColorOrGradient, Gradient};
use avenger_common::{types::PathTransform, value::ScalarOrArray};
use itertools::izip;
use lyon_path::{
    geom::{euclid::Vector2D, Angle, Point, Vector},
    traits::SvgPathBuilder,
    Path,
};
use serde::{Deserialize, Serialize};

use super::mark::{default_interactive, SceneMark};

#[derive(Debug, Clone, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SceneArcMark {
    pub name: String,
    #[serde(default = "default_interactive")]
    pub interactive: bool,
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
            Box::new(0..self.len as usize)
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
                self.inner_radius_iter(),
                self.pad_angle_iter()
            )
            .map(
                move |(x, y, start_angle, end_angle, outer_radius, inner_radius, pad_angle)| {
                    // Compute angle
                    let (start_angle, end_angle) =
                        padded_angles(*start_angle, *end_angle, *pad_angle);
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

                    path_builder.build().transformed(
                        &PathTransform::rotation(Angle::radians(start_angle))
                            .then_translate(Vector2D::new(*x + origin[0], *y + origin[1])),
                    )
                },
            ),
        )
    }
}

fn padded_angles(start_angle: f32, end_angle: f32, pad_angle: f32) -> (f32, f32) {
    const TAU: f32 = std::f32::consts::TAU;
    const EPSILON: f32 = 1e-6;

    let total_angle = end_angle - start_angle;
    let abs_angle = total_angle.abs();
    let pad_angle = pad_angle.abs();
    if pad_angle <= EPSILON || abs_angle <= EPSILON || abs_angle >= TAU - EPSILON {
        return (start_angle, end_angle);
    }

    let pad = (pad_angle * 0.5).min(abs_angle * 0.5);
    let direction = total_angle.signum();
    (start_angle + direction * pad, end_angle - direction * pad)
}

impl Default for SceneArcMark {
    fn default() -> Self {
        Self {
            name: "arc_mark".to_string(),
            interactive: true,
            clip: true,
            len: 1,
            gradients: vec![],
            x: ScalarOrArray::new_scalar(0.0),
            y: ScalarOrArray::new_scalar(0.0),
            start_angle: ScalarOrArray::new_scalar(0.0),
            end_angle: ScalarOrArray::new_scalar(0.7),
            outer_radius: ScalarOrArray::new_scalar(10.0),
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

#[cfg(test)]
mod tests {
    use avenger_common::value::ScalarOrArray;
    use lyon_path::Event;

    use super::*;

    #[test]
    fn transformed_path_iter_applies_pad_angle_to_arc_endpoints() {
        let unpadded = SceneArcMark {
            start_angle: ScalarOrArray::new_scalar(0.0),
            end_angle: ScalarOrArray::new_scalar(std::f32::consts::FRAC_PI_2),
            inner_radius: ScalarOrArray::new_scalar(5.0),
            outer_radius: ScalarOrArray::new_scalar(10.0),
            pad_angle: ScalarOrArray::new_scalar(0.0),
            ..Default::default()
        };
        let padded = SceneArcMark {
            pad_angle: ScalarOrArray::new_scalar(0.2),
            ..unpadded.clone()
        };

        let unpadded_path = unpadded.transformed_path_iter([0.0, 0.0]).next().unwrap();
        let padded_path = padded.transformed_path_iter([0.0, 0.0]).next().unwrap();

        let unpadded_outer_start = first_line_to(&unpadded_path);
        let padded_outer_start = first_line_to(&padded_path);
        assert!(unpadded_outer_start.x.abs() <= 1e-4);
        assert!(padded_outer_start.x.abs() > 0.1);
        assert!((unpadded_outer_start.x - padded_outer_start.x).abs() > 0.1);
    }

    #[test]
    fn full_circle_arc_does_not_apply_pad_angle() {
        let unpadded = SceneArcMark {
            start_angle: ScalarOrArray::new_scalar(0.0),
            end_angle: ScalarOrArray::new_scalar(std::f32::consts::TAU),
            inner_radius: ScalarOrArray::new_scalar(5.0),
            outer_radius: ScalarOrArray::new_scalar(10.0),
            pad_angle: ScalarOrArray::new_scalar(0.0),
            ..Default::default()
        };
        let padded = SceneArcMark {
            pad_angle: ScalarOrArray::new_scalar(0.4),
            ..unpadded.clone()
        };

        let unpadded_path = unpadded.transformed_path_iter([0.0, 0.0]).next().unwrap();
        let padded_path = padded.transformed_path_iter([0.0, 0.0]).next().unwrap();

        let unpadded_outer_start = first_line_to(&unpadded_path);
        let padded_outer_start = first_line_to(&padded_path);
        assert!((unpadded_outer_start.x - padded_outer_start.x).abs() <= 1e-4);
        assert!((unpadded_outer_start.y - padded_outer_start.y).abs() <= 1e-4);
    }

    fn first_line_to(path: &lyon_path::Path) -> lyon_path::math::Point {
        path.iter()
            .find_map(|event| match event {
                Event::Line { to, .. } => Some(to),
                _ => None,
            })
            .expect("arc path should include line to outer radius")
    }
}
