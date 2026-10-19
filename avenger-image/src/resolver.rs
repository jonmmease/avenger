//! The interface through which renderers find the images that scenes name by URL.

use std::collections::HashMap;
use std::fmt;
use std::hash::BuildHasher;
use std::sync::Arc;

use crate::RgbaImage;

/// What a resolver knows about an image.
#[derive(Debug, Clone)]
pub enum ImageState {
    /// The image's pixels.
    Ready(Arc<RgbaImage>),
    /// The image is requested, and its pixels haven't arrived.
    Pending,
    /// The image can't be loaded now, for the reason given.
    Failed(Arc<str>),
}

impl ImageState {
    /// The image's pixels, when they are ready.
    pub fn ready(&self) -> Option<&Arc<RgbaImage>> {
        match self {
            ImageState::Ready(image) => Some(image),
            ImageState::Pending | ImageState::Failed(_) => None,
        }
    }

    /// Whether two states are the same answer: the same pixels, by identity, both pending, or
    /// failed for the same reason. A renderer installs its scene again when an answer changes,
    /// which also keeps its image status current.
    pub fn draws_like(&self, other: &ImageState) -> bool {
        match (self, other) {
            (ImageState::Ready(a), ImageState::Ready(b)) => Arc::ptr_eq(a, b),
            (ImageState::Pending, ImageState::Pending) => true,
            (ImageState::Failed(a), ImageState::Failed(b)) => a == b,
            _ => false,
        }
    }
}

/// Finds images by URL for the renderers that draw them.
///
/// A resolver answers each request from what it holds, and requests what it doesn't hold. It
/// requests images behind each other in two classes: images that a view draws now, through
/// [`ImageResolver::image`], start before images that it may draw soon, through
/// [`ImageResolver::prefetch`].
pub trait ImageResolver: Send + Sync {
    /// The image at `url`. The resolver requests the image when it holds no answer for it.
    fn image(&self, url: &str) -> ImageState;

    /// Request the image at `url` behind every [`ImageResolver::image`] request, unless the
    /// resolver holds an answer for it.
    fn prefetch(&self, url: &str);

    /// The image at `url` when the resolver holds its pixels. This never requests the image.
    fn ready(&self, url: &str) -> Option<Arc<RgbaImage>>;

    /// Keep the images at `urls` in memory until the lease drops. When the last lease that holds
    /// a URL drops, the resolver drops a request for it that hasn't started.
    fn retain(&self, urls: &[&str]) -> ImageLease;
}

/// Keeps a resolver's images in memory until it drops. See [`ImageResolver::retain`].
#[derive(Default)]
pub struct ImageLease {
    _guard: Option<Box<dyn Send + Sync>>,
}

impl ImageLease {
    /// A lease that runs `guard`'s drop when it ends.
    pub fn new(guard: impl Send + Sync + 'static) -> Self {
        Self {
            _guard: Some(Box::new(guard)),
        }
    }
}

impl fmt::Debug for ImageLease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageLease").finish_non_exhaustive()
    }
}

/// Images held by URL, which answer at once. A URL without an image fails.
impl<S: BuildHasher + Send + Sync> ImageResolver for HashMap<String, Arc<RgbaImage>, S> {
    fn image(&self, url: &str) -> ImageState {
        match self.get(url) {
            Some(image) => ImageState::Ready(image.clone()),
            None => ImageState::Failed(format!("no image for {url}").into()),
        }
    }

    fn prefetch(&self, _url: &str) {}

    fn ready(&self, url: &str) -> Option<Arc<RgbaImage>> {
        self.get(url).cloned()
    }

    fn retain(&self, _urls: &[&str]) -> ImageLease {
        ImageLease::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_map_answers_from_the_images_it_holds() {
        let image = Arc::new(RgbaImage {
            width: 1,
            height: 1,
            data: vec![1, 2, 3, 4],
        });
        let images = HashMap::from([("a.png".to_string(), image.clone())]);
        let resolver: &dyn ImageResolver = &images;
        assert!(
            matches!(resolver.image("a.png"), ImageState::Ready(found) if Arc::ptr_eq(&found, &image))
        );
        assert!(matches!(resolver.image("b.png"), ImageState::Failed(_)));
        assert!(resolver.ready("b.png").is_none());
    }

    #[test]
    fn states_draw_alike_by_pixels_identity() {
        let image = Arc::new(RgbaImage::default());
        let ready = ImageState::Ready(image.clone());
        assert!(ready.draws_like(&ImageState::Ready(image)));
        assert!(!ready.draws_like(&ImageState::Ready(Arc::new(RgbaImage::default()))));
        assert!(ImageState::Pending.draws_like(&ImageState::Pending));
        assert!(!ImageState::Pending.draws_like(&ImageState::Failed("x".into())));
    }
}
