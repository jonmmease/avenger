use crate::{error::AvengerSvgError, path::format_number};

pub fn escape_text(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn escape_attr(input: &str) -> String {
    escape_text(input).replace('"', "&quot;")
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
    output.push_str(r##" stop-color="#"##);
    output.push_str(&hex_byte(r));
    output.push_str(&hex_byte(g));
    output.push_str(&hex_byte(b));
    output.push('"');

    if a < 1.0 {
        output.push_str(r#" stop-opacity=""#);
        output.push_str(&format_number(a, precision)?);
        output.push('"');
    }

    Ok(())
}

fn hex_byte(value: f32) -> String {
    format!("{:02x}", (value * 255.0).round() as u8)
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
