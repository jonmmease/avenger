//! Scenes and image checks that the renderers' tests share: the gallery scene, the parity scenes
//! that check the scene graph's rendering semantics in each renderer, a scene whose images load
//! by URL and a resolver for it, map tile scenes with their seam checks, and SVG rasterization
//! and baseline comparison.

pub mod parity;
pub mod raster;
pub mod resources;
pub mod scene;
pub mod tiles;
