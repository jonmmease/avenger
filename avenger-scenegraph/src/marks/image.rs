use std::sync::Arc;

use avenger_common::types::{ImageAlign, ImageBaseline};
use avenger_common::value::{hash_values, ScalarOrArray};
use avenger_image::RgbaImage;
use educe::Educe;
use itertools::izip;
use lyon_path::Path;
use serde::{Deserialize, Serialize};

use super::mark::SceneMark;

/// Where an image's pixels come from: the scene, or a URL that renderers resolve as they draw.
#[derive(Debug, Clone, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SceneImageSource {
    /// Pixels that the scene holds.
    Inline(Arc<RgbaImage>),
    /// An image that a renderer's image resolver loads by URL.
    Resource(SceneImageResource),
}

/// An image named by URL. Its declared size places it before its pixels arrive, and the pixels
/// that arrive draw across that place, whatever their size.
#[derive(Debug, Clone, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SceneImageResource {
    /// The image's URL, which identifies it to the resolver.
    pub url: String,
    /// The image's width in pixels.
    pub width: u32,
    /// The image's height in pixels.
    pub height: u32,
    /// The image that draws until the resource's pixels are ready, or after they fail to load.
    /// `None` draws nothing.
    pub placeholder: Option<Arc<RgbaImage>>,
}

impl SceneImageSource {
    /// The image's size in pixels, `[width, height]`: an inline image's own size, or a resource's
    /// declared size.
    pub fn size(&self) -> [u32; 2] {
        match self {
            SceneImageSource::Inline(image) => [image.width, image.height],
            SceneImageSource::Resource(resource) => [resource.width, resource.height],
        }
    }

    /// The resource's URL, or `None` for an inline image.
    pub fn url(&self) -> Option<&str> {
        match self {
            SceneImageSource::Inline(_) => None,
            SceneImageSource::Resource(resource) => Some(&resource.url),
        }
    }

    /// The pixels that draw for the source: an inline image, a resource's image when `resolved`
    /// finds it by URL, or else the resource's placeholder.
    pub fn pixels(
        &self,
        resolved: &dyn Fn(&str) -> Option<Arc<RgbaImage>>,
    ) -> Option<Arc<RgbaImage>> {
        match self {
            SceneImageSource::Inline(image) => Some(image.clone()),
            SceneImageSource::Resource(resource) => {
                resolved(&resource.url).or_else(|| resource.placeholder.clone())
            }
        }
    }
}

impl Default for SceneImageSource {
    fn default() -> Self {
        SceneImageSource::Inline(Arc::new(RgbaImage::default()))
    }
}

impl From<RgbaImage> for SceneImageSource {
    fn from(image: RgbaImage) -> Self {
        SceneImageSource::Inline(Arc::new(image))
    }
}

impl From<Arc<RgbaImage>> for SceneImageSource {
    fn from(image: Arc<RgbaImage>) -> Self {
        SceneImageSource::Inline(image)
    }
}

impl From<SceneImageResource> for SceneImageSource {
    fn from(resource: SceneImageResource) -> Self {
        SceneImageSource::Resource(resource)
    }
}

#[derive(Debug, Clone, PartialEq, Educe, Serialize, Deserialize)]
#[educe(Hash)]
#[serde(rename_all = "kebab-case")]
pub struct SceneImageMark {
    pub name: String,
    /// Whether hits find the mark. `None` takes the enclosing group's setting, and marks at the
    /// scene root are interactive.
    pub interactive: Option<bool>,
    pub clip: bool,
    pub len: u32,
    pub aspect: bool,
    pub smooth: bool,
    /// Each image's source. The image's size, inline or declared, sets its aspect.
    #[educe(Hash(method(hash_values)))]
    pub image: ScalarOrArray<SceneImageSource>,
    pub x: ScalarOrArray<f32>,
    pub y: ScalarOrArray<f32>,
    pub width: ScalarOrArray<f32>,
    pub height: ScalarOrArray<f32>,
    pub align: ScalarOrArray<ImageAlign>,
    pub baseline: ScalarOrArray<ImageBaseline>,
    pub indices: Option<Arc<Vec<usize>>>,
    pub zindex: Option<i32>,
}

impl SceneImageMark {
    pub fn image_iter(&self) -> Box<dyn Iterator<Item = &SceneImageSource> + '_> {
        self.image.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn x_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.x.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn y_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.y.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn width_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.width.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn height_iter(&self) -> Box<dyn Iterator<Item = &f32> + '_> {
        self.height
            .as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn align_iter(&self) -> Box<dyn Iterator<Item = &ImageAlign> + '_> {
        self.align.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn baseline_iter(&self) -> Box<dyn Iterator<Item = &ImageBaseline> + '_> {
        self.baseline
            .as_iter(self.len as usize, self.indices.as_ref())
    }

    pub fn indices_iter(&self) -> Box<dyn Iterator<Item = usize> + '_> {
        if let Some(indices) = self.indices.as_ref() {
            Box::new(indices.iter().cloned())
        } else {
            Box::new(0..self.len as usize)
        }
    }

    pub fn transformed_path_iter(&self, origin: [f32; 2]) -> Box<dyn Iterator<Item = Path> + '_> {
        Box::new(
            izip!(
                self.image_iter(),
                self.x_iter(),
                self.y_iter(),
                self.width_iter(),
                self.height_iter(),
                self.baseline_iter(),
                self.align_iter(),
            )
            .map(move |(img, x, y, width, height, baseline, align)| {
                let x = *x + origin[0];
                let y = *y + origin[1];

                // Compute image left
                let left = match *align {
                    ImageAlign::Left => x,
                    ImageAlign::Center => x - *width / 2.0,
                    ImageAlign::Right => x - *width,
                };

                // Compute image top
                let top = match *baseline {
                    ImageBaseline::Top => y,
                    ImageBaseline::Middle => y - *height / 2.0,
                    ImageBaseline::Bottom => y - *height,
                };

                // Adjust position and dimensions if aspect ratio should be preserved
                let (left, top, width, height) = if self.aspect {
                    let [img_width, img_height] = img.size();
                    let img_aspect = img_width as f32 / img_height as f32;
                    let outline_aspect = *width / *height;
                    if img_aspect > outline_aspect {
                        // image is wider than the box, so we scale
                        // image to box width and center vertically
                        let aspect_height = *width / img_aspect;
                        let aspect_top = top + (*height - aspect_height) / 2.0;
                        (left, aspect_top, *width, aspect_height)
                    } else if img_aspect < outline_aspect {
                        // image is taller than the box, so we scale
                        // image to box height an center horizontally
                        let aspect_width = *height * img_aspect;
                        let aspect_left = left + (*width - aspect_width) / 2.0;
                        (aspect_left, top, aspect_width, *height)
                    } else {
                        (left, top, *width, *height)
                    }
                } else {
                    (left, top, *width, *height)
                };

                // Create rect path
                let mut path_builder = Path::builder();
                path_builder.begin(lyon_path::math::point(left, top));
                path_builder.line_to(lyon_path::math::point(left + width, top));
                path_builder.line_to(lyon_path::math::point(left + width, top + height));
                path_builder.line_to(lyon_path::math::point(left, top + height));
                path_builder.close();
                path_builder.build()
            }),
        )
    }
}

impl Default for SceneImageMark {
    fn default() -> Self {
        Self {
            interactive: None,
            name: "image_mark".to_string(),
            clip: true,
            len: 1,
            aspect: true,
            indices: None,
            smooth: true,
            x: ScalarOrArray::new_scalar(0.0),
            y: ScalarOrArray::new_scalar(0.0),
            width: ScalarOrArray::new_scalar(0.0),
            height: ScalarOrArray::new_scalar(0.0),
            align: ScalarOrArray::new_scalar(Default::default()),
            baseline: ScalarOrArray::new_scalar(Default::default()),
            image: ScalarOrArray::new_scalar(Default::default()),
            zindex: None,
        }
    }
}

impl From<SceneImageMark> for SceneMark {
    fn from(mark: SceneImageMark) -> Self {
        SceneMark::Image(Arc::new(mark))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lyon_algorithms::aabb::bounding_box;

    fn resource() -> SceneImageSource {
        SceneImageResource {
            url: "https://example.com/a.png".to_string(),
            width: 40,
            height: 10,
            placeholder: Some(Arc::new(RgbaImage {
                width: 1,
                height: 1,
                data: vec![9, 9, 9, 255],
            })),
        }
        .into()
    }

    #[test]
    fn sources_round_trip_through_serde() {
        let inline: SceneImageSource = RgbaImage {
            width: 1,
            height: 1,
            data: vec![1, 2, 3, 4],
        }
        .into();
        for source in [inline, resource()] {
            let json = serde_json::to_string(&source).unwrap();
            let parsed: SceneImageSource = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, source);
        }
    }

    /// A resource keeps its aspect by its declared size, before its pixels arrive.
    #[test]
    fn a_resource_places_its_quad_by_its_declared_size() {
        let mark = SceneImageMark {
            image: resource().into(),
            width: 20.0.into(),
            height: 20.0.into(),
            ..Default::default()
        };
        let path = mark.transformed_path_iter([0.0, 0.0]).next().unwrap();
        let bounds = bounding_box(&path);
        assert_eq!(
            [bounds.min.x, bounds.min.y, bounds.max.x, bounds.max.y],
            [0.0, 7.5, 20.0, 12.5]
        );
    }

    #[test]
    fn a_resource_draws_what_the_lookup_finds_or_its_placeholder() {
        let found = Arc::new(RgbaImage::default());
        let source = resource();
        let pixels = source.pixels(&|_| Some(found.clone())).unwrap();
        assert!(Arc::ptr_eq(&pixels, &found));
        assert_eq!(source.pixels(&|_| None).unwrap().data, [9, 9, 9, 255]);
        let SceneImageSource::Resource(mut bare) = source else {
            unreachable!()
        };
        bare.placeholder = None;
        assert!(SceneImageSource::from(bare).pixels(&|_| None).is_none());
    }

    /// A scene lists each resource URL once, in its marks' order, through nested groups and
    /// mesh image instances, and resolves them from what a resolver holds, requesting none.
    #[test]
    fn a_scene_lists_and_resolves_its_resource_urls() {
        use std::collections::HashMap;

        use crate::marks::group::SceneGroup;
        use crate::marks::mesh_image::{MeshImage, SceneMeshImageMark};
        use crate::scene_graph::SceneGraph;
        let url = |name: &str| -> SceneImageSource {
            SceneImageResource {
                url: name.to_string(),
                width: 1,
                height: 1,
                placeholder: None,
            }
            .into()
        };
        let images = SceneImageMark {
            len: 3,
            image: ScalarOrArray::new_array(vec![url("b"), url("a"), RgbaImage::default().into()]),
            ..Default::default()
        };
        let mesh = SceneMeshImageMark {
            instances: vec![MeshImage {
                image: url("c"),
                positions: Vec::new(),
                uvs: Vec::new(),
                indices: Vec::new(),
            }],
            ..Default::default()
        };
        let scene = SceneGraph {
            marks: vec![
                images.clone().into(),
                SceneGroup {
                    marks: vec![mesh.into(), images.into()],
                    ..Default::default()
                }
                .into(),
            ],
            width: 10.0,
            height: 10.0,
            origin: [0.0; 2],
        };
        assert_eq!(scene.image_urls(), ["b", "a", "c"]);

        /// Holds images, and fails a test that requests one.
        struct Held(HashMap<String, Arc<RgbaImage>>);
        impl avenger_image::ImageResolver for Held {
            fn image(&self, url: &str) -> avenger_image::ImageState {
                panic!("requested {url}")
            }
            fn prefetch(&self, url: &str) {
                panic!("prefetched {url}")
            }
            fn ready(&self, url: &str) -> Option<Arc<RgbaImage>> {
                self.0.get(url).cloned()
            }
            fn retain(&self, _urls: &[&str]) -> avenger_image::ImageLease {
                avenger_image::ImageLease::default()
            }
        }
        let held = Held(HashMap::from([(
            "a".to_string(),
            Arc::new(RgbaImage::default()),
        )]));
        let resolved = scene.resolve_images(Some(&held));
        assert_eq!(resolved.ready.keys().collect::<Vec<_>>(), ["a"]);
        assert_eq!(resolved.unready, ["b", "c"]);
        assert_eq!(scene.resolve_images(None).unready, ["b", "a", "c"]);
    }
}
