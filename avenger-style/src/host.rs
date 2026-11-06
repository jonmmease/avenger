//! The host inputs: the host box, the preferences of the reader, and the host theme.

use std::sync::Arc;

use crate::{table::Sheet, values::SystemColor, Env};

/// What the host gives a render: the box that the chart draws in, the preferences, and the host
/// theme. Conditions read only these.
#[derive(Clone, Debug, Default)]
pub struct HostInputs {
    /// The width of the host box in CSS pixels, when the host gives one.
    pub width: Option<f32>,
    /// The height of the host box in CSS pixels, when the host gives one.
    pub height: Option<f32>,
    pub preferences: Preferences,
    /// A stylesheet of the user origin, with the values of its `env()` names.
    pub theme: Option<Arc<HostTheme>>,
}

impl HostInputs {
    /// The host box of a size in both dimensions, with the default preferences.
    pub fn sized(width: f32, height: f32) -> Self {
        HostInputs {
            width: Some(width),
            height: Some(height),
            ..Default::default()
        }
    }

    /// Whether `other` gives the same preferences and the same host theme.
    pub(crate) fn same_preferences_and_theme(&self, other: &HostInputs) -> bool {
        self.preferences == other.preferences
            && match (&self.theme, &other.theme) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
    }
}

/// The host theme: a stylesheet and the values that the host gives its `env()` names.
#[derive(Clone, Debug)]
pub struct HostTheme {
    pub sheet: Sheet,
    pub env: Env,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorScheme {
    #[default]
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Contrast {
    #[default]
    NoPreference,
    More,
    Less,
    Custom,
}

/// The preferences of the reader, which `@media` conditions test.
#[derive(Clone, Debug, PartialEq)]
pub struct Preferences {
    pub color_scheme: ColorScheme,
    pub contrast: Contrast,
    pub reduced_motion: bool,
    pub forced_colors: bool,
    /// Whether the output is for printing.
    pub print: bool,
    /// The text scale: `1rem` is 10 CSS pixels times this.
    pub text_scale: f32,
    /// The values of the system colors as sRGB RGBA, in the order of `SystemColor::ALL`, or
    /// `None` for the defaults of the color scheme.
    pub system_colors: Option<[[f32; 4]; 6]>,
}

impl Default for Preferences {
    fn default() -> Self {
        Preferences {
            color_scheme: ColorScheme::Light,
            contrast: Contrast::NoPreference,
            reduced_motion: false,
            forced_colors: false,
            print: false,
            text_scale: 1.0,
            system_colors: None,
        }
    }
}

impl Preferences {
    /// The value of a system color: the host's, or the default of the color scheme.
    pub fn system_color(&self, color: SystemColor) -> [f32; 4] {
        let index = SystemColor::ALL
            .iter()
            .position(|known| *known == color)
            .expect("a system color");
        if let Some(colors) = &self.system_colors {
            return colors[index];
        }
        let hex = match self.color_scheme {
            ColorScheme::Light => ["ffffff", "000000", "37006e", "ffffff", "600000", "00009f"],
            ColorScheme::Dark => ["000000", "ffffff", "1aebff", "000000", "3ff23f", "ffff00"],
        }[index];
        let channel =
            |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).expect("hex") as f32 / 255.0;
        [channel(0), channel(2), channel(4), 1.0]
    }
}
