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
    #[tracing::instrument(skip_all)]
    pub fn to_scene_graph(&self) -> Result<SceneGraph, AvengerVegaError> {
        Ok(SceneGraph {
            groups: self.scenegraph.to_scene_graph(false)?,
            width: self.width,
            height: self.height,
            origin: self.origin,
        })
    }
}
