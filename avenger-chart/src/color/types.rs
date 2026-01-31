//! Core color types for color space support

use crate::{
    color::convert,
    theme::{CssRgba, calc::ChannelKeyword},
};

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
    /// Lab color space (CIE L*a*b*)
    Lab,
    /// LCH (Lightness-Chroma-Hue) - cylindrical Lab
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

    /// Create from CssRgba (0-255 range)
    pub fn from_css_rgba(rgba: &CssRgba) -> Self {
        Self::from_srgb(
            rgba.red as f32 / 255.0,
            rgba.green as f32 / 255.0,
            rgba.blue as f32 / 255.0,
            rgba.alpha as f32 / 255.0,
        )
    }

    /// Convert to CssRgba (0-255 range)
    pub fn to_css_rgba(&self) -> CssRgba {
        // First convert to sRGB if needed
        let srgb = if self.color_space == ColorSpace::Srgb {
            *self
        } else {
            self.to_color_space(ColorSpace::Srgb)
        };

        // Clamp components to [0, 1] and convert to u8
        let r = (srgb.components[0].clamp(0.0, 1.0) * 255.0) as u8;
        let g = (srgb.components[1].clamp(0.0, 1.0) * 255.0) as u8;
        let b = (srgb.components[2].clamp(0.0, 1.0) * 255.0) as u8;
        let a = (srgb.alpha.clamp(0.0, 1.0) * 255.0) as u8;

        CssRgba {
            red: r,
            green: g,
            blue: b,
            alpha: a,
        }
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
    pub fn get_component_by_channel_keyword(&self, keyword: ChannelKeyword) -> Result<f32, String> {
        match (self.color_space, keyword) {
            // Oklch: L (lightness), C (chroma), H (hue)
            (ColorSpace::Oklch, ChannelKeyword::L) => Ok(self.components[0]),
            (ColorSpace::Oklch, ChannelKeyword::C) => Ok(self.components[1]),
            (ColorSpace::Oklch, ChannelKeyword::H) => Ok(self.components[2]),
            (ColorSpace::Oklch, ChannelKeyword::Alpha | ChannelKeyword::A) => Ok(self.alpha),

            // Oklab: L (lightness), A (green-red), B (blue-yellow)
            (ColorSpace::Oklab, ChannelKeyword::L) => Ok(self.components[0]),
            (ColorSpace::Oklab, ChannelKeyword::A) => Ok(self.components[1]),
            (ColorSpace::Oklab, ChannelKeyword::LabB) => Ok(self.components[2]),
            (ColorSpace::Oklab, ChannelKeyword::Alpha) => Ok(self.alpha),

            // Lch: L (lightness), C (chroma), H (hue)
            (ColorSpace::Lch, ChannelKeyword::L) => Ok(self.components[0]),
            (ColorSpace::Lch, ChannelKeyword::C) => Ok(self.components[1]),
            (ColorSpace::Lch, ChannelKeyword::H) => Ok(self.components[2]),
            (ColorSpace::Lch, ChannelKeyword::Alpha | ChannelKeyword::A) => Ok(self.alpha),

            // Lab: L (lightness), A (green-red), B (blue-yellow)
            (ColorSpace::Lab, ChannelKeyword::L) => Ok(self.components[0]),
            (ColorSpace::Lab, ChannelKeyword::A) => Ok(self.components[1]),
            (ColorSpace::Lab, ChannelKeyword::LabB) => Ok(self.components[2]),
            (ColorSpace::Lab, ChannelKeyword::Alpha) => Ok(self.alpha),

            // HSL: H (hue), S (saturation), L (lightness)
            (ColorSpace::Hsl, ChannelKeyword::H) => Ok(self.components[0]),
            (ColorSpace::Hsl, ChannelKeyword::S) => Ok(self.components[1]),
            (ColorSpace::Hsl, ChannelKeyword::L) => Ok(self.components[2]),
            (ColorSpace::Hsl, ChannelKeyword::Alpha | ChannelKeyword::A) => Ok(self.alpha),

            // HWB: H (hue), W (whiteness), B (blackness)
            (ColorSpace::Hwb, ChannelKeyword::H) => Ok(self.components[0]),
            (ColorSpace::Hwb, ChannelKeyword::W) => Ok(self.components[1]),
            (ColorSpace::Hwb, ChannelKeyword::BlacknessB) => Ok(self.components[2]),
            (ColorSpace::Hwb, ChannelKeyword::Alpha | ChannelKeyword::A) => Ok(self.alpha),

            // sRGB: R, G, B (0-1 range)
            (ColorSpace::Srgb, ChannelKeyword::R) => Ok(self.components[0]),
            (ColorSpace::Srgb, ChannelKeyword::G) => Ok(self.components[1]),
            (ColorSpace::Srgb, ChannelKeyword::B) => Ok(self.components[2]),
            (ColorSpace::Srgb, ChannelKeyword::Alpha | ChannelKeyword::A) => Ok(self.alpha),

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
    fn test_color_space_is_polar() {
        assert!(ColorSpace::Hsl.is_polar());
        assert!(ColorSpace::Hwb.is_polar());
        assert!(ColorSpace::Lch.is_polar());
        assert!(ColorSpace::Oklch.is_polar());
        assert!(!ColorSpace::Srgb.is_polar());
        assert!(!ColorSpace::Lab.is_polar());
        assert!(!ColorSpace::Oklab.is_polar());
    }

    #[test]
    fn test_color_space_hue_index() {
        assert_eq!(ColorSpace::Hsl.hue_index(), Some(0));
        assert_eq!(ColorSpace::Hwb.hue_index(), Some(0));
        assert_eq!(ColorSpace::Lch.hue_index(), Some(2));
        assert_eq!(ColorSpace::Oklch.hue_index(), Some(2));
        assert_eq!(ColorSpace::Srgb.hue_index(), None);
        assert_eq!(ColorSpace::Lab.hue_index(), None);
    }

    #[test]
    fn test_from_css_rgba() {
        let rgba = CssRgba {
            red: 255,
            green: 0,
            blue: 0,
            alpha: 255,
        };
        let color = AbsoluteColor::from_css_rgba(&rgba);
        assert_eq!(color.components[0], 1.0);
        assert_eq!(color.components[1], 0.0);
        assert_eq!(color.components[2], 0.0);
        assert_eq!(color.alpha, 1.0);
        assert_eq!(color.color_space, ColorSpace::Srgb);
    }

    #[test]
    fn test_to_css_rgba() {
        let color = AbsoluteColor::from_srgb(1.0, 0.0, 0.0, 1.0);
        let rgba = color.to_css_rgba();
        assert_eq!(rgba.red, 255);
        assert_eq!(rgba.green, 0);
        assert_eq!(rgba.blue, 0);
        assert_eq!(rgba.alpha, 255);
    }
}
