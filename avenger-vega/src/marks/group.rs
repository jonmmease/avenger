use crate::error::AvengerVegaError;
use crate::marks::mark::{VegaMark, VegaMarkContainer, VegaMarkItem};
use crate::marks::values::CssColorOrGradient;
use avenger_scenegraph::marks::group::{Clip, SceneGroup};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::value::Gradient;
use lyon_extra::euclid::{Box2D, Point2D};
use lyon_path::builder::BorderRadii;
use lyon_path::Winding;
use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VegaGroupItem {
    pub items: Vec<VegaMark>,
    pub clip: Option<bool>,
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub name: Option<String>,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub fill: Option<CssColorOrGradient>,
    pub stroke: Option<CssColorOrGradient>,
    pub stroke_width: Option<f32>,
    pub corner_radius: Option<f32>,
    pub corner_radius_top_left: Option<f32>,
    pub corner_radius_top_right: Option<f32>,
    pub corner_radius_bottom_left: Option<f32>,
    pub corner_radius_bottom_right: Option<f32>,
    pub opacity: Option<f32>,
    pub fill_opacity: Option<f32>,
    pub stroke_opacity: Option<f32>,
    pub stroke_offset: Option<f32>,
    pub zindex: Option<i32>,
}

impl VegaMarkItem for VegaGroupItem {}

impl VegaMarkContainer<VegaGroupItem> {
    pub fn to_scene_graph(&self, force_clip: bool) -> Result<Vec<SceneGroup>, AvengerVegaError> {
        let mut groups: Vec<SceneGroup> = Vec::new();
        for group_item in &self.items {
            let should_clip = group_item.clip.unwrap_or(false) || force_clip;
            let mut marks: Vec<SceneMark> = Vec::new();
            for item in &group_item.items {
                let item_marks: Vec<SceneMark> = match item {
                    VegaMark::Group(mark) => mark
                        .to_scene_graph(should_clip)?
                        .into_iter()
                        .map(SceneMark::Group)
                        .collect(),
                    VegaMark::Rect(mark) => {
                        vec![mark.to_scene_graph(should_clip)?]
                    }
                    VegaMark::Rule(mark) => {
                        vec![mark.to_scene_graph(should_clip)?]
                    }
                    VegaMark::Symbol(mark) => {
                        vec![mark.to_scene_graph(should_clip)?]
                    }
                    VegaMark::Text(mark) => {
                        vec![mark.to_scene_graph(should_clip)?]
                    }
                    VegaMark::Arc(mark) => {
                        vec![mark.to_scene_graph(should_clip)?]
                    }
                    VegaMark::Path(mark) => {
                        vec![mark.to_scene_graph(should_clip)?]
                    }
                    VegaMark::Shape(mark) => {
                        vec![mark.to_scene_graph(should_clip)?]
                    }
                    VegaMark::Line(mark) => {
                        vec![mark.to_scene_graph(should_clip)?]
                    }
                    VegaMark::Area(mark) => {
                        vec![mark.to_scene_graph(should_clip)?]
                    }
                    VegaMark::Trail(mark) => {
                        vec![mark.to_scene_graph(should_clip)?]
                    }
                    VegaMark::Image(mark) => {
                        vec![mark.to_scene_graph(should_clip)?]
                    }
                };
                marks.extend(item_marks);
            }

            let mut gradients = Vec::<Gradient>::new();
            let fill = if let Some(v) = &group_item.fill {
                let opacity =
                    group_item.fill_opacity.unwrap_or(1.0) * group_item.opacity.unwrap_or(1.0);
                Some(v.to_color_or_grad(opacity, &mut gradients)?)
            } else {
                None
            };
            let stroke = if let Some(v) = &group_item.stroke {
                let opacity =
                    group_item.fill_opacity.unwrap_or(1.0) * group_item.opacity.unwrap_or(1.0);
                Some(v.to_color_or_grad(opacity, &mut gradients)?)
            } else {
                None
            };

            let clip = if let (Some(width), Some(height)) = (group_item.width, group_item.height) {
                let corner_radius = group_item.corner_radius.unwrap_or(0.0);
                let corner_radius_top_left =
                    group_item.corner_radius_top_left.unwrap_or(corner_radius);
                let corner_radius_top_right =
                    group_item.corner_radius_top_right.unwrap_or(corner_radius);
                let corner_radius_bottom_left = group_item
                    .corner_radius_bottom_left
                    .unwrap_or(corner_radius);
                let corner_radius_bottom_right = group_item
                    .corner_radius_bottom_right
                    .unwrap_or(corner_radius);

                if corner_radius_top_left > 0.0
                    || corner_radius_top_right > 0.0
                    || corner_radius_bottom_left > 0.0
                    || corner_radius_bottom_right > 0.0
                {
                    // Rounded rectange path
                    let mut builder = lyon_path::Path::builder();
                    builder.add_rounded_rectangle(
                        &Box2D::new(Point2D::new(0.0, 0.0), Point2D::new(width, height)),
                        &BorderRadii {
                            top_left: corner_radius_top_left,
                            top_right: corner_radius_top_right,
                            bottom_left: corner_radius_bottom_left,
                            bottom_right: corner_radius_bottom_right,
                        },
                        Winding::Positive,
                    );
                    Clip::Path(builder.build())
                } else {
                    // Rect
                    Clip::Rect {
                        x: 0.0, // x and y are zero to align with origin
                        y: 0.0,
                        width,
                        height,
                    }
                }
            } else {
                Clip::None
            };

            groups.push(SceneGroup {
                name: self.name.clone().unwrap_or("group_mark".to_string()),
                zindex: self.zindex,
                origin: [group_item.x.unwrap_or(0.0), group_item.y.unwrap_or(0.0)],
                clip,
                marks,
                gradients,
                fill,
                stroke,
                stroke_width: group_item.stroke_width,
                stroke_offset: group_item.stroke_offset,
            })
        }
        Ok(groups)
    }
}
