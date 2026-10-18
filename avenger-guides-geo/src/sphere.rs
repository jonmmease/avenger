//! The sphere: the projection's whole world.

use avenger_color::ColorOrGradient;
use avenger_common::types::StrokeJoin;
use avenger_geo::paths::geometry_path;
use avenger_geo::Projector;
use avenger_scenegraph::marks::{group::SceneGroup, path::ScenePathMark};

/// How the sphere is drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct SphereStyle {
    pub fill: [f32; 4],
    pub stroke: [f32; 4],
    pub stroke_width: f32,
}

impl Default for SphereStyle {
    fn default() -> Self {
        Self {
            fill: [0.94, 0.97, 1.0, 1.0],
            stroke: [0.45, 0.45, 0.45, 1.0],
            stroke_width: 1.0,
        }
    }
}

/// The sphere through `projector`: a group with a path mark that fills the
/// projection's outline and strokes it, or no marks when the outline is empty.
/// Draw it before the map's data.
pub fn make_sphere_marks(projector: &Projector, style: &SphereStyle) -> SceneGroup {
    let path = geometry_path(&projector.project_sphere());
    let marks = if path.iter().next().is_some() {
        vec![ScenePathMark {
            path: path.into(),
            fill: ColorOrGradient::Color(style.fill).into(),
            stroke: ColorOrGradient::Color(style.stroke).into(),
            stroke_width: Some(style.stroke_width),
            stroke_join: StrokeJoin::Round,
            ..Default::default()
        }
        .into()]
    } else {
        Vec::new()
    };
    SceneGroup {
        interactive: Some(false),
        marks,
        ..Default::default()
    }
}
