use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use avenger_common::hash::hash_floats;
use educe::Educe;
use serde::{Deserialize, Serialize};

use avenger_common::value::ScalarOrArrayValue;
use avenger_image::{ImageResolver, RgbaImage};

use crate::marks::{group::SceneGroup, image::SceneImageSource, mark::SceneMark};

#[derive(Debug, Clone, Educe, Serialize, Deserialize)]
#[educe(Hash)]
pub struct SceneGraph {
    pub marks: Vec<SceneMark>,
    #[educe(Hash(method(hash_floats)))]
    pub width: f32,
    #[educe(Hash(method(hash_floats)))]
    pub height: f32,
    #[educe(Hash(method(hash_floats)))]
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

    /// The URLs of the scene's image resources, each once, in the scene's order: the image
    /// marks' and mesh image instances' images that name their pixels by URL.
    pub fn image_urls(&self) -> Vec<&str> {
        let mut urls = Vec::new();
        collect_image_urls(&self.marks, &mut urls, &mut HashSet::new());
        urls
    }

    /// The scene's image resources as `resolver` holds them now, in the scene's order, without
    /// requesting the ones it doesn't hold. Without a resolver, none is ready.
    pub fn resolve_images(&self, resolver: Option<&dyn ImageResolver>) -> ResolvedImages {
        let mut resolved = ResolvedImages::default();
        for url in self.image_urls() {
            match resolver.and_then(|resolver| resolver.ready(url)) {
                Some(image) => {
                    resolved.ready.insert(url.to_string(), image);
                }
                None => resolved.unready.push(url.to_string()),
            }
        }
        resolved
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

/// A scene's image resources as a resolver answered them.
#[derive(Debug, Clone, Default)]
pub struct ResolvedImages {
    /// The ready images by URL.
    pub ready: HashMap<String, Arc<RgbaImage>>,
    /// The URLs of the images that are pending or failed, which draw their placeholders.
    pub unready: Vec<String>,
}

impl ResolvedImages {
    /// The ready image at `url`.
    pub fn get(&self, url: &str) -> Option<Arc<RgbaImage>> {
        self.ready.get(url).cloned()
    }
}

fn collect_image_urls<'a>(
    marks: &'a [SceneMark],
    urls: &mut Vec<&'a str>,
    seen: &mut HashSet<&'a str>,
) {
    for mark in marks {
        match mark {
            SceneMark::Group(group) => collect_image_urls(&group.marks, urls, seen),
            SceneMark::Image(mark) => match mark.image.value() {
                ScalarOrArrayValue::Scalar(source) => push_url(source.url(), urls, seen),
                ScalarOrArrayValue::Array(sources) => {
                    for index in mark.indices_iter() {
                        let url = sources.get(index).and_then(SceneImageSource::url);
                        push_url(url, urls, seen);
                    }
                }
            },
            SceneMark::MeshImage(mark) => {
                for instance in &mark.instances {
                    push_url(instance.image.url(), urls, seen);
                }
            }
            _ => {}
        }
    }
}

fn push_url<'a>(url: Option<&'a str>, urls: &mut Vec<&'a str>, seen: &mut HashSet<&'a str>) {
    if let Some(url) = url.filter(|url| seen.insert(url)) {
        urls.push(url);
    }
}
