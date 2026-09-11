use std::{error::Error, path::PathBuf};

use avenger_typst_label::{
    Color, EngineOptions, FontWeight, LabelEngine, LabelFrameItem, LabelOptions, LineCap, LineJoin,
    PathCommand, PathItem, Point, Stroke, SvgOptions, TextItemKind, Transform, svg_items,
};

const PRECISION: usize = 3;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let output = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/typst-label-math-svg-probe/math-label.svg"));
    let font_dir = args.next().map(PathBuf::from).or_else(default_font_dir);

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut engine_options = EngineOptions::default();
    engine_options.fonts.load_system_fonts = true;
    engine_options.fonts.default_sans_serif_family = Some("Lato".to_string());
    engine_options.fonts.default_math_family = Some("Lete Sans Math".to_string());
    if let Some(font_dir) = font_dir {
        engine_options.fonts.extra_font_dirs.push(font_dir);
    }
    let engine = LabelEngine::new(engine_options)?;
    let mut options = LabelOptions::default();
    options.text.font_size = 36.0;
    options.text.font_weight = FontWeight::Number(500);
    options.math.font_size = 36.0;
    options.math.font_weight = FontWeight::Number(500);

    let label = engine.compile(r"$y = sqrt(x) / (1 + x^2)$", &options)?;
    let svg_label = svg_items(&label, &SvgOptions::default())?;
    let (svg, path_count) = svg_document(&svg_label)?;
    std::fs::write(&output, svg.as_bytes())?;

    println!(
        "{} svg_bytes={} paths={} metrics={:.2}x{:.2}",
        output.display(),
        svg.len(),
        path_count,
        label.metrics.width,
        label.metrics.height,
    );
    Ok(())
}

fn svg_document(label: &avenger_typst_label::SvgLabel) -> Result<(String, usize), Box<dyn Error>> {
    let width = label.metrics.width.max(1.0);
    let height = label.metrics.height.max(1.0);
    let mut svg = String::new();
    svg.push_str(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 "#);
    push_number(&mut svg, width, PRECISION)?;
    svg.push(' ');
    push_number(&mut svg, height, PRECISION)?;
    svg.push_str(r#"" width=""#);
    push_number(&mut svg, width, PRECISION)?;
    svg.push_str(r#"" height=""#);
    push_number(&mut svg, height, PRECISION)?;
    svg.push_str(r#"">"#);

    let mut path_count = 0;
    write_items(&mut svg, &label.items, Point::ZERO, &mut path_count)?;
    svg.push_str("</svg>\n");
    Ok((svg, path_count))
}

fn write_items(
    svg: &mut String,
    items: &[(Point, LabelFrameItem)],
    parent_offset: Point,
    path_count: &mut usize,
) -> Result<(), Box<dyn Error>> {
    for (point, item) in items {
        let offset = Point {
            x: parent_offset.x + point.x,
            y: parent_offset.y + point.y,
        };
        match item {
            LabelFrameItem::Shape(shape) => {
                write_path(svg, &shape.item, offset)?;
                *path_count += 1;
            }
            LabelFrameItem::Group(group) => write_items(svg, &group.items, offset, path_count)?,
            LabelFrameItem::Text(text) if text.kind == TextItemKind::Math => {}
            LabelFrameItem::Text(_) => {
                return Err("unexpected native text item in math SVG probe".into());
            }
            LabelFrameItem::Image(_) => {
                return Err("unexpected image item in math SVG probe".into());
            }
        }
    }
    Ok(())
}

fn write_path(svg: &mut String, item: &PathItem, offset: Point) -> Result<(), Box<dyn Error>> {
    let d = path_data(&item.path, item.transform, offset)?;
    if d.is_empty() {
        return Ok(());
    }

    svg.push_str(r#"<path d=""#);
    svg.push_str(&d);
    svg.push('"');
    if let Some(fill) = item.fill {
        push_color_attr(svg, "fill", fill)?;
    } else {
        svg.push_str(r#" fill="none""#);
    }
    if let Some(stroke) = &item.stroke {
        push_stroke_attrs(svg, stroke)?;
    }
    svg.push_str("/>");
    Ok(())
}

fn path_data(
    path: &avenger_typst_label::PathData,
    transform: Transform,
    offset: Point,
) -> Result<String, Box<dyn Error>> {
    let mut d = String::new();
    for command in &path.commands {
        if !d.is_empty() {
            d.push(' ');
        }
        match *command {
            PathCommand::MoveTo { x, y } => {
                d.push('M');
                push_transformed_point(&mut d, transform, offset, x, y)?;
            }
            PathCommand::LineTo { x, y } => {
                d.push('L');
                push_transformed_point(&mut d, transform, offset, x, y)?;
            }
            PathCommand::QuadTo { x1, y1, x, y } => {
                d.push('Q');
                push_transformed_point(&mut d, transform, offset, x1, y1)?;
                d.push(' ');
                push_transformed_point(&mut d, transform, offset, x, y)?;
            }
            PathCommand::CubicTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                d.push('C');
                push_transformed_point(&mut d, transform, offset, x1, y1)?;
                d.push(' ');
                push_transformed_point(&mut d, transform, offset, x2, y2)?;
                d.push(' ');
                push_transformed_point(&mut d, transform, offset, x, y)?;
            }
            PathCommand::Close => d.push('Z'),
        }
    }
    Ok(d)
}

fn push_transformed_point(
    output: &mut String,
    transform: Transform,
    offset: Point,
    x: f32,
    y: f32,
) -> Result<(), Box<dyn Error>> {
    push_number(
        output,
        offset.x + transform.sx * x + transform.kx * y + transform.tx,
        PRECISION,
    )?;
    output.push(' ');
    push_number(
        output,
        offset.y + transform.ky * x + transform.sy * y + transform.ty,
        PRECISION,
    )?;
    Ok(())
}

fn push_stroke_attrs(output: &mut String, stroke: &Stroke) -> Result<(), Box<dyn Error>> {
    push_color_attr(output, "stroke", stroke.color)?;
    output.push_str(r#" stroke-width=""#);
    push_number(output, stroke.width, PRECISION)?;
    output.push('"');
    output.push_str(r#" stroke-linecap=""#);
    output.push_str(match stroke.line_cap {
        LineCap::Butt => "butt",
        LineCap::Round => "round",
        LineCap::Square => "square",
    });
    output.push('"');
    output.push_str(r#" stroke-linejoin=""#);
    output.push_str(match stroke.line_join {
        LineJoin::Bevel => "bevel",
        LineJoin::Miter => "miter",
        LineJoin::Round => "round",
    });
    output.push('"');
    output.push_str(r#" stroke-miterlimit=""#);
    push_number(output, stroke.miter_limit, PRECISION)?;
    output.push('"');
    if let Some(dash) = &stroke.dash
        && !dash.array.is_empty()
    {
        output.push_str(r#" stroke-dashoffset=""#);
        push_number(output, dash.phase, PRECISION)?;
        output.push('"');
        output.push_str(r#" stroke-dasharray=""#);
        for (index, value) in dash.array.iter().enumerate() {
            if index > 0 {
                output.push(' ');
            }
            push_number(output, *value, PRECISION)?;
        }
        output.push('"');
    }
    Ok(())
}

fn push_color_attr(output: &mut String, attr: &str, color: Color) -> Result<(), Box<dyn Error>> {
    output.push(' ');
    output.push_str(attr);
    output.push_str(r#"="rgb("#);
    push_number(output, color.r * 100.0, PRECISION)?;
    output.push_str("% ");
    push_number(output, color.g * 100.0, PRECISION)?;
    output.push_str("% ");
    push_number(output, color.b * 100.0, PRECISION)?;
    output.push_str(r#"%)""#);
    if color.a < 1.0 {
        output.push(' ');
        output.push_str(attr);
        output.push_str(r#"-opacity=""#);
        push_number(output, color.a, PRECISION)?;
        output.push('"');
    }
    Ok(())
}

fn push_number(output: &mut String, value: f32, precision: usize) -> Result<(), Box<dyn Error>> {
    if !value.is_finite() {
        return Err(format!("non-finite SVG coordinate {value}").into());
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
    output.push_str(&s);
    Ok(())
}

fn default_font_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scratch/font-subset-output");
    dir.is_dir().then_some(dir)
}
