//! Palettes: the built-in palettes and the palettes of `@palette` rules.

mod builtin;

use avenger_color::{interpolate_colors, ColorSpace, HueInterpolationMethod};

use crate::stylesheet::{Interpolation, PaletteKind};

/// The kind of a scale, which a stylesheet selects and which a palette fits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScaleKind {
    Categorical,
    Ordinal,
    Sequential,
    Diverging,
}

/// A named list of colors for the scales of colors.
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    /// The name, in lowercase.
    pub name: String,
    pub kind: PaletteKind,
    /// The colors as sRGB RGBA: in order for a categorical palette, and stops at equal distances
    /// for a sequential or a diverging one.
    pub colors: Vec<[f32; 4]>,
    pub interpolate: Interpolation,
    pub cvd_safe: bool,
}

impl Palette {
    /// Whether the palette fits a scale of `kind`: a categorical palette fits categorical and
    /// ordinal scales, and a sequential or a diverging palette fits ordinal scales and scales of
    /// its own kind.
    pub fn fits(&self, kind: ScaleKind) -> bool {
        matches!(
            (self.kind, kind),
            (_, ScaleKind::Ordinal)
                | (PaletteKind::Categorical, ScaleKind::Categorical)
                | (PaletteKind::Sequential, ScaleKind::Sequential)
                | (PaletteKind::Diverging, ScaleKind::Diverging)
        )
    }

    /// The color space in which the palette interpolates.
    pub fn space(&self) -> ColorSpace {
        match self.interpolate {
            Interpolation::Srgb => ColorSpace::Srgb,
            Interpolation::SrgbLinear => ColorSpace::SrgbLinear,
            Interpolation::Lab => ColorSpace::Lab,
            Interpolation::Oklab => ColorSpace::Oklab,
            Interpolation::Lch => ColorSpace::Lch,
            Interpolation::Oklch => ColorSpace::Oklch,
            Interpolation::Hsl => ColorSpace::Hsl,
        }
    }

    /// The colors at `positions` from 0 to 1, interpolated in the palette's space.
    pub fn interpolate(&self, positions: &[f32]) -> Vec<[f32; 4]> {
        interpolate_colors(
            self.space(),
            HueInterpolationMethod::Shorter,
            &self.colors,
            positions,
        )
        .expect("a palette has a color")
    }

    /// `count` colors for an ordinal scale: the colors in order for a categorical palette, and
    /// samples at `k / (count + 1)` of a sequential or a diverging one, so no value gets an end
    /// color.
    pub fn sample(&self, count: usize) -> Vec<[f32; 4]> {
        match self.kind {
            PaletteKind::Categorical => (0..count)
                .map(|index| self.colors[index % self.colors.len()])
                .collect(),
            _ => {
                let positions: Vec<f32> =
                    (1..=count).map(|k| k as f32 / (count + 1) as f32).collect();
                self.interpolate(&positions)
            }
        }
    }

    pub(crate) fn builtin() -> Vec<Palette> {
        builtin::BUILTIN
            .iter()
            .map(|(name, kind, hex, cvd_safe)| Palette {
                name: name.to_string(),
                kind: *kind,
                colors: (0..hex.len() / 6)
                    .map(|index| {
                        let channel = |at: usize| {
                            u8::from_str_radix(&hex[index * 6 + at..index * 6 + at + 2], 16)
                                .expect("a hex color") as f32
                                / 255.0
                        };
                        [channel(0), channel(2), channel(4), 1.0]
                    })
                    .collect(),
                interpolate: Interpolation::Srgb,
                cvd_safe: *cvd_safe,
            })
            .collect()
    }
}
