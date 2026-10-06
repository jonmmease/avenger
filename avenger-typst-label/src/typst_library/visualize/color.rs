//! Colors.
//!
//! avenger: Typst's color model (`crates/typst-library/src/visualize/color.rs`) is not ported.
//! Colors are `avenger_color::AbsoluteColor`, with CSS semantics (D22), and [`ColorExt`]
//! supplies the parts of upstream's `Color` API that ported code calls.

use ecow::{EcoString, eco_format};

use crate::typst_library::diag::{At, SourceResult, bail};
use crate::typst_library::foundations::{Args, Repr, Str, cast, func, ty};
use crate::typst_library::layout::Ratio;
use crate::typst_syntax::Spanned;

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

func! {
/// Create a grayscale color.
///
/// A grayscale color is represented internally by a single `lightness`
/// component.
///
/// These components are also available using the
/// @color.components[`components`] method.
///
/// ```example
/// #for x in range(250, step: 50) {
///   box(square(fill: luma(x)))
/// }
/// ```
// avenger: Typst's arguments; the gray is an sRGB gray, as upstream's sRGB-encoded luma, and a
// color converts to the gray of its relative luminance.
#[func]
pub fn luma(
    args: &mut Args,
    /// The lightness component.
    #[external]
    lightness: Component,
    /// The alpha component.
    #[external]
    alpha: RatioComponent,
    /// Alternatively: The color to convert to grayscale.
    ///
    /// If this is given, the `lightness` should not be given.
    #[external]
    color: Color,
) -> SourceResult<Color> {
    Ok(if let Some(color) = args.find::<Color>()? {
        let [r, g, b, a] = color.to_rgba();
        let gray = encode_srgb(avenger_color::relative_luminance_srgb(r, g, b));
        Color::from_srgb(gray, gray, gray, a)
    } else {
        let Component(gray) =
            args.expect("gray component").unwrap_or(Component(Ratio::one()));
        let RatioComponent(alpha) = args.eat()?.unwrap_or(RatioComponent(Ratio::one()));
        let gray = gray.get() as f32;
        Color::from_srgb(gray, gray, gray, alpha.get() as f32)
    })
}
}

func! {
/// Create an RGB(A) color.
///
/// The color is specified in the sRGB color space.
///
/// An RGB(A) color is represented internally by an array of four components:
/// - red (@ratio)
/// - green (@ratio)
/// - blue (@ratio)
/// - alpha (@ratio)
///
/// These components are also available using the
/// @color.components[`components`] method.
///
/// ```example
/// #square(fill: rgb("#b1f2eb"))
/// #square(fill: rgb(87, 127, 230))
/// #square(fill: rgb(25%, 13%, 65%))
/// ```
// avenger: a string is any CSS color, parsed by `avenger-color` (D22), or hexadecimal digits
// without the hash, as Typst also accepts.
#[func(title = "RGB")]
pub fn rgb(
    args: &mut Args,
    /// The red component.
    #[external]
    red: Component,
    /// The green component.
    #[external]
    green: Component,
    /// The blue component.
    #[external]
    blue: Component,
    /// The alpha component.
    #[external]
    alpha: Component,
    /// Alternatively: The color as a CSS color string, such as `{"#239dad"}`,
    /// `{"tomato"}` or `{"hsl(120, 100%, 50%)"}`.
    ///
    /// If this is given, the individual components should not be given.
    #[external]
    hex: Str,
    /// Alternatively: The color to convert to RGB(a).
    ///
    /// If this is given, the individual components should not be given.
    #[external]
    color: Color,
) -> SourceResult<Color> {
    Ok(if let Some(string) = args.find::<Spanned<Str>>()? {
        let bare_hex = matches!(string.v.len(), 3 | 4 | 6 | 8)
            && string.v.bytes().all(|byte| byte.is_ascii_hexdigit());
        let rgba = avenger_color::parse_color_string_strict(&string.v)
            .or_else(|err| {
                if bare_hex {
                    avenger_color::parse_color_string_strict(&format!("#{}", string.v))
                } else {
                    Err(err)
                }
            })
            .map_err(|err| eco_format!("{err}"))
            .at(string.span)?;
        Color::from_rgba(rgba)
    } else if let Some(color) = args.find::<Color>()? {
        Color::from_rgba(color.to_rgba())
    } else {
        let Component(r) = args.expect("red component")?;
        let Component(g) = args.expect("green component")?;
        let Component(b) = args.expect("blue component")?;
        let Component(a) = args.eat()?.unwrap_or(Component(Ratio::one()));
        Color::from_srgb(r.get() as f32, g.get() as f32, b.get() as f32, a.get() as f32)
    })
}
}

/// The sRGB encoding of a linear light value.
fn encode_srgb(linear: f32) -> f32 {
    if linear <= 0.0031308 {
        12.92 * linear
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    }
}

/// An integer or ratio component.
pub struct Component(Ratio);

cast! {
    Component,
    v: i64 => match v {
        0 ..= 255 => Self(Ratio::new(v as f64 / 255.0)),
        _ => bail!("number must be between 0 and 255"),
    },
    v: Ratio => if (0.0 ..= 1.0).contains(&v.get()) {
        Self(v)
    } else {
        bail!("ratio must be between 0% and 100%");
    },
}

/// A component that must be a ratio.
pub struct RatioComponent(Ratio);

cast! {
    RatioComponent,
    v: Ratio => if (0.0 ..= 1.0).contains(&v.get()) {
        Self(v)
    } else {
        bail!("ratio must be between 0% and 100%");
    },
}
