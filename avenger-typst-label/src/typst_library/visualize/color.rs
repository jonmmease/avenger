//! Colors.
//!
//! avenger: Typst's color model (`crates/typst-library/src/visualize/color.rs`) is not ported.
//! Colors are `avenger_color::AbsoluteColor`, with CSS semantics (D22), and [`ColorExt`]
//! supplies the parts of upstream's `Color` API that ported code calls.

use ecow::{EcoString, eco_format};

use crate::typst_library::foundations::{Repr, ty};

pub use avenger_color::AbsoluteColor as Color;

ty!(Color, name = "color", title = "Color", long = "color");

/// Upstream's `Color` constants and constructors that ported code calls.
pub trait ColorExt {
    /// Opaque black.
    const BLACK: Self;
    /// Opaque white.
    const WHITE: Self;

    /// Construct a new RGBA color from 8-bit values.
    fn from_u8(r: u8, g: u8, b: u8, a: u8) -> Self;
}

impl ColorExt for Color {
    const BLACK: Self = srgb(0.0, 0.0, 0.0);
    const WHITE: Self = srgb(1.0, 1.0, 1.0);

    fn from_u8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self::from_rgba8([r, g, b, a])
    }
}

/// An opaque sRGB color, in a const context.
const fn srgb(r: f32, g: f32, b: f32) -> Color {
    Color {
        components: [r, g, b],
        alpha: 1.0,
        color_space: avenger_color::ColorSpace::Srgb,
    }
}

impl Repr for Color {
    // avenger: every color displays as upstream displays an RGB color, `rgb("#rrggbb")`, with
    // an alpha byte when it isn't opaque.
    fn repr(&self) -> EcoString {
        let [r, g, b, a] = self.to_rgba().map(|c| (c * 255.0).round() as u8);
        let hex = if a != 255 {
            eco_format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
        } else {
            eco_format!("#{r:02x}{g:02x}{b:02x}")
        };
        eco_format!("rgb({})", hex.repr())
    }
}
