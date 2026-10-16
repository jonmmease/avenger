//! Draw maps as an Avenger scene graph, with the sphere and labeled graticule
//! from this crate, and export them with `avenger-svg`. Run with an optional
//! output directory (default: target/geo-gallery).

use avenger_color::ColorOrGradient;
use avenger_common::types::{FontWeight, StrokeCap, SymbolShape, TextBaseline};
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_geo::ingest::geojson_to_features;
use avenger_geo::projections::{ConicEqualArea, EqualEarth, Mercator, NaturalEarth1, WinkelTripel};
use avenger_geo::{Graticule, Projection, Projector};
use avenger_guides_geo::{
    make_graticule_marks, make_sphere_marks, GraticuleConfig, GraticuleStyle, MapSide, SphereStyle,
};
use avenger_scenegraph::marks::{
    group::SceneGroup, mark::SceneMark, path::ScenePathMark, rect::SceneRectMark,
    rule::SceneRuleMark, symbol::SceneSymbolMark, text::SceneTextMark,
};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::{EngineOptions, FontOptions, LabelEngine};
use geo_traits::GeometryTrait;
use geo_types::LineString;
use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;

const INK: [f32; 4] = rgb(0x172b42);
const MUTED: [f32; 4] = rgb(0x52667c);

const fn rgb(hex: u32) -> [f32; 4] {
    [
        ((hex >> 16) & 0xff) as f32 / 255.0,
        ((hex >> 8) & 0xff) as f32 / 255.0,
        (hex & 0xff) as f32 / 255.0,
        1.0,
    ]
}

fn with_alpha([r, g, b, _]: [f32; 4], alpha: f32) -> [f32; 4] {
    [r, g, b, alpha]
}

fn text(source: &str, x: f32, y: f32, size: f32, weight: FontWeight, color: [f32; 4]) -> SceneMark {
    SceneTextMark {
        text: source.to_string().into(),
        x: x.into(),
        y: y.into(),
        baseline: TextBaseline::Alphabetic.into(),
        font: "Lato".to_string().into(),
        font_size: size.into(),
        font_weight: weight.into(),
        color: color.into(),
        ..Default::default()
    }
    .into()
}

/// A path mark that draws `geometry` through `projector`.
fn projected(
    projector: &Projector,
    geometry: &impl GeometryTrait<T = f64>,
    fill: Option<[f32; 4]>,
    stroke: [f32; 4],
    stroke_width: f32,
) -> SceneMark {
    ScenePathMark {
        path: projector.project_to_path(geometry).into(),
        fill: ColorOrGradient::Color(fill.unwrap_or([0.0; 4])).into(),
        stroke: ColorOrGradient::Color(stroke).into(),
        stroke_width: Some(stroke_width),
        stroke_cap: StrokeCap::Round,
        ..Default::default()
    }
    .into()
}

fn gallery(engine: &LabelEngine) -> Result<SceneGraph, Box<dyn Error>> {
    // Meridians run pole to pole, so that their labels line up along the
    // bottom of each map.
    let default = Graticule::default().with_step(30.0);
    let graticule = Graticule {
        extent_minor: default.extent_major,
        ..default
    };
    let sphere_style = SphereStyle {
        fill: rgb(0xedf4fa),
        stroke: rgb(0x7292ad),
        stroke_width: 1.0,
    };
    let graticule_style = GraticuleStyle {
        stroke: rgb(0xb5c9dc),
        stroke_width: 0.65,
        label_color: MUTED,
        label_font_family: "Lato".to_string(),
        label_font_size: 8.0,
        ..Default::default()
    };
    let routes = [
        (
            LineString::from(vec![(-0.1, 51.5), (139.7, 35.7)]),
            rgb(0xb54786),
        ),
        (
            LineString::from(vec![(-122.4, 37.8), (139.7, 35.7)]),
            rgb(0xc66a18),
        ),
    ];
    let features = geojson_to_features(
        r#"{"type":"Polygon","coordinates":[[[10,10],[40,10],[40,40],[10,40],[10,10]]]}"#,
    )?;
    let polygon = wkb::reader::read_wkb(features[0].wkb.as_ref().ok_or("missing polygon")?)?;
    // North-up world maps label parallels on the left and meridians along the
    // bottom. The conic's fan and the rotated view draw lines only.
    let labeled = vec![MapSide::Left, MapSide::Bottom];
    let panels = [
        (
            "Equal Earth",
            "Equal-area world view",
            Projection::new(EqualEarth),
            [0.0, 0.0, 0.0],
            labeled.clone(),
        ),
        (
            "Mercator",
            "World-square clipping at the poles",
            Projection::new(Mercator),
            [0.0, 0.0, 0.0],
            labeled.clone(),
        ),
        (
            "Natural Earth",
            "Curved meridians",
            Projection::new(NaturalEarth1),
            [0.0, 0.0, 0.0],
            labeled.clone(),
        ),
        (
            "Winkel tripel",
            "Compromise world projection",
            Projection::new(WinkelTripel),
            [0.0, 0.0, 0.0],
            labeled.clone(),
        ),
        (
            "Conic equal-area",
            "Standard parallels: 29.5° and 45.5°",
            Projection::new(ConicEqualArea::albers()),
            [96.0, 0.0, 0.0],
            Vec::new(),
        ),
        (
            "Rotated Equal Earth",
            "Three-axis rotation: 96°, −30°, 15°",
            Projection::new(EqualEarth),
            [96.0, -30.0, 15.0],
            Vec::new(),
        ),
    ];

    let mut marks: Vec<SceneMark> = vec![
        SceneRectMark {
            width: Some(1140.0.into()),
            height: Some(740.0.into()),
            fill: ColorOrGradient::Color(rgb(0xf4f6fa)).into(),
            ..Default::default()
        }
        .into(),
        text("One geometry, six geographic views", 32.0, 45.0, 28.0, FontWeight::BOLD, INK),
        text(
            "Spherical projection · great-circle resampling · antimeridian clipping · viewport fitting",
            32.0,
            74.0,
            15.0,
            FontWeight::NORMAL,
            MUTED,
        ),
    ];
    for (index, (title, subtitle, projection, rotation, label_sides)) in
        panels.into_iter().enumerate()
    {
        let mut projection = projection.with_rotate(rotation).with_precision(0.2);
        // The margins on the left and bottom hold the graticule's labels.
        let summary = projection.summarize_sphere()?;
        projection.fit([[44.0, 66.0], [330.0, 238.0]], &summary)?;
        let projector = projection.build()?;
        let graticule = GraticuleConfig {
            graticule: graticule.clone(),
            label_sides,
            style: graticule_style.clone(),
            ..Default::default()
        };
        let mut panel = vec![
            SceneRectMark {
                width: Some(348.0.into()),
                height: Some(266.0.into()),
                corner_radius: 12.0.into(),
                fill: ColorOrGradient::Color([1.0; 4]).into(),
                stroke: ColorOrGradient::Color(rgb(0xdbe2ec)).into(),
                stroke_width: 1.0.into(),
                ..Default::default()
            }
            .into(),
            text(title, 18.0, 30.0, 18.0, FontWeight::BOLD, INK),
            text(subtitle, 18.0, 52.0, 12.0, FontWeight::NORMAL, MUTED),
            make_sphere_marks(&projector, &sphere_style).into(),
            make_graticule_marks(&projector, &graticule, engine)?.into(),
            projected(
                &projector,
                &polygon,
                Some(with_alpha(rgb(0x35a997), 0.55)),
                rgb(0x148675),
                1.2,
            ),
        ];
        for (line, color) in &routes {
            panel.push(projected(&projector, line, None, *color, 2.3));
            let ends: Vec<_> = line
                .points()
                .filter_map(|point| projector.project_point(point.x(), point.y()))
                .collect();
            panel.push(
                SceneSymbolMark {
                    len: ends.len() as u32,
                    shapes: vec![SymbolShape::Circle],
                    x: ends
                        .iter()
                        .map(|end| end.0 as f32)
                        .collect::<Vec<_>>()
                        .into(),
                    y: ends
                        .iter()
                        .map(|end| end.1 as f32)
                        .collect::<Vec<_>>()
                        .into(),
                    size: 36.0.into(),
                    fill: ColorOrGradient::Color(*color).into(),
                    stroke: ColorOrGradient::Color([1.0; 4]).into(),
                    stroke_width: Some(1.0),
                    ..Default::default()
                }
                .into(),
            );
        }
        marks.push(
            SceneGroup {
                origin: [
                    24.0 + (index % 3) as f32 * 372.0,
                    100.0 + (index / 3) as f32 * 286.0,
                ],
                marks: panel,
                ..Default::default()
            }
            .into(),
        );
    }
    for (x, color, label) in [
        (32.0, rgb(0xb54786), "London–Tokyo"),
        (230.0, rgb(0xc66a18), "San Francisco–Tokyo (crosses ±180°)"),
    ] {
        marks.push(
            SceneRuleMark {
                x: x.into(),
                y: 702.0.into(),
                x2: (x + 24.0).into(),
                y2: 702.0.into(),
                stroke: ColorOrGradient::Color(color).into(),
                stroke_width: 3.0.into(),
                ..Default::default()
            }
            .into(),
        );
        marks.push(text(label, x + 32.0, 707.0, 13.0, FontWeight::NORMAL, INK));
    }
    marks.push(
        SceneRectMark {
            x: 570.0.into(),
            y: 694.0.into(),
            width: Some(18.0.into()),
            height: Some(16.0.into()),
            fill: ColorOrGradient::Color(with_alpha(rgb(0x35a997), 0.55)).into(),
            ..Default::default()
        }
        .into(),
    );
    marks.push(text(
        "Illustrative GeoJSON polygon",
        598.0,
        707.0,
        13.0,
        FontWeight::NORMAL,
        INK,
    ));
    Ok(SceneGraph {
        width: 1140.0,
        height: 740.0,
        origin: [0.0, 0.0],
        marks,
    })
}

fn main() -> Result<(), Box<dyn Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/geo-gallery"));
    std::fs::create_dir_all(&output)?;
    // The bundled fonts, without the system's, keep the images the same on every machine.
    let engine = LabelEngine::new(EngineOptions {
        fonts: FontOptions {
            load_system_fonts: false,
            ..avenger_typst_label::bundled_font_options()
        },
    })
    .with_number_formatting(Arc::new(D3NumberFormatProvider::new()));
    let svg = avenger_svg::SvgRenderer::new().render_scene_graph(&gallery(&engine)?, &engine)?;
    std::fs::write(output.join("projections.svg"), &svg)?;
    avenger_render_fixtures::raster::svg_to_png(&svg, 2.0).save(output.join("projections.png"))?;
    println!(
        "Wrote projections.svg and projections.png to {}",
        output.display()
    );
    Ok(())
}
