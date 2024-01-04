use crate::error::VegaSceneGraphError;
use crate::marks::group::VegaGroupItem;
use crate::marks::mark::VegaMarkContainer;
use sg2d::scene_graph::SceneGraph;

pub type VegaSceneGraph = VegaMarkContainer<VegaGroupItem>;

impl VegaSceneGraph {
    pub fn to_scene_graph(
        &self,
        origin: [f32; 2],
        width: f32,
        height: f32,
    ) -> Result<SceneGraph, VegaSceneGraphError> {
        let groups = self
            .items
            .iter()
            .map(|group| group.to_scene_graph(origin))
            .collect::<Result<Vec<_>, VegaSceneGraphError>>()?;

        Ok(SceneGraph {
            groups,
            width,
            height,
        })
    }
}
