//! Color spaces and color values.

use crate::convert;

/// Color spaces supported by [`AbsoluteColor`].
///
/// Components use the order and units described by each variant. Component values
/// can exceed their nominal ranges during conversion or interpolation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum ColorSpace {
    /// sRGB red, green, and blue, each nominally in [0, 1].
    Srgb = 0,
    /// sRGB hue in degrees, saturation in percent, and lightness in percent.
    Hsl,
    /// sRGB hue in degrees, whiteness in percent, and blackness in percent.
    Hwb,
    /// CIE D50 lightness (0–100), a-axis, and b-axis.
    Lab,
    /// CIE D50 lightness (0–100), chroma, and hue in degrees.
    Lch,
    /// Oklab lightness (0–1), a-axis, and b-axis.
    Oklab,
    /// Oklab lightness (0–1), chroma, and hue in degrees.
    Oklch,
}

impl ColorSpace {
    /// Whether the color space has a hue component.
    #[inline]
    pub fn is_polar(&self) -> bool {
        matches!(self, Self::Hsl | Self::Hwb | Self::Lch | Self::Oklch)
    }

    /// Whether the color space has no hue component.
    #[inline]
    pub fn is_rectangular(&self) -> bool {
        !self.is_polar()
    }

    /// Index of the hue component, or `None` for rectangular spaces.
    ///
    /// HSL and HWB use index 0. LCH and Oklch use index 2.
    #[inline]
    pub fn hue_index(&self) -> Option<usize> {
        match self {
            Self::Hsl | Self::Hwb => Some(0),
            Self::Lch | Self::Oklch => Some(2),
            _ => None,
        }
    }
}

/// Three color components and alpha in a supported [`ColorSpace`].
///
/// Constructors retain the supplied values without clipping. Polar conversions
/// represent undefined hue as `NaN`. [`Self::to_rgba`] converts to sRGB and clips
/// all four channels to [0, 1].
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct AbsoluteColor {
    /// Color components in the order and units defined by [`ColorSpace`].
    pub components: [f32; 3],
    /// Opacity: 0 is transparent and 1 is opaque.
    pub alpha: f32,
    /// Color space that defines the component order and units.
    pub color_space: ColorSpace,
}

impl AbsoluteColor {
    /// Create a color from components in the order and units of `color_space`.
    pub fn new(color_space: ColorSpace, c0: f32, c1: f32, c2: f32, alpha: f32) -> Self {
        Self {
            components: [c0, c1, c2],
            alpha,
            color_space,
        }
    }

    /// Create a color from sRGB red, green, blue, and alpha, nominally in [0, 1].
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

    /// Convert to sRGB `[r, g, b, a]`, clipping all four channels to [0, 1].
    pub fn to_rgba(&self) -> [f32; 4] {
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

    /// Convert the color components without clipping, preserving alpha.
    pub fn to_color_space(&self, target: ColorSpace) -> Self {
        if self.color_space == target {
            return *self;
        }

        let components = convert::convert_color_space(&self.components, self.color_space, target);

        Self {
            components,
            alpha: self.alpha,
            color_space: target,
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
}
