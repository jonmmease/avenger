use crate::error::VegaSceneGraphError;
use crate::marks::mark::{VegaMark, VegaMarkItem};
use avenger::marks::group::{GroupBounds, SceneGroup};
use avenger::marks::mark::SceneMark;
use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VegaGroupItem {
    pub items: Vec<VegaMark>,
    #[serde(default)]
    pub(crate) x: f32,
    #[serde(default)]
    pub(crate) y: f32,
    pub width: Option<f32>,
    pub height: Option<f32>,
}

impl VegaMarkItem for VegaGroupItem {}

impl VegaGroupItem {
    pub fn to_scene_graph(&self) -> Result<SceneGroup, VegaSceneGraphError> {
        let mut marks: Vec<SceneMark> = Vec::new();
        for item in &self.items {
            let item_marks: Vec<_> = match item {
                VegaMark::Group(group) => group
                    .items
                    .iter()
                    .map(|item| Ok(SceneMark::Group(item.to_scene_graph()?)))
                    .collect::<Result<Vec<_>, VegaSceneGraphError>>()?,
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
        Ok(SceneGroup {
            bounds: GroupBounds {
                x: self.x,
                y: self.y,
                width: self.width,
                height: self.height,
            },
            marks,
        })
    }
}