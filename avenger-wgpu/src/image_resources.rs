use std::sync::Arc;

pub use avenger_image::ImageResourceResolver;
use avenger_image::{ImageResourceState, RgbaImage};
use avenger_resource::ResourceKey;
use avenger_scenegraph::marks::image::SceneImageUnavailablePolicy;

use crate::error::AvengerWgpuError;
pub use crate::marks::tile_array::TileUploadStats;

#[derive(Clone)]
pub struct WgpuImageResourceConfig {
    pub resolver: Option<Arc<dyn ImageResourceResolver>>,
    pub missing_policy: WgpuMissingImagePolicy,
    pub placeholder: WgpuImagePlaceholder,
}

impl Default for WgpuImageResourceConfig {
    fn default() -> Self {
        Self {
            resolver: None,
            missing_policy: WgpuMissingImagePolicy::Error,
            placeholder: WgpuImagePlaceholder::Checkerboard,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WgpuMissingImagePolicy {
    DrawPlaceholder,
    Skip,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WgpuImagePlaceholder {
    Transparent,
    Solid([u8; 4]),
    Checkerboard,
    Inline(RgbaImage),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WgpuImageResourceStatus {
    pub pending: Vec<ResourceKey>,
    pub missing: Vec<ResourceKey>,
    pub failed: Vec<(ResourceKey, String)>,
    pub generation: u64,
}

impl WgpuImageResourceStatus {
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty() && self.missing.is_empty() && self.failed.is_empty()
    }

    pub(crate) fn with_generation(mut self, generation: u64) -> Self {
        self.generation = generation;
        self
    }
}

pub(crate) enum ImageSizeRequirement {
    Atlas([u32; 2]),
    Tile(u32),
}

impl ImageSizeRequirement {
    fn dimensions(&self) -> [u32; 2] {
        match *self {
            Self::Atlas(size) => size,
            Self::Tile(size) => [size, size],
        }
    }
}

pub(crate) enum ResolvedImageContent {
    Image(Arc<RgbaImage>),
    Placeholder,
    Empty,
}

/// Resolve availability independently of atlas packing or tile uploads.
/// Atlas primaries may resize. Tile images and all fallbacks require an exact size.
pub(crate) fn resolve_image_resource(
    key: &ResourceKey,
    fallback_key: Option<&ResourceKey>,
    size: ImageSizeRequirement,
    policy: SceneImageUnavailablePolicy,
    config: &WgpuImageResourceConfig,
    status: &mut WgpuImageResourceStatus,
) -> Result<ResolvedImageContent, AvengerWgpuError> {
    let unavailable = |reason: &str| {
        let policy = match policy {
            SceneImageUnavailablePolicy::RendererDefault => config.missing_policy,
            SceneImageUnavailablePolicy::Skip => WgpuMissingImagePolicy::Skip,
            SceneImageUnavailablePolicy::DrawPlaceholder => WgpuMissingImagePolicy::DrawPlaceholder,
            SceneImageUnavailablePolicy::Error => WgpuMissingImagePolicy::Error,
        };
        match policy {
            WgpuMissingImagePolicy::Skip => Ok(ResolvedImageContent::Empty),
            WgpuMissingImagePolicy::DrawPlaceholder => Ok(ResolvedImageContent::Placeholder),
            WgpuMissingImagePolicy::Error => {
                Err(AvengerWgpuError::ImageResourceError(reason.into()))
            }
        }
    };
    let Some(resolver) = &config.resolver else {
        push_unique(&mut status.missing, key.clone());
        return unavailable("No WGPU image resource resolver configured");
    };
    let [width, height] = size.dimensions();
    let reason = match resolver.image_state(key) {
        ImageResourceState::Ready(image) => {
            validate_image_pixels(&image)?;
            if [image.width, image.height] == [width, height] {
                return Ok(ResolvedImageContent::Image(image));
            }
            if matches!(size, ImageSizeRequirement::Atlas(_)) {
                let pixels = image.to_image().expect("validated image pixels");
                let resized = image::imageops::resize(
                    &pixels,
                    width,
                    height,
                    image::imageops::FilterType::CatmullRom,
                );
                return Ok(ResolvedImageContent::Image(Arc::new(
                    RgbaImage::from_image(&resized),
                )));
            }
            let message = format!("Ready resource image {key:?} has dimensions ({}, {}), expected ({width}, {height})", image.width, image.height);
            push_unique_failed(status, key.clone(), message.clone());
            return unavailable(&message);
        }
        ImageResourceState::Pending => {
            push_unique(&mut status.pending, key.clone());
            "pending".to_owned()
        }
        ImageResourceState::Missing => {
            push_unique(&mut status.missing, key.clone());
            "missing".to_owned()
        }
        ImageResourceState::Failed(error) => {
            push_unique_failed(status, key.clone(), error.to_string());
            error.to_string()
        }
    };
    if let Some(fallback_key) = fallback_key {
        if let ImageResourceState::Ready(image) = resolver.image_state(fallback_key) {
            validate_image_pixels(&image)?;
            if [image.width, image.height] == [width, height] {
                return Ok(ResolvedImageContent::Image(image));
            }
            push_unique_failed(status, fallback_key.clone(), format!(
                "Fallback resource image {fallback_key:?} has dimensions ({}, {}), expected ({width}, {height})",
                image.width, image.height,
            ));
        }
    }
    unavailable(&reason)
}

fn validate_image_pixels(image: &RgbaImage) -> Result<(), AvengerWgpuError> {
    image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(
        image.width,
        image.height,
        image.data.as_slice(),
    )
    .map(|_| ())
    .ok_or_else(|| AvengerWgpuError::ConversionError("Invalid RGBA image buffer".into()))
}

pub(crate) fn push_unique(values: &mut Vec<ResourceKey>, key: ResourceKey) {
    if !values.contains(&key) {
        values.push(key);
    }
}

pub(crate) fn push_unique_failed(
    status: &mut WgpuImageResourceStatus,
    key: ResourceKey,
    error: String,
) {
    if !status.failed.iter().any(|(existing, _)| existing == &key) {
        status.failed.push((key, error));
    }
}

impl WgpuImageResourceStatus {
    pub(crate) fn merge(&mut self, other: Self) {
        for key in other.pending {
            push_unique(&mut self.pending, key);
        }
        for key in other.missing {
            push_unique(&mut self.missing, key);
        }
        for (key, error) in other.failed {
            push_unique_failed(self, key, error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Images(HashMap<ResourceKey, ImageResourceState>);
    impl ImageResourceResolver for Images {
        fn image_state(&self, key: &ResourceKey) -> ImageResourceState {
            self.0
                .get(key)
                .cloned()
                .unwrap_or(ImageResourceState::Missing)
        }
    }

    #[test]
    fn resource_resolution_reports_unavailable_states_and_applies_policies() {
        let key = ResourceKey::new("primary");
        for state in [
            ImageResourceState::Pending,
            ImageResourceState::Missing,
            ImageResourceState::Failed(Arc::from("offline")),
        ] {
            let config = WgpuImageResourceConfig {
                resolver: Some(Arc::new(Images(HashMap::from([(
                    key.clone(),
                    state.clone(),
                )])))),
                ..Default::default()
            };
            for policy in [
                SceneImageUnavailablePolicy::Skip,
                SceneImageUnavailablePolicy::DrawPlaceholder,
                SceneImageUnavailablePolicy::RendererDefault,
            ] {
                let mut status = WgpuImageResourceStatus::default();
                let result = resolve_image_resource(
                    &key,
                    None,
                    ImageSizeRequirement::Tile(2),
                    policy,
                    &config,
                    &mut status,
                );
                match policy {
                    SceneImageUnavailablePolicy::Skip => {
                        assert!(matches!(result, Ok(ResolvedImageContent::Empty)))
                    }
                    SceneImageUnavailablePolicy::DrawPlaceholder => {
                        assert!(matches!(result, Ok(ResolvedImageContent::Placeholder)))
                    }
                    _ => assert!(matches!(
                        result,
                        Err(AvengerWgpuError::ImageResourceError(_))
                    )),
                }
                match &state {
                    ImageResourceState::Pending => assert_eq!(status.pending, vec![key.clone()]),
                    ImageResourceState::Missing => assert_eq!(status.missing, vec![key.clone()]),
                    ImageResourceState::Failed(_) => {
                        assert_eq!(status.failed, vec![(key.clone(), "offline".into())])
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    #[test]
    fn resource_resolution_applies_storage_and_fallback_size_requirements() {
        let key = ResourceKey::new("primary");
        let fallback = ResourceKey::new("fallback");
        let pixels = Arc::new(RgbaImage {
            width: 1,
            height: 1,
            data: vec![255, 0, 0, 255],
        });
        for primary_ready in [true, false] {
            let states = if primary_ready {
                HashMap::from([(key.clone(), ImageResourceState::Ready(pixels.clone()))])
            } else {
                HashMap::from([(fallback.clone(), ImageResourceState::Ready(pixels.clone()))])
            };
            let config = WgpuImageResourceConfig {
                resolver: Some(Arc::new(Images(states))),
                ..Default::default()
            };
            for size in [
                ImageSizeRequirement::Atlas([2, 2]),
                ImageSizeRequirement::Tile(2),
            ] {
                let resizes = primary_ready && matches!(size, ImageSizeRequirement::Atlas(_));
                let mut status = WgpuImageResourceStatus::default();
                let result = resolve_image_resource(
                    &key,
                    Some(&fallback),
                    size,
                    SceneImageUnavailablePolicy::Skip,
                    &config,
                    &mut status,
                )
                .unwrap();
                if resizes {
                    let ResolvedImageContent::Image(image) = result else {
                        panic!("expected resized primary");
                    };
                    assert_eq!((image.width, image.height), (2, 2));
                    assert_eq!(image.data, [255, 0, 0, 255].repeat(4));
                } else {
                    assert!(matches!(result, ResolvedImageContent::Empty));
                    assert_eq!(
                        status.failed[0].0,
                        if primary_ready { &key } else { &fallback }.clone()
                    );
                }
            }
            let mut status = WgpuImageResourceStatus::default();
            assert!(matches!(
                resolve_image_resource(
                    &key,
                    Some(&fallback),
                    ImageSizeRequirement::Tile(1),
                    SceneImageUnavailablePolicy::Error,
                    &config,
                    &mut status
                ),
                Ok(ResolvedImageContent::Image(_))
            ));
        }
    }
}
