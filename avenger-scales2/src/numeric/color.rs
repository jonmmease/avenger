use avenger_common::types::ColorOrGradient;
use palette::{Hsla, IntoColor, Laba, Mix, Srgba};
use std::fmt::Debug;

/// A trait for color spaces that can be used with the `NumericColorScale`
pub trait ColorSpace:
    Mix<Scalar = f32> + Copy + IntoColor<Srgba> + Debug + Send + Sync + 'static
{
}

impl<T: Mix<Scalar = f32> + Copy + IntoColor<Srgba> + Debug + Send + Sync + 'static> ColorSpace
    for T
{
}

pub trait ColorInterpolator {
    fn interp(&self, v: f32) -> ColorOrGradient;
}

pub struct SrgbaColorInterpolator {
    colors: Vec<Srgba>,
}

impl SrgbaColorInterpolator {
    pub fn new(colors: Vec<Srgba>) -> Self {
        Self { colors }
    }
}

impl ColorInterpolator for SrgbaColorInterpolator {
    fn interp(&self, v: f32) -> ColorOrGradient {
        interp_color_to_color_or_gradient(&self.colors, v)
    }
}

pub struct HslaColorInterpolator {
    colors: Vec<Hsla>,
}

impl HslaColorInterpolator {
    pub fn new(colors: Vec<Srgba>) -> Self {
        Self {
            colors: colors.into_iter().map(|c| c.into_color()).collect(),
        }
    }
}

impl ColorInterpolator for HslaColorInterpolator {
    fn interp(&self, v: f32) -> ColorOrGradient {
        interp_color_to_color_or_gradient(&self.colors, v)
    }
}

pub struct LabaColorInterpolator {
    colors: Vec<Laba>,
}

impl LabaColorInterpolator {
    pub fn new(colors: Vec<Srgba>) -> Self {
        Self {
            colors: colors.into_iter().map(|c| c.into_color()).collect(),
        }
    }
}

impl ColorInterpolator for LabaColorInterpolator {
    fn interp(&self, v: f32) -> ColorOrGradient {
        interp_color_to_color_or_gradient(&self.colors, v)
    }
}

pub fn interp_color_to_color_or_gradient<C: ColorSpace>(colors: &[C], value: f32) -> ColorOrGradient
where
    C: ColorSpace,
{
    if !value.is_finite() {
        // Return transparent black if the value is not finite
        ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])
    } else {
        let lower = value.floor() as usize;
        let upper = value.ceil() as usize;
        let interp_factor = value - lower as f32;
        let mixed = colors[lower].mix(colors[upper], interp_factor);
        let c: Srgba = mixed.into_color();
        ColorOrGradient::Color([c.red, c.green, c.blue, c.alpha])
    }
}
