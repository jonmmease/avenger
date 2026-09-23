pub mod contrast;
pub mod interpolate;
pub mod model;
pub mod parse;

pub use contrast::relative_luminance_srgb;
pub use interpolate::{interpolate_colors, ColorInterpolationError, ColorInterpolationSpace};
pub use model::{
    apply_opacity_to_color, ColorOrGradient, Gradient, GradientStop, LinearGradient, RadialGradient,
};
pub use parse::{parse_color_string, parse_color_string_strict, ColorParseError};
