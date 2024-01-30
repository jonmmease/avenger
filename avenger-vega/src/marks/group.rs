use crate::error::AvengerVegaError;
use crate::marks::mark::{VegaMark, VegaMarkItem};
use crate::marks::values::CssColorOrGradient;
use avenger::marks::group::{GroupBounds, SceneGroup};
use avenger::marks::mark::SceneMark;
use avenger::marks::value::Gradient;
use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VegaGroupItem {
    pub items: Vec<VegaMark>,
    #[serde(default)]
    pub(crate) x: f32,
    #[serde(default)]
    pub(crate) y: f32,
    pub name: Option<String>,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub fill: Option<CssColorOrGradient>,
    pub stroke: Option<CssColorOrGradient>,
    pub stroke_width: Option<f32>,
    pub corner_radius: Option<f32>,
    pub opacity: Option<f32>,
    pub fill_opacity: Option<f32>,
    pub stroke_opacity: Option<f32>,
    pub stroke_offset: Option<f32>,
    pub zindex: Option<i32>,
}

impl VegaMarkItem for VegaGroupItem {}

impl VegaGroupItem {
    pub fn to_scene_graph(&self) -> Result<SceneGroup, AvengerVegaError> {
        let mut marks: Vec<SceneMark> = Vec::new();
        for item in &self.items {
            let item_marks: Vec<_> = match item {
                VegaMark::Group(group) => group
                    .items
                    .iter()
                    .map(|item| Ok(SceneMark::Group(item.to_scene_graph()?)))
                    .collect::<Result<Vec<_>, AvengerVegaError>>()?,
                VegaMark::Rect(mark) => {
                    vec![mark.to_scene_graph()?]
                }
                VegaMark::Rule(mark) => {
                    vec![mark.to_scene_graph()?]
                }
                VegaMark::Symbol(mark) => {
                    vec![mark.to_scene_graph()?]
                }
                VegaMark::Text(mark) => {
                    vec![mark.to_scene_graph()?]
                }
                VegaMark::Arc(mark) => {
                    vec![mark.to_scene_graph()?]
                }
                VegaMark::Path(mark) => {
                    vec![mark.to_scene_graph()?]
                }
                VegaMark::Shape(mark) => {
                    vec![mark.to_scene_graph()?]
                }
                VegaMark::Line(mark) => {
                    vec![mark.to_scene_graph()?]
                }
                VegaMark::Area(mark) => {
                    vec![mark.to_scene_graph()?]
                }
                VegaMark::Trail(mark) => {
                    vec![mark.to_scene_graph()?]
                }
                VegaMark::Image(mark) => {
                    vec![mark.to_scene_graph()?]
                }
            };
            marks.extend(item_marks);
        }

        let mut gradients = Vec::<Gradient>::new();
        let fill = if let Some(v) = &self.fill {
            let opacity = self.fill_opacity.unwrap_or(1.0) * self.opacity.unwrap_or(1.0);
            Some(v.to_color_or_grad(opacity, &mut gradients)?)
        } else {
            None
        };
        let stroke = if let Some(v) = &self.stroke {
            let opacity = self.fill_opacity.unwrap_or(1.0) * self.opacity.unwrap_or(1.0);
            Some(v.to_color_or_grad(opacity, &mut gradients)?)
        } else {
            None
        };

        Ok(SceneGroup {
            name: self
                .name
                .clone()
                .unwrap_or_else(|| "group_item".to_string()),
            bounds: GroupBounds {
                x: self.x,
                y: self.y,
                width: self.width,
                height: self.height,
            },
            marks,
            gradients,
            fill,
            stroke,
            stroke_width: self.stroke_width,
            stroke_offset: self.stroke_offset,
            corner_radius: self.corner_radius,
        })
    }
}
