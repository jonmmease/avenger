use lyon_path::{Event, Path};

use crate::error::AvengerSvgError;

pub fn lyon_path_to_svg_d(path: &Path, precision: usize) -> Result<String, AvengerSvgError> {
    let mut d = String::new();

    for event in path.iter() {
        if !d.is_empty() {
            d.push(' ');
        }

        match event {
            Event::Begin { at } => {
                d.push('M');
                push_point(&mut d, at.x, at.y, precision)?;
            }
            Event::Line { to, .. } => {
                d.push('L');
                push_point(&mut d, to.x, to.y, precision)?;
            }
            Event::Quadratic { ctrl, to, .. } => {
                d.push('Q');
                push_point(&mut d, ctrl.x, ctrl.y, precision)?;
                d.push(' ');
                push_point(&mut d, to.x, to.y, precision)?;
            }
            Event::Cubic {
                ctrl1, ctrl2, to, ..
            } => {
                d.push('C');
                push_point(&mut d, ctrl1.x, ctrl1.y, precision)?;
                d.push(' ');
                push_point(&mut d, ctrl2.x, ctrl2.y, precision)?;
                d.push(' ');
                push_point(&mut d, to.x, to.y, precision)?;
            }
            Event::End { close: true, .. } => {
                d.push('Z');
            }
            Event::End { close: false, .. } => {
                d.pop();
            }
        }
    }

    Ok(d)
}

pub fn format_number(value: f32, precision: usize) -> Result<String, AvengerSvgError> {
    if !value.is_finite() {
        return Err(AvengerSvgError::InvalidGeometry(format!(
            "non-finite coordinate {value}"
        )));
    }

    let mut s = format!("{value:.precision$}");
    while s.contains('.') && s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    if s == "-0" {
        s = "0".to_string();
    }
    Ok(s)
}

pub fn push_number(
    output: &mut String,
    value: f32,
    precision: usize,
) -> Result<(), AvengerSvgError> {
    output.push_str(&format_number(value, precision)?);
    Ok(())
}

pub fn push_point(
    output: &mut String,
    x: f32,
    y: f32,
    precision: usize,
) -> Result<(), AvengerSvgError> {
    push_number(output, x, precision)?;
    output.push(' ');
    push_number(output, y, precision)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use lyon_path::Path;

    use super::*;

    #[test]
    fn formats_numbers_deterministically() {
        assert_eq!(format_number(1.23456, 3).unwrap(), "1.235");
        assert_eq!(format_number(1.200, 3).unwrap(), "1.2");
        assert_eq!(format_number(-0.0001, 3).unwrap(), "0");
        assert_eq!(format_number(5.0, 3).unwrap(), "5");
    }

    #[test]
    fn serializes_basic_path_events() {
        let mut builder = Path::builder();
        builder.begin(lyon_path::math::point(0.0, 1.0));
        builder.line_to(lyon_path::math::point(2.0, 3.0));
        builder.quadratic_bezier_to(
            lyon_path::math::point(4.0, 5.0),
            lyon_path::math::point(6.0, 7.0),
        );
        builder.cubic_bezier_to(
            lyon_path::math::point(8.0, 9.0),
            lyon_path::math::point(10.0, 11.0),
            lyon_path::math::point(12.0, 13.0),
        );
        builder.close();

        let path = builder.build();
        assert_eq!(
            lyon_path_to_svg_d(&path, 3).unwrap(),
            "M0 1 L2 3 Q4 5 6 7 C8 9 10 11 12 13 Z"
        );
    }
}
