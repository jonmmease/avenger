pub mod canvas;
pub mod context;
pub mod error;
pub mod marks;
pub mod offscreen;
pub(crate) mod readback;
pub mod renderer;
pub mod target;
pub mod util;

#[cfg(target_arch = "wasm32")]
pub mod html_canvas;

/// The wgpu that Avenger renders with. A host that embeds Avenger uses this wgpu, so its device,
/// textures, and encoders are the types Avenger takes.
pub use wgpu;
