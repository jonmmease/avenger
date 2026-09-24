//! Core color types for color space support

use crate::convert;

/// A color space representation in the CSS specification
///
/// https://drafts.csswg.org/css-color-4/#typedef-color-space
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum ColorSpace {
    /// sRGB color space (rgb, rgba)
    Srgb = 0,
    /// HSL (Hue-Saturation-Lightness) in sRGB
    Hsl,
    /// HWB (Hue-Whiteness-Blackness) in sRGB
    Hwb,
    /// CIE L*a*b* with the CSS D50 white point
    Lab,
    /// LCH (Lightness-Chroma-Hue) - cylindrical D50 Lab
    Lch,
    /// Oklab color space (improved Lab)
    Oklab,
    /// Oklch (Lightness-Chroma-Hue) - cylindrical Oklab
    Oklch,
}

impl ColorSpace {
    /// Returns whether this is a polar color space (has hue component)
    #[inline]
    pub fn is_polar(&self) -> bool {
        matches!(self, Self::Hsl | Self::Hwb | Self::Lch | Self::Oklch)
    }

    /// Returns whether this is a rectangular color space
    #[inline]
    pub fn is_rectangular(&self) -> bool {
        !self.is_polar()
    }

    /// Returns the index of the hue component in the color space, if any
    /// - HSL/HWB: hue is component 0
    /// - LCH/Oklch: hue is component 2
    #[inline]
    pub fn hue_index(&self) -> Option<usize> {
        match self {
            Self::Hsl | Self::Hwb => Some(0),
            Self::Lch | Self::Oklch => Some(2),
            _ => None,
        }
    }
}

/// Channel keywords for relative color syntax and component extraction.
///
/// These are intentionally independent of chart theme parsing so callers can
/// extract components from colors without depending on `avenger-chart-core`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ColorChannel {
    /// Lightness/Luminance (Lab, Lch, Oklab, Oklch, Hsl)
    L,
    /// Red component (RGB, sRGB)
    R,
    /// Green component (RGB, sRGB)
    G,
    /// Blue component (RGB, sRGB)
    B,
    /// Chroma (Oklch, Lch)
    C,
    /// Hue (Hsl, Hwb, Oklch, Lch) - in degrees
    H,
    /// Saturation (Hsl)
    S,
    /// Whiteness (Hwb)
    W,
    /// Blackness (Hwb) - note: 'b' conflicts with blue, so named BlacknessB
    BlacknessB,
    /// A axis (Lab, Oklab)
    A,
    /// B axis (Lab, Oklab) - note: conflicts with blue, so named LabB
    LabB,
    /// X component (XYZ color space)
    X,
    /// Y component (XYZ color space)
    Y,
    /// Z component (XYZ color space)
    Z,
    /// Alpha channel (all color spaces)
    Alpha,
}

impl ColorChannel {
    /// Parse from CSS identifier.
    ///
    /// Returns None if the identifier is not a valid channel keyword.
    pub fn from_ident(ident: &str) -> Option<Self> {
        match ident.to_lowercase().as_str() {
            "l" => Some(Self::L),
            "r" => Some(Self::R),
            "g" => Some(Self::G),
            "b" => Some(Self::B),
            "c" => Some(Self::C),
            "h" => Some(Self::H),
            "s" => Some(Self::S),
            "w" => Some(Self::W),
            "a" => Some(Self::A),
            "x" => Some(Self::X),
            "y" => Some(Self::Y),
            "z" => Some(Self::Z),
            "alpha" => Some(Self::Alpha),
            _ => None,
        }
    }

    /// Parse a channel keyword with color space context.
    ///
    /// This disambiguates:
    /// - `b` as RGB blue vs Lab/Oklab b-axis
    /// - `b` as HWB blackness
    pub fn from_ident_with_color_space(
        ident: &str,
        color_space: Option<ColorSpace>,
    ) -> Option<Self> {
        let lower = ident.to_lowercase();
        match (lower.as_str(), color_space) {
            ("b", Some(ColorSpace::Lab | ColorSpace::Oklab)) => Some(Self::LabB),
            ("b", Some(ColorSpace::Hwb)) => Some(Self::BlacknessB),
            _ => Self::from_ident(ident),
        }
    }
}

/// An absolutely specified color in any color space
///
/// This is similar to Stylo's AbsoluteColor but simplified for Avenger's needs.
/// We don't need flags for "none" components in the initial implementation.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct AbsoluteColor {
    /// The 3 color components (meaning depends on color_space)
    pub components: [f32; 3],
    /// Alpha component (0.0 = transparent, 1.0 = opaque)
    pub alpha: f32,
    /// The color space these components are in
    pub color_space: ColorSpace,
}

impl AbsoluteColor {
    /// Create a new absolute color
    pub fn new(color_space: ColorSpace, c0: f32, c1: f32, c2: f32, alpha: f32) -> Self {
        Self {
            components: [c0, c1, c2],
            alpha,
            color_space,
        }
    }

    /// Create from sRGB components (0.0-1.0 range)
    pub fn from_srgb(r: f32, g: f32, b: f32, alpha: f32) -> Self {
        Self::new(ColorSpace::Srgb, r, g, b, alpha)
    }

    /// Create from normalized sRGB components `[r, g, b, a]`.
    pub fn from_rgba(rgba: [f32; 4]) -> Self {
        Self::from_srgb(rgba[0], rgba[1], rgba[2], rgba[3])
    }

    /// Create from u8 sRGB components `[r, g, b, a]`.
    pub fn from_rgba8(rgba: [u8; 4]) -> Self {
        Self::from_srgb(
            rgba[0] as f32 / 255.0,
            rgba[1] as f32 / 255.0,
            rgba[2] as f32 / 255.0,
            rgba[3] as f32 / 255.0,
        )
    }

    /// Convert to normalized sRGB components `[r, g, b, a]`.
    pub fn to_rgba(&self) -> [f32; 4] {
        // First convert to sRGB if needed
        let srgb = if self.color_space == ColorSpace::Srgb {
            *self
        } else {
            self.to_color_space(ColorSpace::Srgb)
        };

        [
            srgb.components[0].clamp(0.0, 1.0),
            srgb.components[1].clamp(0.0, 1.0),
            srgb.components[2].clamp(0.0, 1.0),
            srgb.alpha.clamp(0.0, 1.0),
        ]
    }

    /// Convert to u8 sRGB components `[r, g, b, a]`.
    ///
    /// Clips channels to [0, 1], scales by 255, then truncates fractional bytes.
    pub fn to_rgba8(&self) -> [u8; 4] {
        let [r, g, b, a] = self.to_rgba();
        [
            (r * 255.0) as u8,
            (g * 255.0) as u8,
            (b * 255.0) as u8,
            (a * 255.0) as u8,
        ]
    }

    /// Convert this color to another color space
    pub fn to_color_space(&self, target: ColorSpace) -> Self {
        if self.color_space == target {
            return *self;
        }

        // Get components in target space (will be implemented in convert.rs)
        let components = convert::convert_color_space(&self.components, self.color_space, target);

        Self {
            components,
            alpha: self.alpha,
            color_space: target,
        }
    }

    /// Extract a component value by channel keyword
    ///
    /// Returns the component value in the color's current color space.
    /// This is used for relative color syntax to extract component values
    /// from an origin color.
    ///
    /// # Arguments
    /// * `keyword` - The channel keyword (l, c, h, r, g, b, etc.)
    ///
    /// # Returns
    /// The component value, or an error if the keyword is invalid for this color space
    pub fn get_component_by_channel_keyword(&self, keyword: ColorChannel) -> Result<f32, String> {
        match (self.color_space, keyword) {
            // Oklch: L (lightness), C (chroma), H (hue)
            (ColorSpace::Oklch, ColorChannel::L) => Ok(self.components[0]),
            (ColorSpace::Oklch, ColorChannel::C) => Ok(self.components[1]),
            (ColorSpace::Oklch, ColorChannel::H) => Ok(self.components[2]),
            (ColorSpace::Oklch, ColorChannel::Alpha | ColorChannel::A) => Ok(self.alpha),

            // Oklab: L (lightness), A (green-red), B (blue-yellow)
            (ColorSpace::Oklab, ColorChannel::L) => Ok(self.components[0]),
            (ColorSpace::Oklab, ColorChannel::A) => Ok(self.components[1]),
            (ColorSpace::Oklab, ColorChannel::LabB) => Ok(self.components[2]),
            (ColorSpace::Oklab, ColorChannel::Alpha) => Ok(self.alpha),

            // Lch: L (lightness), C (chroma), H (hue)
            (ColorSpace::Lch, ColorChannel::L) => Ok(self.components[0]),
            (ColorSpace::Lch, ColorChannel::C) => Ok(self.components[1]),
            (ColorSpace::Lch, ColorChannel::H) => Ok(self.components[2]),
            (ColorSpace::Lch, ColorChannel::Alpha | ColorChannel::A) => Ok(self.alpha),

            // Lab: L (lightness), A (green-red), B (blue-yellow)
            (ColorSpace::Lab, ColorChannel::L) => Ok(self.components[0]),
            (ColorSpace::Lab, ColorChannel::A) => Ok(self.components[1]),
            (ColorSpace::Lab, ColorChannel::LabB) => Ok(self.components[2]),
            (ColorSpace::Lab, ColorChannel::Alpha) => Ok(self.alpha),

            // HSL: H (hue), S (saturation), L (lightness)
            (ColorSpace::Hsl, ColorChannel::H) => Ok(self.components[0]),
            (ColorSpace::Hsl, ColorChannel::S) => Ok(self.components[1]),
            (ColorSpace::Hsl, ColorChannel::L) => Ok(self.components[2]),
            (ColorSpace::Hsl, ColorChannel::Alpha | ColorChannel::A) => Ok(self.alpha),

            // HWB: H (hue), W (whiteness), B (blackness)
            (ColorSpace::Hwb, ColorChannel::H) => Ok(self.components[0]),
            (ColorSpace::Hwb, ColorChannel::W) => Ok(self.components[1]),
            (ColorSpace::Hwb, ColorChannel::BlacknessB) => Ok(self.components[2]),
            (ColorSpace::Hwb, ColorChannel::Alpha | ColorChannel::A) => Ok(self.alpha),

            // sRGB: R, G, B (0-1 range)
            (ColorSpace::Srgb, ColorChannel::R) => Ok(self.components[0]),
            (ColorSpace::Srgb, ColorChannel::G) => Ok(self.components[1]),
            (ColorSpace::Srgb, ColorChannel::B) => Ok(self.components[2]),
            (ColorSpace::Srgb, ColorChannel::Alpha | ColorChannel::A) => Ok(self.alpha),

            _ => Err(format!(
                "Invalid channel keyword {:?} for color space {:?}",
                keyword, self.color_space
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_space_metadata() {
        for (space, hue) in [
            (ColorSpace::Srgb, None),
            (ColorSpace::Lab, None),
            (ColorSpace::Oklab, None),
            (ColorSpace::Hsl, Some(0)),
            (ColorSpace::Hwb, Some(0)),
            (ColorSpace::Lch, Some(2)),
            (ColorSpace::Oklch, Some(2)),
        ] {
            assert_eq!(space.hue_index(), hue);
            assert_eq!(space.is_polar(), hue.is_some());
            assert_eq!(space.is_rectangular(), hue.is_none());
        }
    }

    #[test]
    fn rgba_bytes_scale_clip_and_truncate() {
        for bytes in [[0, 127, 255, 64], [255, 128, 1, 192]] {
            let color = AbsoluteColor::from_rgba8(bytes);
            assert_eq!(color.color_space, ColorSpace::Srgb);
            assert_eq!(color.to_rgba(), bytes.map(|v| v as f32 / 255.0));
            assert_eq!(color.to_rgba8(), bytes);
        }
        for (rgba, clipped, bytes) in [
            (
                [0.5, 0.25, 0.75, 0.5],
                [0.5, 0.25, 0.75, 0.5],
                [127, 63, 191, 127],
            ),
            (
                [-0.1, 1.1, 0.5, 1.5],
                [0.0, 1.0, 0.5, 1.0],
                [0, 255, 127, 255],
            ),
            ([1.0, 0.0, 0.0, -0.5], [1.0, 0.0, 0.0, 0.0], [255, 0, 0, 0]),
        ] {
            let color = AbsoluteColor::from_rgba(rgba);
            assert_eq!(color.to_rgba(), clipped);
            assert_eq!(color.to_rgba8(), bytes);
        }
    }

    #[test]
    fn channel_names_extract_components_in_their_color_space() {
        for (space, names) in [
            (ColorSpace::Srgb, ["r", "g", "b"]),
            (ColorSpace::Hsl, ["h", "s", "l"]),
            (ColorSpace::Hwb, ["h", "w", "b"]),
            (ColorSpace::Lab, ["l", "a", "b"]),
            (ColorSpace::Lch, ["l", "c", "h"]),
            (ColorSpace::Oklab, ["l", "a", "b"]),
            (ColorSpace::Oklch, ["l", "c", "h"]),
        ] {
            let color = AbsoluteColor::new(space, 0.2, 0.4, 0.6, 0.37);
            for (name, expected) in names
                .into_iter()
                .zip(color.components)
                .chain([("alpha", color.alpha)])
            {
                for ident in [name.to_owned(), name.to_uppercase()] {
                    let channel =
                        ColorChannel::from_ident_with_color_space(&ident, Some(space)).unwrap();
                    assert_eq!(
                        color.get_component_by_channel_keyword(channel),
                        Ok(expected),
                        "{space:?} {ident}"
                    );
                }
            }
            if !matches!(space, ColorSpace::Lab | ColorSpace::Oklab) {
                assert_eq!(
                    color.get_component_by_channel_keyword(ColorChannel::A),
                    Ok(color.alpha)
                );
            }
            let invalid = if space == ColorSpace::Srgb {
                ColorChannel::L
            } else {
                ColorChannel::R
            };
            assert!(color.get_component_by_channel_keyword(invalid).is_err());
        }
        assert_eq!(
            ColorChannel::from_ident_with_color_space("B", None),
            Some(ColorChannel::B)
        );
        assert_eq!(ColorChannel::from_ident("unknown"), None);
    }
}
