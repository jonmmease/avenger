use crate::error::VegaSceneGraphError;
use crate::marks::mark::{VegaMark, VegaMarkItem};
use serde::{Deserialize, Serialize};
use sg2d::marks::group::{GroupBounds, SceneGroup};
use sg2d::marks::mark::SceneMark;

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
    pub fn to_scene_graph(&self, origin: [f32; 2]) -> Result<SceneGroup, VegaSceneGraphError> {
        let new_origin = [self.x + origin[0], self.y + origin[1]];
        let mut marks: Vec<SceneMark> = Vec::new();
        for item in &self.items {
            let item_marks: Vec<_> = match item {
                VegaMark::Group(group) => group
                    .items
                    .iter()
                    .map(|item| Ok(SceneMark::Group(item.to_scene_graph(new_origin)?)))
                    .collect::<Result<Vec<_>, VegaSceneGraphError>>()?,
                VegaMark::Rect(mark) => {
                    vec![mark.to_scene_graph(new_origin)?]
                }
                VegaMark::Rule(mark) => {
                    vec![mark.to_scene_graph(new_origin)?]
                }
                VegaMark::Symbol(mark) => {
                    vec![mark.to_scene_graph(new_origin)?]
                }
                VegaMark::Text(mark) => {
                    vec![mark.to_scene_graph(new_origin)?]
                }
                VegaMark::Arc(mark) => {
                    vec![mark.to_scene_graph(new_origin)?]
                }
                VegaMark::Path(mark) => {
                    vec![mark.to_scene_graph(new_origin)?]
                }
                VegaMark::Shape(mark) => {
                    vec![mark.to_scene_graph(new_origin)?]
                }
                VegaMark::Line(mark) => {
                    vec![mark.to_scene_graph(new_origin)?]
                }
                _ => {
                    println!("Mark type not yet supported: {:?}", item);
                    continue;
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
