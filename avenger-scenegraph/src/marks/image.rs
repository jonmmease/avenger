use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

use avenger_common::{
    types::{ImageAlign, ImageBaseline},
    value::ScalarOrArray,
};
use avenger_image::RgbaImage;
use avenger_resource::ResourceKey;
use itertools::izip;
use lyon_path::Path;
use serde::{Deserialize, Serialize};

use super::mark::{default_interactive, SceneMark};

#[derive(Debug, Clone, PartialEq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SceneImageSource {
    Inline(RgbaImage),
    SharedInline(Arc<RgbaImage>),
    Resource(SceneImageResource),
}

impl SceneImageSource {
    pub fn inline(image: RgbaImage) -> Self {
        Self::Inline(image)
    }

    pub fn shared_inline(image: Arc<RgbaImage>) -> Self {
        Self::SharedInline(image)
    }

    pub fn intrinsic_size(&self) -> [u32; 2] {
        match self {
            Self::Inline(image) => [image.width, image.height],
            Self::SharedInline(image) => [image.width, image.height],
            Self::Resource(resource) => [resource.intrinsic_width, resource.intrinsic_height],
        }
    }

    pub fn resource_key(&self) -> Option<&ResourceKey> {
        match self {
            Self::Inline(_) | Self::SharedInline(_) => None,
            Self::Resource(resource) => Some(&resource.key),
        }
    }

    pub fn inline_image(&self) -> Option<&RgbaImage> {
        match self {
            Self::Inline(image) => Some(image),
            Self::SharedInline(image) => Some(image.as_ref()),
            Self::Resource(_) => None,
        }
    }
}

impl Default for SceneImageSource {
    fn default() -> Self {
        Self::Inline(RgbaImage::default())
    }
}

#[derive(Debug, Clone, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SceneImageResource {
    pub key: ResourceKey,
    pub intrinsic_width: u32,
    pub intrinsic_height: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_key: Option<ResourceKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SceneImageUnavailablePolicy {
    #[default]
    RendererDefault,
    Skip,
    DrawPlaceholder,
    Error,
}

fn is_default_unavailable_policy(policy: &SceneImageUnavailablePolicy) -> bool {
    *policy == SceneImageUnavailablePolicy::RendererDefault
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SceneImageMark {
    pub name: String,
    #[serde(default = "default_interactive")]
    pub interactive: bool,
    pub clip: bool,
    pub len: u32,
    pub aspect: bool,
    pub smooth: bool,
    pub image: ScalarOrArray<SceneImageSource>,
    pub x: ScalarOrArray<f32>,
    pub y: ScalarOrArray<f32>,
    pub width: ScalarOrArray<f32>,
    pub height: ScalarOrArray<f32>,
    pub align: ScalarOrArray<ImageAlign>,
    pub baseline: ScalarOrArray<ImageBaseline>,
    #[serde(default, skip_serializing_if = "is_default_unavailable_policy")]
    pub unavailable_policy: SceneImageUnavailablePolicy,
    pub indices: Option<Arc<Vec<usize>>>,
    pub zindex: Option<i32>,
    /// `Some(edge_px)` routes the mark's resource images through the
    /// renderer's persistent tile texture-array cache (uniform-size map
    /// tiles that upload once and survive scene rebuilds) instead of the
    /// per-frame image atlas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tile_texture_size: Option<u32>,
}

impl SceneImageMark {
    pub fn image_source_iter(&self) -> Box<dyn Iterator<Item = &SceneImageSource> + '_> {
        self.image.as_iter(self.len as usize, self.indices.as_ref())
    }
    pub fn image_iter(&self) -> Box<dyn Iterator<Item = &SceneImageSource> + '_> {
        self.image_source_iter()
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
                self.image_source_iter(),
                self.x_iter(),
                self.y_iter(),
                self.width_iter(),
                self.height_iter(),
                self.baseline_iter(),
                self.align_iter(),
            )
            .map(
                move |(image_source, x, y, width, height, baseline, align)| {
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
                        let [intrinsic_width, intrinsic_height] = image_source.intrinsic_size();
                        let img_aspect = intrinsic_width as f32 / intrinsic_height as f32;
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
                },
            ),
        )
    }
}

impl Hash for SceneImageMark {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.interactive.hash(state);
        self.clip.hash(state);
        self.len.hash(state);
        self.aspect.hash(state);
        self.smooth.hash(state);
        for image in self.image_source_iter() {
            image.hash(state);
        }
        self.x.hash(state);
        self.y.hash(state);
        self.width.hash(state);
        self.height.hash(state);
        self.align.hash(state);
        self.baseline.hash(state);
        self.unavailable_policy.hash(state);
        self.indices.hash(state);
        self.zindex.hash(state);
        self.tile_texture_size.hash(state);
    }
}

impl Default for SceneImageMark {
    fn default() -> Self {
        Self {
            name: "image_mark".to_string(),
            interactive: true,
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
            unavailable_policy: SceneImageUnavailablePolicy::RendererDefault,
            image: ScalarOrArray::new_scalar(Default::default()),
            zindex: None,
            tile_texture_size: None,
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
    use avenger_common::{
        types::{ImageAlign, ImageBaseline},
        value::ScalarOrArray,
    };
    use avenger_resource::ResourceKey;
    use lyon_algorithms::aabb::bounding_box;
    use serde_json::json;

    use super::*;

    #[test]
    fn old_inline_image_json_deserializes_as_inline_source() {
        let mark: SceneImageMark = serde_json::from_value(json!({
            "name": "image",
            "interactive": true,
            "clip": true,
            "len": 1,
            "aspect": false,
            "smooth": true,
            "image": {
                "value": {
                    "scalar": {
                        "width": 1,
                        "height": 1,
                        "data": [255, 0, 0, 255]
                    }
                }
            },
            "x": { "value": { "scalar": 0.0 } },
            "y": { "value": { "scalar": 0.0 } },
            "width": { "value": { "scalar": 10.0 } },
            "height": { "value": { "scalar": 10.0 } },
            "align": { "value": { "scalar": "left" } },
            "baseline": { "value": { "scalar": "top" } }
        }))
        .unwrap();

        let source = mark.image_source_iter().next().unwrap();
        assert!(matches!(source, SceneImageSource::Inline(_)));
        assert_eq!(source.intrinsic_size(), [1, 1]);
    }

    #[test]
    fn old_inline_image_json_without_source_tag_deserializes_as_inline_source() {
        let source: SceneImageSource = serde_json::from_value(json!({
                    "width": 1,
                    "height": 1,
                    "data": [255, 0, 0, 255]
        }))
        .unwrap();

        assert!(matches!(source, SceneImageSource::Inline(_)));
        assert_eq!(source.intrinsic_size(), [1, 1]);
    }

    #[test]
    fn resource_image_intrinsic_size_preserves_aspect_geometry() {
        let mark = SceneImageMark {
            len: 1,
            aspect: true,
            image: ScalarOrArray::new_scalar(SceneImageSource::Resource(SceneImageResource {
                key: ResourceKey::new("tile/0/0/0"),
                intrinsic_width: 4,
                intrinsic_height: 2,
                fallback_key: None,
            })),
            x: ScalarOrArray::new_scalar(0.0),
            y: ScalarOrArray::new_scalar(0.0),
            width: ScalarOrArray::new_scalar(10.0),
            height: ScalarOrArray::new_scalar(10.0),
            align: ScalarOrArray::new_scalar(ImageAlign::Left),
            baseline: ScalarOrArray::new_scalar(ImageBaseline::Top),
            ..Default::default()
        };

        let path = mark.transformed_path_iter([0.0, 0.0]).next().unwrap();
        let bbox = bounding_box(&path);

        assert_eq!(bbox.min.x, 0.0);
        assert_eq!(bbox.max.x, 10.0);
        assert_eq!(bbox.min.y, 2.5);
        assert_eq!(bbox.max.y, 7.5);
    }
}
