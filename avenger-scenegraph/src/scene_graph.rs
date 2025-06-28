use std::collections::HashMap;

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
            .filter_map(|m| {
                let SceneMark::Group(g) = m else {
                    return None;
                };
                Some(g)
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

        // Walk the path to find the nested mark
        let mut child = self.marks.get(mark_path[0])?;
        for &path_index in mark_path.iter().skip(1) {
            child = child.children().get(path_index)?;
        }

        Some(child)
    }

    /// Returns the absolute origin of a group
    pub fn get_absolute_origin(&self, group_path: &[usize]) -> Option<[f32; 2]> {
        let mut origin = self.origin;

        if group_path.is_empty() {
            return Some(origin);
        }

        // Walk the path to find the nexted mark
        let SceneMark::Group(group) = self.marks.get(group_path[0])? else {
            return None;
        };
        origin = [origin[0] + group.origin[0], origin[1] + group.origin[1]];

        let mut current_group = group;
        for &path_index in group_path.iter().skip(1) {
            let SceneMark::Group(group) = current_group.marks.get(path_index)? else {
                return None;
            };
            origin = [origin[0] + group.origin[0], origin[1] + group.origin[1]];
            current_group = group;
        }

        Some(origin)
    }

    /// Returns all of the group paths in the scene graph
    pub fn group_paths(&self) -> Vec<Vec<usize>> {
        let mut paths = vec![];
        for (index, mark) in self.marks.iter().enumerate() {
            let SceneMark::Group(group) = mark else {
                continue;
            };
            paths.push(vec![index]);
            for sub_path in group.group_paths() {
                let mut path = vec![index];
                path.extend(sub_path);
                paths.push(path);
            }
        }
        paths
    }

    /// Returns the absolute origin of each group
    pub fn group_origins(&self) -> HashMap<Vec<usize>, [f32; 2]> {
        let mut origins = HashMap::new();
        for path in self.group_paths() {
            let origin = self.get_absolute_origin(&path).unwrap();
            origins.insert(path, origin);
        }
        origins
    }

    /// Returns mapping from the names of each named group to their path
    pub fn group_names(&self) -> HashMap<String, Vec<usize>> {
        let mut names = HashMap::new();
        for path in self.group_paths() {
            let SceneMark::Group(group) = self.get_mark(&path).unwrap() else {
                continue;
            };
            names.insert(group.name.clone(), path);
        }
        names
    }
}
