use crate::marks::{group::SceneGroup, mark::SceneMark};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneGraph {
    pub marks: Vec<SceneMark>,
    pub width: f32,
    pub height: f32,
    pub origin: [f32; 2],
}

impl SceneGraph {
    pub fn groups(&self) -> Vec<&SceneGroup> {
        self.marks
            .iter()
            .filter_map(|m| match m {
                SceneMark::Group(g) => Some(g),
                _ => None,
            })
            .collect()
    }

    pub fn children(&self) -> &[SceneMark] {
        &self.marks
    }

    pub fn get_mark(&self, mark_path: &[usize]) -> Option<&SceneMark> {
        // empty path is the root, which is not a mark
        if mark_path.is_empty() {
            return None;
        }

        // Walk the path to find the nexted mark
        let mut child = self.marks.get(mark_path[0])?;
        for index in 1..mark_path.len() {
            child = child.children().get(mark_path[index])?;
        }

        Some(child)
    }
}
