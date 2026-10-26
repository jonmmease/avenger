//! Widget fonts, colors, sizes and spacing.

use crate::widget::Look;

/// The colors of a widget part in one state, as RGBA with components from 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Paint {
    pub fill: [f32; 4],
    pub border: [f32; 4],
    /// The color of text and glyphs, such as a check mark.
    pub foreground: [f32; 4],
}

impl Paint {
    /// Blend each color's RGB components toward `background` by `amount`, preserving alpha.
    /// An opaque color blended by 0.55 appears like the original at 45% opacity over that
    /// background. It stays opaque, so overlapping parts do not show through each other.
    pub fn blend(&self, background: [f32; 4], amount: f32) -> Self {
        let blend = |color: [f32; 4]| {
            let mix =
                |channel: usize| color[channel] + (background[channel] - color[channel]) * amount;
            [mix(0), mix(1), mix(2), color[3]]
        };
        Self {
            fill: blend(self.fill),
            border: blend(self.border),
            foreground: blend(self.foreground),
        }
    }
}

/// A widget part's paint in each state.
#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    pub normal: Paint,
    pub hovered: Paint,
    pub pressed: Paint,
    pub disabled: Paint,
}

impl Palette {
    /// Select paint in priority order: disabled, pressed, hovered, normal.
    pub fn paint(&self, look: Look) -> Paint {
        if !look.enabled {
            self.disabled
        } else if look.pressed {
            self.pressed
        } else if look.hovered {
            self.hovered
        } else {
            self.normal
        }
    }

    /// A palette whose hovered and pressed paints change only the fill, and whose disabled paint
    /// blends the normal paint 55% toward `background`.
    fn shaded(normal: Paint, hovered: [f32; 4], pressed: [f32; 4], background: [f32; 4]) -> Self {
        Self {
            normal,
            hovered: Paint {
                fill: hovered,
                ..normal
            },
            pressed: Paint {
                fill: pressed,
                ..normal
            },
            disabled: normal.blend(background, 0.55),
        }
    }
}

/// Fonts, colors, sizes and spacing for a [`Widgets`](crate::Widgets) tool.
/// `WidgetStyle::default()` is a light style, and [`WidgetStyle::dark`] a dark style.
#[derive(Debug, Clone, PartialEq)]
pub struct WidgetStyle {
    /// A CSS-style list of font families for labels.
    pub font_family: String,
    pub font_size: f32,
    /// The color of the surface that the widgets sit on.
    pub background: [f32; 4],
    /// Control surfaces: a button, an unchecked box or radio button, a slider's track and
    /// thumb, a group box's border, and every label.
    pub neutral: Palette,
    /// Selection: a checked box, a selected radio button and the filled part of a slider.
    pub accent: Palette,
    pub border_width: f32,
    pub corner_radius: f32,
    /// Horizontal and vertical padding between a button's border and label.
    pub button_padding: [f32; 2],
    /// The gap between a control and its label, and below a group's label or title.
    pub label_gap: f32,
    /// The gaps between a radio group's items in a row and in a column.
    pub item_gap: [f32; 2],
    /// The side length of a check box and the diameter of a radio button.
    pub control_size: f32,
    pub track_height: f32,
    /// The diameter of a slider's thumb.
    pub thumb_size: f32,
    /// A slider track's preferred and minimum lengths.
    pub track_length: [f32; 2],
    pub focus_color: [f32; 4],
    pub focus_width: f32,
    /// The gap between a control's bounds and its focus ring.
    pub focus_offset: f32,
    /// The space between a group box's border and the widgets inside it.
    pub group_padding: f32,
}

impl Default for WidgetStyle {
    fn default() -> Self {
        let background = [1.0, 1.0, 1.0, 1.0];
        let neutral = Paint {
            fill: [0.97, 0.97, 0.98, 1.0],
            border: [0.6, 0.62, 0.66, 1.0],
            foreground: [0.13, 0.14, 0.16, 1.0],
        };
        let accent = Paint {
            fill: [0.16, 0.42, 0.85, 1.0],
            border: [0.16, 0.42, 0.85, 1.0],
            foreground: [1.0, 1.0, 1.0, 1.0],
        };
        Self {
            font_family: "sans-serif".to_string(),
            font_size: 13.0,
            background,
            neutral: Palette::shaded(
                neutral,
                [0.92, 0.93, 0.95, 1.0],
                [0.86, 0.87, 0.9, 1.0],
                background,
            ),
            accent: Palette::shaded(
                accent,
                [0.12, 0.36, 0.77, 1.0],
                [0.09, 0.3, 0.67, 1.0],
                background,
            ),
            border_width: 1.0,
            corner_radius: 4.0,
            button_padding: [10.0, 5.0],
            label_gap: 6.0,
            item_gap: [14.0, 6.0],
            control_size: 14.0,
            track_height: 4.0,
            thumb_size: 14.0,
            track_length: [160.0, 60.0],
            focus_color: [0.16, 0.42, 0.85, 0.9],
            focus_width: 2.0,
            focus_offset: 2.0,
            group_padding: 8.0,
        }
    }
}

impl WidgetStyle {
    /// A dark style, for widgets on a dark surface.
    pub fn dark() -> Self {
        let background = [0.12, 0.13, 0.15, 1.0];
        let neutral = Paint {
            fill: [0.2, 0.21, 0.24, 1.0],
            border: [0.46, 0.48, 0.53, 1.0],
            foreground: [0.9, 0.91, 0.93, 1.0],
        };
        let accent = Paint {
            fill: [0.36, 0.58, 0.98, 1.0],
            border: [0.36, 0.58, 0.98, 1.0],
            foreground: [0.06, 0.08, 0.12, 1.0],
        };
        Self {
            background,
            neutral: Palette::shaded(
                neutral,
                [0.25, 0.26, 0.3, 1.0],
                [0.3, 0.31, 0.36, 1.0],
                background,
            ),
            accent: Palette::shaded(
                accent,
                [0.45, 0.65, 1.0, 1.0],
                [0.53, 0.71, 1.0, 1.0],
                background,
            ),
            focus_color: [0.45, 0.65, 1.0, 0.9],
            ..Self::default()
        }
    }
}
