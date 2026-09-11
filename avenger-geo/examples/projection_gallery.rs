//! Render geographic geometry without a chart or scene graph.
//! Run with an optional output directory (default: target/geo-gallery).

use avenger_geo::ingest::{geojson_to_features, stream_wkb_through};
use avenger_geo::{
    Graticule, LyonPathSink, Projection, ProjectionKind, Projector, Sphere, Streamable,
};
use geo_types::LineString;
use lyon_path::{Event, Path};
use std::error::Error;
use std::fmt::Write;
use std::path::PathBuf;

fn path_data(path: &Path) -> String {
    let mut data = String::new();
    for event in path {
        match event {
            Event::Begin { at } => write!(data, "M{:.3},{:.3}", at.x, at.y).unwrap(),
            Event::Line { to, .. } => write!(data, "L{:.3},{:.3}", to.x, to.y).unwrap(),
            Event::End { close: true, .. } => data.push('Z'),
            Event::End { close: false, .. } => {}
            _ => unreachable!("geographic streams emit line segments"),
        }
    }
    data
}

fn project_path(projector: &Projector, object: &dyn Streamable, fill: bool) -> String {
    let mut sink = if fill {
        LyonPathSink::fill()
    } else {
        LyonPathSink::stroke()
    };
    projector.stream(object, &mut sink);
    path_data(&sink.finish())
}

fn gallery() -> Result<String, Box<dyn Error>> {
    let mut svg = String::from(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="1140" height="740" viewBox="0 0 1140 740">
<rect width="1140" height="740" fill="#f4f6fa"/>
<g font-family="Arial, sans-serif" fill="#172b42">
<text x="32" y="45" font-size="28" font-weight="bold">One geometry, six geographic views</text>
<text x="32" y="74" font-size="15" fill="#52667c">Spherical projection · great-circle resampling · antimeridian clipping · viewport fitting</text>
"##,
    );
    let graticule = Graticule::default().with_step(30.0).try_lines()?;
    let routes = [
        (
            LineString::from(vec![(-0.1, 51.5), (139.7, 35.7)]),
            "#b54786",
        ),
        (
            LineString::from(vec![(-122.4, 37.8), (139.7, 35.7)]),
            "#c66a18",
        ),
    ];
    // RFC 7946 winding is converted to the spherical convention during ingest.
    let features = geojson_to_features(
        r#"{"type":"Polygon","coordinates":[[[10,10],[40,10],[40,40],[10,40],[10,10]]]}"#,
    )?;
    let wkb = features[0].wkb.as_ref().ok_or("missing polygon")?;
    let panels = [
        (
            "Equal Earth",
            "Equal-area world view",
            ProjectionKind::EqualEarth,
            [0.0, 0.0, 0.0],
        ),
        (
            "Mercator",
            "World-square clipping at the poles",
            ProjectionKind::Mercator,
            [0.0, 0.0, 0.0],
        ),
        (
            "Natural Earth",
            "Curved meridians",
            ProjectionKind::NaturalEarth1,
            [0.0, 0.0, 0.0],
        ),
        (
            "Winkel tripel",
            "Compromise world projection",
            ProjectionKind::WinkelTripel,
            [0.0, 0.0, 0.0],
        ),
        (
            "Conic equal-area",
            "Standard parallels: 29.5° and 45.5°",
            ProjectionKind::albers(),
            [96.0, 0.0, 0.0],
        ),
        (
            "Rotated Equal Earth",
            "Three-axis rotation: 96°, −30°, 15°",
            ProjectionKind::EqualEarth,
            [96.0, -30.0, 15.0],
        ),
    ];
    for (index, (title, subtitle, kind, rotation)) in panels.into_iter().enumerate() {
        let x = 24 + (index % 3) * 372;
        let y = 100 + (index / 3) * 286;
        write!(
            svg,
            r##"<g transform="translate({x},{y})"><rect width="348" height="266" rx="12" fill="white" stroke="#dbe2ec"/>
<text x="18" y="30" font-size="18" font-weight="bold">{title}</text><text x="18" y="52" font-size="12" fill="#52667c">{subtitle}</text>"##
        )?;
        let mut config = Projection::new(kind)
            .with_rotate(rotation)
            .with_precision(0.2);
        config.fit_extent([[18.0, 69.0], [330.0, 248.0]], &Sphere)?;
        let projector = config.build();
        let outline = project_path(&projector, &Sphere, true);
        write!(
            svg,
            r##"<path d="{outline}" fill="#edf4fa" stroke="#7292ad" stroke-width="1"/>"##
        )?;
        let grid = project_path(&projector, &graticule, false);
        write!(
            svg,
            r##"<path d="{grid}" fill="none" stroke="#b5c9dc" stroke-width="0.65"/>"##
        )?;
        let mut polygon = LyonPathSink::fill();
        stream_wkb_through(&projector, wkb, &mut polygon)?;
        write!(
            svg,
            r##"<path d="{}" fill="#35a997" fill-opacity="0.55" stroke="#148675" stroke-width="1.2"/>"##,
            path_data(&polygon.finish())
        )?;
        for (line, color) in &routes {
            write!(
                svg,
                r##"<path d="{}" fill="none" stroke="{color}" stroke-width="2.3" stroke-linecap="round"/>"##,
                project_path(&projector, line, false)
            )?;
            for point in line.points() {
                if let Some((x, y)) = projector.project(point.x(), point.y()) {
                    write!(
                        svg,
                        r##"<circle cx="{x:.3}" cy="{y:.3}" r="3" fill="{color}" stroke="white" stroke-width="1"/>"##
                    )?;
                }
            }
        }
        svg.push_str("</g>\n");
    }
    svg.push_str(r##"<g font-size="13"><path d="M32,702h24" stroke="#b54786" stroke-width="3"/><text x="64" y="707">London–Tokyo</text><path d="M230,702h24" stroke="#c66a18" stroke-width="3"/><text x="262" y="707">San Francisco–Tokyo (crosses ±180°)</text><rect x="570" y="694" width="18" height="16" fill="#35a997" fill-opacity="0.55"/><text x="598" y="707">Illustrative GeoJSON polygon</text></g></g></svg>"##);
    Ok(svg)
}

fn main() -> Result<(), Box<dyn Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/geo-gallery"));
    std::fs::create_dir_all(&output)?;
    let svg = gallery()?;
    std::fs::write(output.join("projections.svg"), &svg)?;
    let mut options = resvg::usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    let tree = resvg::usvg::Tree::from_str(&svg, &options)?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(2280, 1480).ok_or("image allocation failed")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(2.0, 2.0),
        &mut pixmap.as_mut(),
    );
    pixmap.save_png(output.join("projections.png"))?;
    println!(
        "Wrote projections.svg and projections.png to {}",
        output.display()
    );
    Ok(())
}
