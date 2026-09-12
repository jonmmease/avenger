use crate::WidgetError;
use avenger_text::{
    measurement::TextMeasurementConfig,
    types::{FontStyle, FontWeight, TextSyntaxMode},
};

/// Plain text typography, in logical pixels, shared by measurement and paint.
#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    pub font: String,
    pub size: f32,
    pub weight: FontWeight,
    pub style: FontStyle,
}
impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font: "sans-serif".into(),
            size: 14.0,
            weight: FontWeight::default(),
            style: FontStyle::Normal,
        }
    }
}
impl TextStyle {
    pub(crate) fn config<'a>(&'a self, text: &'a str) -> TextMeasurementConfig<'a> {
        TextMeasurementConfig {
            text,
            font: &self.font,
            font_size: self.size,
            font_weight: self.weight,
            font_style: self.style,
            syntax_mode: TextSyntaxMode::Plain,
            params: avenger_text::empty_label_params(),
            number_locale: None,
            number_locale_specs: None,
            datetime_locale: None,
            datetime_timezone: None,
            datetime_locale_specs: None,
        }
    }
    pub(crate) fn validate(&self) -> Result<(), WidgetError> {
        if self.font.is_empty() || !self.size.is_finite() || self.size <= 0.0 {
            Err(WidgetError::Invalid(
                "text style requires a font and a positive finite size".into(),
            ))
        } else {
            Ok(())
        }
    }
}

/// Fill, border, and foreground RGBA colors with components in 0..=1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlPaint {
    pub fill: [f32; 4],
    pub border: [f32; 4],
    pub foreground: [f32; 4],
}

/// Paint for mutually exclusive interaction states. Geometry is independent.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintStates<P = ControlPaint> {
    pub normal: P,
    pub hovered: P,
    pub pressed: P,
    pub disabled: P,
}
impl<P: Copy> PaintStates<P> {
    pub(crate) fn resolve(&self, enabled: bool, hovered: bool, pressed: bool) -> P {
        if !enabled {
            self.disabled
        } else if pressed {
            self.pressed
        } else if hovered {
            self.hovered
        } else {
            self.normal
        }
    }
    fn map<T>(&self, f: impl Fn(P) -> T) -> PaintStates<T> {
        PaintStates {
            normal: f(self.normal),
            hovered: f(self.hovered),
            pressed: f(self.pressed),
            disabled: f(self.disabled),
        }
    }
}
impl PaintStates {
    pub(crate) fn validate(&self) -> Result<(), WidgetError> {
        for p in [self.normal, self.hovered, self.pressed, self.disabled] {
            colors(&[p.fill, p.border, p.foreground])?;
        }
        Ok(())
    }
}

fn colors(values: &[[f32; 4]]) -> Result<(), WidgetError> {
    if values
        .iter()
        .flatten()
        .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
    {
        Ok(())
    } else {
        Err(WidgetError::Invalid(
            "paint colors require finite RGBA components in 0..=1".into(),
        ))
    }
}

/// A focus outline outside the control box, measured in logical pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct FocusStyle {
    pub color: [f32; 4],
    pub width: f32,
    pub gap: f32,
}
impl Default for FocusStyle {
    fn default() -> Self {
        Self {
            color: [0.10, 0.39, 0.84, 1.0],
            width: 2.0,
            gap: 2.0,
        }
    }
}
impl FocusStyle {
    pub(crate) fn overflow(&self) -> f32 {
        self.gap + self.width
    }
    pub(crate) fn validate(&self) -> Result<(), WidgetError> {
        lengths(&[self.width, self.gap])?;
        if self
            .color
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        {
            return Err(WidgetError::Invalid(
                "focus color requires finite RGBA components in 0..=1".into(),
            ));
        }
        Ok(())
    }
}

/// Button typography, fixed geometry, and neutral/accent interaction paint.
#[derive(Clone, Debug, PartialEq)]
pub struct ButtonStyle {
    pub text: TextStyle,
    pub height: f32,
    pub min_width: f32,
    pub padding: f32,
    pub radius: f32,
    pub border_width: f32,
    pub focus: FocusStyle,
    pub neutral: PaintStates,
    pub accent: PaintStates,
}
/// Checkbox row geometry and checked/unchecked paint.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckboxStyle {
    pub text: TextStyle,
    pub row_height: f32,
    pub box_size: f32,
    pub gap: f32,
    pub radius: f32,
    pub border_width: f32,
    pub focus: FocusStyle,
    pub unchecked: PaintStates,
    pub checked: PaintStates,
}

/// Radio colors, including the selected outline and center indicator.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadioPaint {
    pub fill: [f32; 4],
    pub border: [f32; 4],
    pub indicator: [f32; 4],
    pub foreground: [f32; 4],
}

/// Circular radio geometry and interaction paint.
#[derive(Clone, Debug, PartialEq)]
pub struct RadioStyle {
    pub text: TextStyle,
    pub row_height: f32,
    pub diameter: f32,
    pub gap: f32,
    pub border_width: f32,
    pub focus: FocusStyle,
    pub paint: PaintStates<RadioPaint>,
}

/// Colors for a slider's track, progress, thumb, and optional readout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliderPaint {
    pub track: [f32; 4],
    pub progress: [f32; 4],
    pub thumb: [f32; 4],
    pub thumb_border: [f32; 4],
    pub foreground: [f32; 4],
}

/// Spacing shared by checkbox and radio groups, in logical pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct ChoiceGroupStyle {
    pub gap: f32,
    pub label_gap: f32,
    pub text: TextStyle,
}
/// Horizontal slider geometry. The readout reserves a fixed width when present.
#[derive(Clone, Debug, PartialEq)]
pub struct SliderStyle {
    pub text: TextStyle,
    pub width: f32,
    pub height: f32,
    pub thumb_size: f32,
    pub track_height: f32,
    pub readout_width: f32,
    pub readout_gap: f32,
    pub focus: FocusStyle,
    pub paint: PaintStates<SliderPaint>,
}

/// Single-line field typography, geometry, and editor paint in logical pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct TextInputStyle {
    pub text: TextStyle,
    pub width: f32,
    pub height: f32,
    pub padding: f32,
    pub radius: f32,
    pub border_width: f32,
    pub focus: FocusStyle,
    pub paint: PaintStates,
    pub read_only: ControlPaint,
    pub invalid_border: [f32; 4],
    pub selection: [f32; 4],
    pub caret: [f32; 4],
    pub placeholder: [f32; 4],
}

/// Concrete widget styles. State changes alter paint without changing measurement.
#[derive(Clone, Debug, PartialEq)]
pub struct WidgetTheme {
    pub button: ButtonStyle,
    pub checkbox: CheckboxStyle,
    pub radio: RadioStyle,
    pub group: ChoiceGroupStyle,
    pub slider: SliderStyle,
    pub text_input: TextInputStyle,
}

fn states(fill: [f32; 4], border: [f32; 4], foreground: [f32; 4], dark: bool) -> PaintStates {
    let shifted = |delta: f32| {
        let mut c = fill;
        for v in &mut c[..3] {
            *v = (*v + delta).clamp(0.0, 1.0);
        }
        c
    };
    let normal = ControlPaint {
        fill,
        border,
        foreground,
    };
    let mut disabled = normal;
    disabled.foreground[3] *= 0.45;
    disabled.border[3] *= 0.45;
    disabled.fill[3] *= 0.45;
    PaintStates {
        normal,
        hovered: ControlPaint {
            fill: shifted(if dark { 0.045 } else { -0.035 }),
            ..normal
        },
        pressed: ControlPaint {
            fill: shifted(if dark { 0.075 } else { -0.07 }),
            ..normal
        },
        disabled,
    }
}
impl Default for WidgetTheme {
    fn default() -> Self {
        Self::light()
    }
}
impl WidgetTheme {
    /// Styles for a light background.
    pub fn light() -> Self {
        Self::make(false)
    }
    /// Styles for a dark background.
    pub fn dark() -> Self {
        Self::make(true)
    }
    fn make(dark: bool) -> Self {
        let fill = if dark {
            [0.12, 0.15, 0.20, 1.0]
        } else {
            [0.96, 0.97, 0.985, 1.0]
        };
        let ink = if dark {
            [0.90, 0.93, 0.98, 1.0]
        } else {
            [0.07, 0.12, 0.19, 1.0]
        };
        let border = if dark {
            [0.30, 0.36, 0.44, 1.0]
        } else {
            [0.63, 0.69, 0.76, 1.0]
        };
        let neutral = states(fill, border, ink, dark);
        let accent = states(
            [0.08, 0.33, 0.67, 1.0],
            [0.06, 0.25, 0.52, 1.0],
            [1.0; 4],
            dark,
        );
        let mut focus = FocusStyle::default();
        if dark {
            focus.color = [0.35, 0.65, 1.0, 1.0];
        }
        Self {
            text_input: TextInputStyle {
                text: TextStyle::default(),
                width: 240.0,
                height: 36.0,
                padding: 9.0,
                radius: 4.0,
                border_width: 1.0,
                focus: focus.clone(),
                paint: neutral.clone(),
                read_only: ControlPaint {
                    fill,
                    border,
                    foreground: ink,
                },
                invalid_border: if dark {
                    [1.0, 0.36, 0.30, 1.0]
                } else {
                    [0.70, 0.10, 0.08, 1.0]
                },
                selection: if dark {
                    [0.22, 0.45, 0.80, 0.65]
                } else {
                    [0.25, 0.53, 0.90, 0.30]
                },
                caret: ink,
                placeholder: [ink[0], ink[1], ink[2], 0.5],
            },
            button: ButtonStyle {
                text: TextStyle::default(),
                height: 36.0,
                min_width: 80.0,
                padding: 14.0,
                radius: 5.0,
                border_width: 1.0,
                focus: focus.clone(),
                neutral: neutral.clone(),
                accent: accent.clone(),
            },
            radio: RadioStyle {
                text: TextStyle::default(),
                row_height: 30.0,
                diameter: 18.0,
                gap: 9.0,
                border_width: 1.5,
                focus: focus.clone(),
                paint: neutral.map(|p| RadioPaint {
                    fill: p.fill,
                    border: p.border,
                    indicator: [
                        accent.normal.fill[0],
                        accent.normal.fill[1],
                        accent.normal.fill[2],
                        p.fill[3],
                    ],
                    foreground: p.foreground,
                }),
            },
            group: ChoiceGroupStyle {
                gap: 12.0,
                label_gap: 8.0,
                text: TextStyle::default(),
            },
            slider: SliderStyle {
                text: TextStyle::default(),
                width: 240.0,
                height: 36.0,
                thumb_size: 18.0,
                track_height: 4.0,
                readout_width: 48.0,
                readout_gap: 10.0,
                focus: focus.clone(),
                paint: PaintStates {
                    normal: (neutral.normal, accent.normal),
                    hovered: (neutral.hovered, accent.hovered),
                    pressed: (neutral.pressed, accent.pressed),
                    disabled: (neutral.disabled, accent.disabled),
                }
                .map(|(n, a)| SliderPaint {
                    track: n.border,
                    progress: a.fill,
                    thumb: a.fill,
                    thumb_border: a.border,
                    foreground: n.foreground,
                }),
            },
            checkbox: CheckboxStyle {
                text: TextStyle::default(),
                row_height: 30.0,
                box_size: 18.0,
                gap: 9.0,
                radius: 3.0,
                border_width: 1.0,
                focus: focus.clone(),
                unchecked: neutral.clone(),
                checked: accent.clone(),
            },
        }
    }
    pub(crate) fn validate(&self) -> Result<(), WidgetError> {
        let t = &self.text_input;
        t.text.validate()?;
        t.focus.validate()?;
        t.paint.validate()?;
        lengths(&[t.width, t.height, t.padding, t.radius, t.border_width])?;
        for color in [
            t.read_only.fill,
            t.read_only.border,
            t.read_only.foreground,
            t.invalid_border,
            t.selection,
            t.caret,
            t.placeholder,
        ] {
            if color
                .iter()
                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            {
                return Err(WidgetError::Invalid(
                    "text colors require finite RGBA components in 0..=1".into(),
                ));
            }
        }
        let b = &self.button;
        b.text.validate()?;
        b.focus.validate()?;
        b.neutral.validate()?;
        b.accent.validate()?;
        lengths(&[b.height, b.min_width, b.padding, b.radius, b.border_width])?;
        self.group.text.validate()?;
        lengths(&[self.group.gap, self.group.label_gap])?;
        let s = &self.slider;
        s.text.validate()?;
        s.focus.validate()?;
        for p in [
            s.paint.normal,
            s.paint.hovered,
            s.paint.pressed,
            s.paint.disabled,
        ] {
            colors(&[p.track, p.progress, p.thumb, p.thumb_border, p.foreground])?;
        }
        lengths(&[
            s.width,
            s.height,
            s.thumb_size,
            s.track_height,
            s.readout_width,
            s.readout_gap,
        ])?;
        let r = &self.radio;
        r.text.validate()?;
        r.focus.validate()?;
        lengths(&[r.row_height, r.diameter, r.gap, r.border_width])?;
        for p in [
            r.paint.normal,
            r.paint.hovered,
            r.paint.pressed,
            r.paint.disabled,
        ] {
            colors(&[p.fill, p.border, p.indicator, p.foreground])?;
        }
        {
            let c = &self.checkbox;
            c.text.validate()?;
            c.focus.validate()?;
            c.unchecked.validate()?;
            c.checked.validate()?;
            lengths(&[c.row_height, c.box_size, c.gap, c.radius, c.border_width])?;
        }
        Ok(())
    }
}
pub(crate) fn lengths(values: &[f32]) -> Result<(), WidgetError> {
    if values.iter().all(|v| v.is_finite() && *v >= 0.0) {
        Ok(())
    } else {
        Err(WidgetError::Invalid(
            "lengths must be finite and nonnegative".into(),
        ))
    }
}
