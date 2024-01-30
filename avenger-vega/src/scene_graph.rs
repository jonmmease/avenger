use crate::error::AvengerVegaError;
use crate::marks::group::VegaGroupItem;
use crate::marks::mark::VegaMarkContainer;
use avenger::scene_graph::SceneGraph;
use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VegaSceneGraph {
    pub width: f32,
    pub height: f32,
    pub origin: [f32; 2],
    pub scenegraph: VegaMarkContainer<VegaGroupItem>,
}

impl VegaSceneGraph {
    pub fn to_scene_graph(&self) -> Result<SceneGraph, AvengerVegaError> {
        let groups = self
            .scenegraph
            .items
            .iter()
            .map(|group| group.to_scene_graph())
            .collect::<Result<Vec<_>, AvengerVegaError>>()?;

        Ok(SceneGraph {
            groups,
            width: self.width,
            height: self.height,
            origin: self.origin,
        })
    }
}
