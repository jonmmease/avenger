//! A scene whose images load by URL, and a resolver whose answers a test sets, for the tests of
//! image resolution.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use avenger_image::{ImageLease, ImageResolver, ImageState, RgbaImage};
use avenger_scenegraph::{
    marks::{
        image::{SceneImageMark, SceneImageResource, SceneImageSource},
        mesh_image::{MeshImage, SceneMeshImageMark},
    },
    scene_graph::SceneGraph,
};

/// The URL of the scene's image mark.
pub const IMAGE_URL: &str = "https://example.com/image.png";
/// The URL of the scene's mesh image instance.
pub const MESH_URL: &str = "https://example.com/mesh.png";
/// The placeholders' color.
pub const PLACEHOLDER: [u8; 4] = [128, 128, 128, 255];

/// An image of one pixel.
pub fn pixel(color: [u8; 4]) -> Arc<RgbaImage> {
    Arc::new(RgbaImage {
        width: 1,
        height: 1,
        data: color.to_vec(),
    })
}

fn resource(url: &str) -> SceneImageSource {
    SceneImageResource {
        url: url.to_string(),
        width: 1,
        height: 1,
        placeholder: Some(pixel(PLACEHOLDER)),
    }
    .into()
}

/// A 20×10 scene with two images that load by URL, each with a gray placeholder: an image mark
/// over the left half, at [`IMAGE_URL`], and a mesh image instance over the right half, at
/// [`MESH_URL`]. The left half's center is (5, 5), and the right half's is (15, 5).
pub fn resource_scene() -> SceneGraph {
    SceneGraph {
        width: 20.0,
        height: 10.0,
        origin: [0.0; 2],
        marks: vec![
            SceneImageMark {
                image: resource(IMAGE_URL).into(),
                width: 10.0.into(),
                height: 10.0.into(),
                aspect: false,
                smooth: false,
                ..Default::default()
            }
            .into(),
            SceneMeshImageMark {
                smooth: false,
                instances: vec![MeshImage {
                    image: resource(MESH_URL),
                    positions: vec![[10.0, 0.0], [20.0, 0.0], [20.0, 10.0], [10.0, 10.0]],
                    uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                    indices: vec![0, 1, 2, 0, 2, 3],
                }],
                ..Default::default()
            }
            .into(),
        ],
    }
}

/// A resolver whose answers a test sets. It records the URLs that it's asked for and asked to
/// prefetch, counts the leases that hold each URL, and records the URLs whose last lease ended.
/// A URL without an answer is pending.
#[derive(Default)]
pub struct TestResolver {
    answers: Mutex<HashMap<String, ImageState>>,
    requested: Mutex<Vec<String>>,
    prefetched: Mutex<Vec<String>>,
    leases: Arc<Mutex<Leases>>,
}

#[derive(Default)]
struct Leases {
    held: HashMap<String, usize>,
    released: Vec<String>,
}

impl TestResolver {
    /// Answer `url` with `state`.
    pub fn set(&self, url: &str, state: ImageState) {
        lock(&self.answers).insert(url.to_string(), state);
    }

    /// Drop the answer for `url`, and the resolver's pixels with it.
    pub fn forget(&self, url: &str) {
        lock(&self.answers).remove(url);
    }

    /// The URLs that [`ImageResolver::image`] was asked for, in order.
    pub fn requested(&self) -> Vec<String> {
        lock(&self.requested).clone()
    }

    /// The URLs that [`ImageResolver::prefetch`] was asked for, in order.
    pub fn prefetched(&self) -> Vec<String> {
        lock(&self.prefetched).clone()
    }

    /// The number of leases that hold `url`.
    pub fn held(&self, url: &str) -> usize {
        lock(&self.leases).held.get(url).copied().unwrap_or(0)
    }

    /// The URLs whose last lease ended, in order.
    pub fn released(&self) -> Vec<String> {
        lock(&self.leases).released.clone()
    }
}

impl ImageResolver for TestResolver {
    fn image(&self, url: &str) -> ImageState {
        lock(&self.requested).push(url.to_string());
        lock(&self.answers)
            .get(url)
            .cloned()
            .unwrap_or(ImageState::Pending)
    }

    fn prefetch(&self, url: &str) {
        lock(&self.prefetched).push(url.to_string());
    }

    fn ready(&self, url: &str) -> Option<Arc<RgbaImage>> {
        lock(&self.answers).get(url)?.ready().cloned()
    }

    fn retain(&self, urls: &[&str]) -> ImageLease {
        struct Release(Arc<Mutex<Leases>>, Vec<String>);
        impl Drop for Release {
            fn drop(&mut self) {
                let mut leases = lock(&self.0);
                for url in &self.1 {
                    let held = leases.held.get_mut(url).unwrap();
                    *held -= 1;
                    if *held == 0 {
                        leases.released.push(url.clone());
                    }
                }
            }
        }
        let urls: Vec<String> = urls.iter().map(|url| url.to_string()).collect();
        let mut leases = lock(&self.leases);
        for url in &urls {
            *leases.held.entry(url.clone()).or_default() += 1;
        }
        ImageLease::new(Release(self.leases.clone(), urls))
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap()
}
