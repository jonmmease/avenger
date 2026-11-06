//! Colors: the functions of CSS Color 4 through `cssparser-color`, `color-mix()`,
//! `contrast-color()`, `currentColor`, and the system colors.

use avenger_color::{AbsoluteColor, ColorSpace, HueInterpolationMethod};
use cssparser::{color::PredefinedColorSpace, ParseError, Parser, Token};
use cssparser_color::{AngleOrNumber, ColorParser, FromParsedColor, NumberOrPercentage};

use super::calc::{parse_numeric, Category};

/// A specified color, which the cascade resolves to an absolute color.
#[derive(Clone, Debug, PartialEq)]
pub enum Color {
    /// A color in a color space, normalized as CSS Color 4 does at parse time, with the
    /// components and the alpha that it writes as `none`, in the order of its components. A
    /// missing component draws as zero, and `color-mix()` takes the other color's value for it.
    Absolute {
        color: AbsoluteColor,
        missing: [bool; 4],
    },
    /// The computed `color` of the node.
    CurrentColor,
    System(SystemColor),
    /// `color-mix(in <space> [<hue> hue], <color> [<percentage>], <color> [<percentage>])`, with
    /// each percentage as a fraction.
    Mix {
        space: ColorSpace,
        hue: HueInterpolationMethod,
        colors: Box<[(Color, Option<f32>); 2]>,
    },
    /// `contrast-color(<color>)`: white or black, whichever contrasts more with the color.
    Contrast(Box<Color>),
}

/// The system colors of CSS Color 4, whose values the host gives with its preferences.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SystemColor {
    Canvas,
    CanvasText,
    Highlight,
    HighlightText,
    GrayText,
    LinkText,
}

impl SystemColor {
    pub const ALL: [SystemColor; 6] = [
        SystemColor::Canvas,
        SystemColor::CanvasText,
        SystemColor::Highlight,
        SystemColor::HighlightText,
        SystemColor::GrayText,
        SystemColor::LinkText,
    ];

    pub fn name(self) -> &'static str {
        match self {
            SystemColor::Canvas => "Canvas",
            SystemColor::CanvasText => "CanvasText",
            SystemColor::Highlight => "Highlight",
            SystemColor::HighlightText => "HighlightText",
            SystemColor::GrayText => "GrayText",
            SystemColor::LinkText => "LinkText",
        }
    }

    fn from_name(name: &str) -> Option<SystemColor> {
        SystemColor::ALL
            .into_iter()
            .find(|color| color.name().eq_ignore_ascii_case(name))
    }
}

impl Color {
    /// Whether the color reads the node's `color`, so it resolves only after `color` does.
    pub fn reads_current_color(&self) -> bool {
        match self {
            Color::CurrentColor => true,
            Color::Mix { colors, .. } => {
                colors.iter().any(|(color, _)| color.reads_current_color())
            }
            Color::Contrast(color) => color.reads_current_color(),
            Color::Absolute { .. } | Color::System(_) => false,
        }
    }
}

/// Parses one color.
pub(crate) fn parse_color(input: &mut Parser<'_>) -> Result<Color, ()> {
    let state = input.state();
    match input.next().map_err(|_| ())?.clone() {
        Token::Ident(name) => {
            if let Some(system) = SystemColor::from_name(&name) {
                return Ok(Color::System(system));
            }
        }
        Token::Function(name) if name.eq_ignore_ascii_case("color-mix") => {
            return input
                .parse_nested_block(|input| mix(input).map_err(|()| ParseError::custom(())))
                .map_err(|_: ParseError<()>| ());
        }
        Token::Function(name)
            if name.eq_ignore_ascii_case("rgb") || name.eq_ignore_ascii_case("rgba") =>
        {
            return input
                .parse_nested_block(|input| rgb(input).map_err(|()| ParseError::custom(())))
                .map_err(|_: ParseError<()>| ());
        }
        Token::Function(name) if name.eq_ignore_ascii_case("contrast-color") => {
            return input
                .parse_nested_block(|input| {
                    let color = parse_color(input).map_err(|()| ParseError::custom(()))?;
                    input.expect_exhausted()?;
                    Ok(Color::Contrast(Box::new(color)))
                })
                .map_err(|_: ParseError<()>| ());
        }
        _ => {}
    }
    input.reset(&state);
    match cssparser_color::parse_color_with(&Components, input) {
        Ok(Parsed(Some(color))) => Ok(color),
        _ => Err(()),
    }
}

/// The arguments of `rgb()` and `rgba()`, which `cssparser-color` would round to bytes and whose
/// `none` it would drop: three channels as numbers from 0 to 255 or as percentages, separated by
/// spaces with an optional `/ <alpha>`, or by commas without `none` in the legacy syntax, whose
/// channels are all numbers or all percentages.
fn rgb(input: &mut Parser<'_>) -> Result<Color, ()> {
    let none = |input: &mut Parser<'_>| {
        input
            .try_parse(|input| input.expect_ident_matching("none"))
            .is_ok()
    };
    // A channel as a fraction, with whether it is a percentage.
    let channel = |input: &mut Parser<'_>| -> Result<Option<(f32, bool)>, ()> {
        if none(input) {
            return Ok(None);
        }
        let parsed = Components.parse_number_or_percentage(input);
        Ok(Some(match parsed.map_err(|_| ())? {
            NumberOrPercentage::Number { value } => ((value / 255.0).clamp(0.0, 1.0), false),
            NumberOrPercentage::Percentage { unit_value } => (unit_value.clamp(0.0, 1.0), true),
        }))
    };
    let alpha = |input: &mut Parser<'_>| -> Result<Option<f32>, ()> {
        if none(input) {
            return Ok(None);
        }
        let parsed = Components.parse_number_or_percentage(input);
        Ok(Some(match parsed.map_err(|_| ())? {
            NumberOrPercentage::Number { value } => value,
            NumberOrPercentage::Percentage { unit_value } => unit_value,
        }))
    };
    let first = channel(input)?;
    let legacy = first.is_some() && input.try_parse(|input| input.expect_comma()).is_ok();
    let (second, third, opacity) = match legacy {
        true => {
            let second = channel(input)?.ok_or(())?;
            input.expect_comma().map_err(|_| ())?;
            let third = channel(input)?.ok_or(())?;
            let percentage = first.map(|(_, percentage)| percentage);
            if Some(second.1) != percentage || Some(third.1) != percentage {
                return Err(());
            }
            let opacity = match input.try_parse(|input| input.expect_comma()) {
                Ok(()) => Some(alpha(input)?.ok_or(())?),
                Err(_) => Some(1.0),
            };
            (Some(second), Some(third), opacity)
        }
        false => {
            let second = channel(input)?;
            let third = channel(input)?;
            let opacity = match input.try_parse(|input| input.expect_delim('/')) {
                Ok(()) => alpha(input)?,
                Err(_) => Some(1.0),
            };
            (second, third, opacity)
        }
    };
    input.expect_exhausted().map_err(|_| ())?;
    let value = |channel: Option<(f32, bool)>| channel.map(|(value, _)| value);
    match absolute(
        ColorSpace::Srgb,
        [value(first), value(second), value(third)],
        opacity,
    ) {
        Parsed(Some(color)) => Ok(color),
        Parsed(None) => Err(()),
    }
}

/// The arguments of `color-mix()`.
fn mix(input: &mut Parser<'_>) -> Result<Color, ()> {
    input.expect_ident_matching("in").map_err(|_| ())?;
    let space = match input
        .expect_ident()
        .map_err(|_| ())?
        .to_ascii_lowercase()
        .as_str()
    {
        "srgb" => ColorSpace::Srgb,
        "hsl" => ColorSpace::Hsl,
        "hwb" => ColorSpace::Hwb,
        "lab" => ColorSpace::Lab,
        "lch" => ColorSpace::Lch,
        "oklab" => ColorSpace::Oklab,
        "oklch" => ColorSpace::Oklch,
        _ => return Err(()),
    };
    let hue = input
        .try_parse(|input| {
            let method = match input.expect_ident()?.to_ascii_lowercase().as_str() {
                "shorter" => HueInterpolationMethod::Shorter,
                "longer" => HueInterpolationMethod::Longer,
                "increasing" => HueInterpolationMethod::Increasing,
                "decreasing" => HueInterpolationMethod::Decreasing,
                _ => return Err(ParseError::<()>::custom(())),
            };
            input.expect_ident_matching("hue")?;
            Ok(method)
        })
        .unwrap_or(HueInterpolationMethod::Shorter);
    if hue != HueInterpolationMethod::Shorter && !space.is_polar() {
        return Err(());
    }
    let mut colors = Vec::with_capacity(2);
    for _ in 0..2 {
        input.expect_comma().map_err(|_| ())?;
        let percentage = |input: &mut Parser<'_>| -> Option<f32> {
            input
                .try_parse(|input| match parse_numeric(input) {
                    Ok(calc) if calc.category() == Some(Category::Percentage) => {
                        calc.constant().ok_or(())
                    }
                    _ => Err(()),
                })
                .ok()
        };
        let before = percentage(input);
        let color = parse_color(input)?;
        let after = percentage(input);
        let weight = match (before, after) {
            (Some(_), Some(_)) => return Err(()),
            (weight, None) | (None, weight) => weight,
        };
        if weight.is_some_and(|weight| !(0.0..=1.0).contains(&weight)) {
            return Err(());
        }
        colors.push((color, weight));
    }
    input.expect_exhausted().map_err(|_| ())?;
    let [first, second]: [(Color, Option<f32>); 2] = colors.try_into().map_err(|_| ())?;
    Ok(Color::Mix {
        space,
        hue,
        colors: Box::new([first, second]),
    })
}

/// The color that `cssparser-color` builds, or `None` for a color space that avenger-color does
/// not convert.
struct Parsed(Option<Color>);

/// A color from its components and its alpha, `None` for `none`, with the normalization of CSS
/// Color 4 at parse time: the saturation of HSL and the chroma of LCH and OKLCH are at least 0,
/// the lightness of Lab, LCH, OKLab, and OKLCH is within its range, and the alpha is from 0 to 1.
fn absolute(space: ColorSpace, components: [Option<f32>; 3], alpha: Option<f32>) -> Parsed {
    let missing = [
        components[0].is_none(),
        components[1].is_none(),
        components[2].is_none(),
        alpha.is_none(),
    ];
    let [mut c0, mut c1, mut c2] = components.map(|component| component.unwrap_or(0.0));
    if matches!(space, ColorSpace::Hsl | ColorSpace::Hwb) {
        // `cssparser-color` gives the percentages of HSL and HWB as fractions.
        c1 *= 100.0;
        c2 *= 100.0;
    }
    match space {
        ColorSpace::Hsl => c1 = c1.max(0.0),
        ColorSpace::Lab => c0 = c0.clamp(0.0, 100.0),
        ColorSpace::Lch => {
            c0 = c0.clamp(0.0, 100.0);
            c1 = c1.max(0.0);
        }
        ColorSpace::Oklab => c0 = c0.clamp(0.0, 1.0),
        ColorSpace::Oklch => {
            c0 = c0.clamp(0.0, 1.0);
            c1 = c1.max(0.0);
        }
        _ => {}
    }
    let alpha = alpha.unwrap_or(0.0).clamp(0.0, 1.0);
    Parsed(Some(Color::Absolute {
        color: AbsoluteColor::new(space, c0, c1, c2, alpha),
        missing,
    }))
}

impl FromParsedColor for Parsed {
    fn from_current_color() -> Self {
        Parsed(Some(Color::CurrentColor))
    }

    fn from_rgba(red: u8, green: u8, blue: u8, alpha: f32) -> Self {
        let channel = |value: u8| value as f32 / 255.0;
        Parsed(Some(Color::Absolute {
            color: AbsoluteColor::from_srgb(channel(red), channel(green), channel(blue), alpha),
            missing: [false; 4],
        }))
    }

    fn from_hsl(
        hue: Option<f32>,
        saturation: Option<f32>,
        lightness: Option<f32>,
        alpha: Option<f32>,
    ) -> Self {
        absolute(ColorSpace::Hsl, [hue, saturation, lightness], alpha)
    }

    fn from_hwb(
        hue: Option<f32>,
        whiteness: Option<f32>,
        blackness: Option<f32>,
        alpha: Option<f32>,
    ) -> Self {
        absolute(ColorSpace::Hwb, [hue, whiteness, blackness], alpha)
    }

    fn from_lab(
        lightness: Option<f32>,
        a: Option<f32>,
        b: Option<f32>,
        alpha: Option<f32>,
    ) -> Self {
        absolute(ColorSpace::Lab, [lightness, a, b], alpha)
    }

    fn from_lch(
        lightness: Option<f32>,
        chroma: Option<f32>,
        hue: Option<f32>,
        alpha: Option<f32>,
    ) -> Self {
        absolute(ColorSpace::Lch, [lightness, chroma, hue], alpha)
    }

    fn from_oklab(
        lightness: Option<f32>,
        a: Option<f32>,
        b: Option<f32>,
        alpha: Option<f32>,
    ) -> Self {
        absolute(ColorSpace::Oklab, [lightness, a, b], alpha)
    }

    fn from_oklch(
        lightness: Option<f32>,
        chroma: Option<f32>,
        hue: Option<f32>,
        alpha: Option<f32>,
    ) -> Self {
        absolute(ColorSpace::Oklch, [lightness, chroma, hue], alpha)
    }

    fn from_color_function(
        color_space: PredefinedColorSpace,
        c1: Option<f32>,
        c2: Option<f32>,
        c3: Option<f32>,
        alpha: Option<f32>,
    ) -> Self {
        match color_space {
            PredefinedColorSpace::Srgb => absolute(ColorSpace::Srgb, [c1, c2, c3], alpha),
            _ => Parsed(None),
        }
    }
}

/// The components of colors, which can be `calc()` expressions of numbers.
struct Components;

impl<'i> ColorParser<'i> for Components {
    type Output = Parsed;
    type Error = ();

    fn parse_angle_or_number(
        &self,
        input: &mut Parser<'_>,
    ) -> Result<AngleOrNumber, ParseError<Self::Error>> {
        let calc = parse_numeric(input).map_err(|_| ParseError::custom(()))?;
        let value = calc.constant().ok_or_else(|| ParseError::custom(()))?;
        match calc.category() {
            Some(Category::Number) => Ok(AngleOrNumber::Number { value }),
            Some(Category::Angle) => Ok(AngleOrNumber::Angle { degrees: value }),
            _ => Err(ParseError::custom(())),
        }
    }

    fn parse_percentage(&self, input: &mut Parser<'_>) -> Result<f32, ParseError<Self::Error>> {
        let calc = parse_numeric(input).map_err(|_| ParseError::custom(()))?;
        match calc.category() {
            Some(Category::Percentage) => calc.constant().ok_or_else(|| ParseError::custom(())),
            _ => Err(ParseError::custom(())),
        }
    }

    fn parse_number(&self, input: &mut Parser<'_>) -> Result<f32, ParseError<Self::Error>> {
        let calc = parse_numeric(input).map_err(|_| ParseError::custom(()))?;
        match calc.category() {
            Some(Category::Number) => calc.constant().ok_or_else(|| ParseError::custom(())),
            _ => Err(ParseError::custom(())),
        }
    }

    fn parse_number_or_percentage(
        &self,
        input: &mut Parser<'_>,
    ) -> Result<NumberOrPercentage, ParseError<Self::Error>> {
        let calc = parse_numeric(input).map_err(|_| ParseError::custom(()))?;
        let value = calc.constant().ok_or_else(|| ParseError::custom(()))?;
        match calc.category() {
            Some(Category::Number) => Ok(NumberOrPercentage::Number { value }),
            Some(Category::Percentage) => Ok(NumberOrPercentage::Percentage { unit_value: value }),
            _ => Err(ParseError::custom(())),
        }
    }
}
