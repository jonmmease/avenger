use avenger_color::ColorOrGradient;
use avenger_common::types::{StrokeCap, StrokeJoin};

use crate::{error::AvengerSvgError, path::format_number};

pub trait PaintResolver {
    fn push_paint_attrs(
        &mut self,
        output: &mut String,
        attr: &str,
        paint: &ColorOrGradient,
        precision: usize,
    ) -> Result<(), AvengerSvgError>;
}

pub fn escape_text(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn escape_attr(input: &str) -> String {
    escape_text(input).replace('"', "&quot;")
}

pub fn push_fill_attrs(
    output: &mut String,
    fill: Option<&ColorOrGradient>,
    resolver: &mut impl PaintResolver,
    precision: usize,
) -> Result<(), AvengerSvgError> {
    match fill {
        Some(fill) => resolver.push_paint_attrs(output, "fill", fill, precision),
        None => {
            output.push_str(r#" fill="none""#);
            Ok(())
        }
    }
}

pub fn push_stroke_attrs(
    output: &mut String,
    stroke: Option<&ColorOrGradient>,
    stroke_width: Option<f32>,
    stroke_cap: Option<StrokeCap>,
    stroke_join: Option<StrokeJoin>,
    stroke_dash: Option<&[f32]>,
    resolver: &mut impl PaintResolver,
    precision: usize,
) -> Result<(), AvengerSvgError> {
    let width = stroke_width.unwrap_or(0.0);
    if stroke.is_none() || width <= 0.0 {
        output.push_str(r#" stroke="none""#);
        return Ok(());
    }

    resolver.push_paint_attrs(output, "stroke", stroke.unwrap(), precision)?;
    output.push_str(r#" stroke-width=""#);
    output.push_str(&format_number(width, precision)?);
    output.push('"');

    if let Some(cap) = stroke_cap {
        output.push_str(r#" stroke-linecap=""#);
        output.push_str(match cap {
            StrokeCap::Butt => "butt",
            StrokeCap::Round => "round",
            StrokeCap::Square => "square",
        });
        output.push('"');
    }

    if let Some(join) = stroke_join {
        output.push_str(r#" stroke-linejoin=""#);
        output.push_str(match join {
            StrokeJoin::Bevel => "bevel",
            StrokeJoin::Miter => "miter",
            StrokeJoin::Round => "round",
        });
        output.push('"');
    }

    if let Some(dash) = stroke_dash {
        if !dash.is_empty() {
            output.push_str(r#" stroke-dasharray=""#);
            for (index, value) in dash.iter().enumerate() {
                if index > 0 {
                    output.push(' ');
                }
                output.push_str(&format_number(*value, precision)?);
            }
            output.push('"');
        }
    }

    Ok(())
}

pub fn push_color_only_paint_attrs(
    output: &mut String,
    attr: &str,
    paint: &ColorOrGradient,
    precision: usize,
) -> Result<(), AvengerSvgError> {
    match paint {
        ColorOrGradient::Color(color) => push_color_attrs(output, attr, *color, precision),
        ColorOrGradient::GradientIndex(index) => Err(AvengerSvgError::UnsupportedPaint(format!(
            "gradient index {index}"
        ))),
    }
}

pub fn push_color_attrs(
    output: &mut String,
    attr: &str,
    color: [f32; 4],
    precision: usize,
) -> Result<(), AvengerSvgError> {
    let [r, g, b, a] = color.map(|v| v.clamp(0.0, 1.0));
    if a <= 0.0 {
        output.push(' ');
        output.push_str(attr);
        output.push_str(r#"="none""#);
        return Ok(());
    }

    output.push(' ');
    output.push_str(attr);
    output.push_str("=\"#");
    output.push_str(&hex_byte(r));
    output.push_str(&hex_byte(g));
    output.push_str(&hex_byte(b));
    output.push('"');

    if a < 1.0 {
        output.push(' ');
        output.push_str(attr);
        output.push_str(r#"-opacity=""#);
        output.push_str(&format_number(a, precision)?);
        output.push('"');
    }

    Ok(())
}

pub fn push_stop_color_attrs(
    output: &mut String,
    color: [f32; 4],
    precision: usize,
) -> Result<(), AvengerSvgError> {
    let [r, g, b, a] = color.map(|v| v.clamp(0.0, 1.0));
    output.push_str(r#" stop-color=""#);
    output.push_str(&color_to_hex([r, g, b]));
    output.push('"');

    if a < 1.0 {
        output.push_str(r#" stop-opacity=""#);
        output.push_str(&format_number(a, precision)?);
        output.push('"');
    }

    Ok(())
}

pub fn color_to_hex(rgb: [f32; 3]) -> String {
    let [r, g, b] = rgb.map(|v| v.clamp(0.0, 1.0));
    format!("#{}{}{}", hex_byte(r), hex_byte(g), hex_byte(b))
}

fn hex_byte(value: f32) -> String {
    format!("{:02x}", (value * 255.0).round() as u8)
}

pub struct ColorOnlyPaintResolver;

impl PaintResolver for ColorOnlyPaintResolver {
    fn push_paint_attrs(
        &mut self,
        output: &mut String,
        attr: &str,
        paint: &ColorOrGradient,
        precision: usize,
    ) -> Result<(), AvengerSvgError> {
        push_color_only_paint_attrs(output, attr, paint, precision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_text_and_attributes() {
        assert_eq!(escape_text("<a&b>"), "&lt;a&amp;b&gt;");
        assert_eq!(escape_attr("\"<a&b>\""), "&quot;&lt;a&amp;b&gt;&quot;");
    }

    #[test]
    fn formats_color_and_opacity_attrs() {
        let mut out = String::new();
        push_color_attrs(&mut out, "fill", [1.0, 0.5, 0.0, 0.25], 3).unwrap();
        assert_eq!(out, r##" fill="#ff8000" fill-opacity="0.25""##);
    }

    #[test]
    fn transparent_color_becomes_none() {
        let mut out = String::new();
        push_color_attrs(&mut out, "stroke", [1.0, 0.0, 0.0, 0.0], 3).unwrap();
        assert_eq!(out, r#" stroke="none""#);
    }

    #[test]
    fn formats_stop_colors_without_none() {
        let mut out = String::new();
        push_stop_color_attrs(&mut out, [1.0, 0.5, 0.0, 0.25], 3).unwrap();
        assert_eq!(out, r##" stop-color="#ff8000" stop-opacity="0.25""##);
    }
}
