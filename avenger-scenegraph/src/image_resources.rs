use std::sync::Arc;

use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use avenger_image::{ImageResourceResolver, ImageResourceState};

use crate::{
    error::AvengerSceneGraphError,
    marks::{
        image::{SceneImageMark, SceneImageSource},
        mark::SceneMark,
    },
    scene_graph::SceneGraph,
};

/// Return a copy of the scene graph with ready image resources replaced by inline images.
///
/// This utility does not load or wait for images. Hosts that need complete SVG
/// or PDF output can use their own loader/cache, wait until resources are
/// ready, then call this before passing the scene to export renderers.
pub fn resolve_ready_image_resources(
    scene_graph: &SceneGraph,
    resolver: &dyn ImageResourceResolver,
) -> Result<SceneGraph, AvengerSceneGraphError> {
    Ok(SceneGraph {
        marks: resolve_marks(&scene_graph.marks, resolver)?,
        width: scene_graph.width,
        height: scene_graph.height,
        origin: scene_graph.origin,
    })
}

fn resolve_marks(
    marks: &[SceneMark],
    resolver: &dyn ImageResourceResolver,
) -> Result<Vec<SceneMark>, AvengerSceneGraphError> {
    marks
        .iter()
        .map(|mark| resolve_mark(mark, resolver))
        .collect()
}

fn resolve_mark(
    mark: &SceneMark,
    resolver: &dyn ImageResourceResolver,
) -> Result<SceneMark, AvengerSceneGraphError> {
    match mark {
        SceneMark::Image(image) => resolve_image_mark(image, resolver),
        SceneMark::WarpedImage(warped) => {
            let mut resolved = warped.as_ref().clone();
            resolved.image = resolve_image_source(&warped.image, resolver)?;
            Ok(SceneMark::WarpedImage(std::sync::Arc::new(resolved)))
        }
        SceneMark::Group(group) => {
            let mut resolved = group.clone();
            resolved.marks = resolve_marks(&group.marks, resolver)?;
            Ok(SceneMark::Group(resolved))
        }
        _ => Ok(mark.clone()),
    }
}

fn resolve_image_mark(
    image: &Arc<SceneImageMark>,
    resolver: &dyn ImageResourceResolver,
) -> Result<SceneMark, AvengerSceneGraphError> {
    let mut resolved = image.as_ref().clone();
    resolved.image = resolve_image_sources(&image.image, resolver)?;
    Ok(SceneMark::Image(Arc::new(resolved)))
}

fn resolve_image_sources(
    sources: &ScalarOrArray<SceneImageSource>,
    resolver: &dyn ImageResourceResolver,
) -> Result<ScalarOrArray<SceneImageSource>, AvengerSceneGraphError> {
    match sources.value() {
        ScalarOrArrayValue::Scalar(source) => Ok(ScalarOrArray::new_scalar(resolve_image_source(
            source, resolver,
        )?)),
        ScalarOrArrayValue::Array(sources) => {
            let mut resolved = Vec::with_capacity(sources.len());
            for source in sources.iter() {
                resolved.push(resolve_image_source(source, resolver)?);
            }
            Ok(ScalarOrArray::new_array(resolved))
        }
    }
}

fn resolve_image_source(
    source: &SceneImageSource,
    resolver: &dyn ImageResourceResolver,
) -> Result<SceneImageSource, AvengerSceneGraphError> {
    let SceneImageSource::Resource(resource) = source else {
        return Ok(source.clone());
    };

    match resolver.image_state(&resource.key) {
        ImageResourceState::Ready(image) => Ok(SceneImageSource::SharedInline(image)),
        ImageResourceState::Pending => Err(AvengerSceneGraphError::ImageResourcePending(
            resource.key.clone(),
        )),
        ImageResourceState::Missing => Err(AvengerSceneGraphError::ImageResourceMissing(
            resource.key.clone(),
        )),
        ImageResourceState::Failed(error) => Err(AvengerSceneGraphError::ImageResourceFailed(
            resource.key.clone(),
            error.to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use avenger_common::{
        types::{ImageAlign, ImageBaseline},
        value::ScalarOrArray,
    };
    use avenger_image::{ImageResourceState, RgbaImage};
    use avenger_resource::ResourceKey;

    use super::*;
    use crate::marks::image::SceneImageResource;

    struct SingleImageResolver {
        key: ResourceKey,
        state: ImageResourceState,
    }

    impl ImageResourceResolver for SingleImageResolver {
        fn image_state(&self, key: &ResourceKey) -> ImageResourceState {
            if key == &self.key {
                self.state.clone()
            } else {
                ImageResourceState::Missing
            }
        }
    }

    #[test]
    fn ready_resources_are_inlined() {
        let key = ResourceKey::new("tile/0/0/0");
        let scene_graph = resource_scene_graph(key.clone());
        let resolver = SingleImageResolver {
            key,
            state: ImageResourceState::Ready(Arc::new(RgbaImage {
                width: 2,
                height: 2,
                data: [0, 128, 255, 255].repeat(4),
            })),
        };

        let resolved = resolve_ready_image_resources(&scene_graph, &resolver).unwrap();
        let SceneMark::Image(image) = &resolved.marks[0] else {
            panic!("expected image mark");
        };
        let source = image.image.first().expect("image source");
        assert!(source.inline_image().is_some());
    }

    #[test]
    fn pending_resources_error() {
        let key = ResourceKey::new("tile/0/0/0");
        let scene_graph = resource_scene_graph(key.clone());
        let resolver = SingleImageResolver {
            key,
            state: ImageResourceState::Pending,
        };

        assert!(matches!(
            resolve_ready_image_resources(&scene_graph, &resolver),
            Err(AvengerSceneGraphError::ImageResourcePending(_))
        ));
    }

    fn resource_scene_graph(key: ResourceKey) -> SceneGraph {
        SceneGraph {
            width: 8.0,
            height: 8.0,
            origin: [0.0, 0.0],
            marks: vec![SceneImageMark {
                len: 1,
                aspect: false,
                smooth: false,
                image: ScalarOrArray::new_scalar(SceneImageSource::Resource(SceneImageResource {
                    key,
                    intrinsic_width: 2,
                    intrinsic_height: 2,
                    fallback_key: None,
                })),
                x: ScalarOrArray::new_scalar(0.0),
                y: ScalarOrArray::new_scalar(0.0),
                width: ScalarOrArray::new_scalar(8.0),
                height: ScalarOrArray::new_scalar(8.0),
                align: ScalarOrArray::new_scalar(ImageAlign::Left),
                baseline: ScalarOrArray::new_scalar(ImageBaseline::Top),
                ..Default::default()
            }
            .into()],
        }
    }
}
