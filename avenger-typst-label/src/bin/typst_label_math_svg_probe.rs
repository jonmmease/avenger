//! Measures the label engine's binary size. It is the smallest program that uses the engine end
//! to end: it compiles one math label and writes its drawing items as an SVG document. Built
//! with the workspace's `release-size` profile, its size is the figure in the crate's README.
//! It loads its fonts at runtime from `<font-dir>`, which must hold Lato and Lete Sans Math, so
//! they don't count toward the size.
//!
//! ```sh
//! cargo build --profile release-size -p avenger-typst-label --features size-probe \
//!     --bin typst-label-math-svg-probe
//! target/release-size/typst-label-math-svg-probe <font-dir> [output.svg]
//! ```

use std::fmt::Write;
use std::{error::Error, path::PathBuf};

use avenger_typst_label::{
    CurveItem, EngineOptions, LabelEngine, LabelOptions, LineCap, LineJoin, PathItem,
    SvgItem, SvgLabel, SvgOptions, Transform, svg_items,
};

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let font_dir = args
        .next()
        .map(PathBuf::from)
        .ok_or("usage: typst-label-math-svg-probe <font-dir> [output.svg]")?;
    let output = args.next().map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from("target/typst-label-math-svg-probe/math-label.svg")
    });

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut engine_options = EngineOptions::default();
    engine_options.fonts.default_sans_serif_family = Some("Lato".to_string());
    engine_options.fonts.default_math_family = Some("Lete Sans Math".to_string());
    engine_options.fonts.extra_font_dirs.push(font_dir);
    let engine = LabelEngine::new(engine_options);
    let mut options = LabelOptions::default();
    options.text.font_size = 36.0;

    let label = engine.compile(r"$y = sqrt(x) / (1 + x^2)$", &options)?;
    let svg = svg_document(&svg_items(&label, &SvgOptions::default()))?;
    std::fs::write(&output, svg.as_bytes())?;

    println!(
        "{} svg_bytes={} metrics={:.2}x{:.2}",
        output.display(),
        svg.len(),
        label.metrics.width,
        label.metrics.height,
    );
    Ok(())
}

/// An SVG document of the label's paths. Images (bitmap glyphs) are left out.
fn svg_document(label: &SvgLabel) -> Result<String, Box<dyn Error>> {
    let (width, height) = (label.size.x.max(1.0), label.size.y.max(1.0));
    let mut svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width:.3} {height:.3}" width="{width:.3}" height="{height:.3}">"#
    );
    for item in &label.items {
        if let SvgItem::Path(path) = item {
            write_path(&mut svg, path)?;
        }
    }
    svg.push_str("</svg>\n");
    Ok(svg)
}

fn write_path(svg: &mut String, item: &PathItem) -> Result<(), Box<dyn Error>> {
    let mut d = String::new();
    for segment in &item.path.0 {
        match segment {
            CurveItem::Move(p) => write!(d, "M{:.3} {:.3}", p.x, p.y)?,
            CurveItem::Line(p) => write!(d, "L{:.3} {:.3}", p.x, p.y)?,
            CurveItem::Cubic(a, b, c) => write!(
                d,
                "C{:.3} {:.3} {:.3} {:.3} {:.3} {:.3}",
                a.x, a.y, b.x, b.y, c.x, c.y
            )?,
            CurveItem::Close => d.push('Z'),
        }
    }
    if d.is_empty() {
        return Ok(());
    }
    let Transform { sx, ky, kx, sy, tx, ty } = item.transform;
    write!(svg, r#"<path transform="matrix({sx} {ky} {kx} {sy} {tx} {ty})" d="{d}""#)?;
    match item.fill {
        Some(fill) => write!(svg, r#" fill="{}""#, hex(fill.to_rgba8()))?,
        None => svg.push_str(r#" fill="none""#),
    }
    if let Some(stroke) = &item.stroke {
        let cap = match stroke.cap {
            LineCap::Butt => "butt",
            LineCap::Round => "round",
            LineCap::Square => "square",
        };
        let join = match stroke.join {
            LineJoin::Miter => "miter",
            LineJoin::Round => "round",
            LineJoin::Bevel => "bevel",
        };
        write!(
            svg,
            r#" stroke="{}" stroke-width="{}" stroke-linecap="{cap}" stroke-linejoin="{join}""#,
            hex(stroke.paint.to_rgba8()),
            stroke.thickness
        )?;
    }
    svg.push_str("/>");
    Ok(())
}

fn hex([r, g, b, a]: [u8; 4]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
}
