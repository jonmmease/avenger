//! Draw maps as an Avenger scene graph, with the sphere and the prepared
//! graticule from this crate, laid out with `avenger-layout`, and export them
//! with `avenger-svg`. Run with an optional output directory (default:
//! target/geo-gallery).

use avenger_color::ColorOrGradient;
use avenger_common::types::{FontWeight, StrokeCap, SymbolShape, TextBaseline};
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_geo::ingest::geojson_to_features;
use avenger_geo::projections::{ConicEqualArea, EqualEarth, Mercator, NaturalEarth1, WinkelTripel};
use avenger_geo::{Projection, Projector};
use avenger_guides::GuideSide;
use avenger_guides_geo::{
    make_sphere_marks, Graticule, GraticuleConfig, GraticuleStyle, MapFrame, SphereStyle,
};
use avenger_layout::{
    Direction, Edges, Gap, Grid, GridSlot, Layout, Leaf, Measure, Measurement, Overflow, Placement,
    Position, Rect, Side, Slab, SolveOptions, Track,
};
use avenger_scenegraph::marks::{
    group::SceneGroup, mark::SceneMark, path::ScenePathMark, rect::SceneRectMark,
    rule::SceneRuleMark, symbol::SceneSymbolMark, text::SceneTextMark,
};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::{EngineOptions, FontOptions, LabelEngine};
use geo_traits::GeometryTrait;
use geo_types::{LineString, MultiPoint};
use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;

const WIDTH: f32 = 1140.0;
const HEIGHT: f32 = 740.0;
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

/// A map panel: its titles, its prepared graticule and the sides that label its lines.
struct Panel {
    title: &'static str,
    subtitle: &'static str,
    graticule: Graticule,
    label_sides: Vec<GuideSide>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum PanelId {
    Map(usize),
    Labels(usize, GuideSide),
}

fn layout_side(side: GuideSide) -> Side {
    match side {
        GuideSide::Top => Side::Top,
        GuideSide::Bottom => Side::Bottom,
        GuideSide::Left => Side::Left,
        GuideSide::Right => Side::Right,
    }
}

/// Six cards in three columns. Each card's padding holds its titles, and its sides contain the
/// slabs of its map's labels. The map keeps its aspect, centered in the space that is left.
fn gallery_layout(panels: &[Panel]) -> Layout<PanelId> {
    let mut grid = Grid::new(
        (0..3).map(|_| Track::fr(1.0)),
        (0..2).map(|_| Track::fr(1.0)),
    )
    .column_gap(Gap::new(24.0))
    .row_gap(Gap::new(20.0));
    for (index, panel) in panels.iter().enumerate() {
        let mut map = Leaf::new()
            .id(PanelId::Map(index))
            .aspect(panel.graticule.aspect())
            .padding(Edges::new(16.0, 64.0, 16.0, 12.0))
            .overflow_direction(Direction::Horizontal, Overflow::Contain)
            .overflow_direction(Direction::Vertical, Overflow::Contain)
            .place(Direction::Horizontal, Placement::stretch(Position::Center))
            .place(Direction::Vertical, Placement::stretch(Position::Center));
        for &side in &panel.label_sides {
            map = map.slab(
                layout_side(side),
                Slab::new().id(PanelId::Labels(index, side)),
            );
        }
        grid = grid.child(GridSlot::new(index % 3, index / 3), map);
    }
    grid.into()
}

/// Measures each label slab with its graticule's extent.
struct MapLabels<'a>(&'a [Panel]);

impl Measure<PanelId> for MapLabels<'_> {
    fn height(&mut self, id: &PanelId, width: f32) -> Option<Measurement> {
        self.answer(id, width)
    }

    fn width(&mut self, id: &PanelId, height: f32) -> Option<Measurement> {
        self.answer(id, height)
    }
}

impl MapLabels<'_> {
    fn answer(&self, id: &PanelId, length: f32) -> Option<Measurement> {
        let PanelId::Labels(index, side) = id else {
            return None;
        };
        let extent = self.0[*index].graticule.extent(*side, length);
        Some(Measurement::over(extent.thickness, extent.holds).with_overhang(extent.overhang))
    }
}

fn slot(rect: Rect) -> [f32; 4] {
    [rect.x, rect.y, rect.width, rect.height]
}

/// Points along the border of the box from `west`, `south` to `east`, `north`, a degree apart,
/// for summarizing a region whose parallels curve.
fn border(west: f64, south: f64, east: f64, north: f64) -> MultiPoint<f64> {
    let mut points = Vec::new();
    for step in 0..=((east - west) as usize) {
        let lon = west + step as f64;
        points.extend([(lon, south), (lon, north)]);
    }
    for step in 0..=((north - south) as usize) {
        let lat = south + step as f64;
        points.extend([(west, lat), (east, lat)]);
    }
    MultiPoint::from(points)
}

fn panels(engine: &LabelEngine) -> Result<Vec<Panel>, Box<dyn Error>> {
    // Meridians run pole to pole, so that their labels line up along the
    // bottom of each world map.
    let default = avenger_geo::Graticule::default().with_step(30.0);
    let world_lines = avenger_geo::Graticule {
        extent_minor: default.extent_major,
        ..default
    };
    let style = GraticuleStyle {
        stroke: rgb(0xb5c9dc),
        stroke_width: 0.65,
        label_color: MUTED,
        label_font_family: "Lato".to_string(),
        label_font_size: 8.0,
        ..Default::default()
    };
    // North-up world maps label parallels on the left and meridians along the
    // bottom. The rotated view draws lines only. A regional conic of Europe and
    // North Africa fills its rectangle and is clipped to it, with lines 10°
    // apart.
    let labeled = vec![GuideSide::Left, GuideSide::Bottom];
    let maps = [
        (
            "Equal Earth",
            "Equal-area world view",
            Projection::new(EqualEarth),
            None,
            labeled.clone(),
        ),
        (
            "Mercator",
            "World-square clipping at the poles",
            Projection::new(Mercator),
            None,
            labeled.clone(),
        ),
        (
            "Natural Earth",
            "Curved meridians",
            Projection::new(NaturalEarth1),
            None,
            labeled.clone(),
        ),
        (
            "Winkel tripel",
            "Compromise world projection",
            Projection::new(WinkelTripel),
            None,
            labeled.clone(),
        ),
        (
            "Conic equal-area",
            "Europe and North Africa, clipped to the frame",
            Projection::new(ConicEqualArea::new([20.0, 50.0])).with_rotate([-15.0, 0.0, 0.0]),
            Some(border(-12.0, 8.0, 42.0, 62.0)),
            labeled.clone(),
        ),
        (
            "Rotated Equal Earth",
            "Three-axis rotation: 96°, −30°, 15°",
            Projection::new(EqualEarth).with_rotate([96.0, -30.0, 15.0]),
            None,
            Vec::new(),
        ),
    ];
    maps.into_iter()
        .map(|(title, subtitle, projection, region, label_sides)| {
            let projection = projection.with_precision(0.2);
            let (summary, frame, graticule) = match &region {
                Some(region) => (
                    projection.summarize(region)?,
                    MapFrame::Rectangle,
                    avenger_geo::Graticule::default().with_step(10.0),
                ),
                None => (
                    projection.summarize_sphere()?,
                    MapFrame::Outline,
                    world_lines.clone(),
                ),
            };
            let config = GraticuleConfig {
                graticule,
                frame,
                label_sides: label_sides.clone(),
                style: style.clone(),
                ..Default::default()
            };
            Ok(Panel {
                title,
                subtitle,
                graticule: Graticule::new(&projection, &summary, &config, engine)?,
                label_sides,
            })
        })
        .collect()
}

fn gallery(engine: &LabelEngine) -> Result<SceneGraph, Box<dyn Error>> {
    let sphere_style = SphereStyle {
        fill: rgb(0xedf4fa),
        stroke: rgb(0x7292ad),
        stroke_width: 1.0,
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

    let panels = panels(engine)?;
    let options = SolveOptions {
        padding: Edges::new(24.0, 100.0, 24.0, 70.0),
        ..SolveOptions::sized(WIDTH, HEIGHT)
    };
    let solution = gallery_layout(&panels)
        .measure(&options, &mut MapLabels(&panels))?
        .into_solution();
    for warning in solution.warnings() {
        eprintln!("layout warning: {warning:?}");
    }

    let mut marks: Vec<SceneMark> = vec![
        SceneRectMark {
            width: Some(WIDTH.into()),
            height: Some(HEIGHT.into()),
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
    for region in solution.regions() {
        match region.id {
            Some(PanelId::Map(index)) => {
                let panel = &panels[index];
                let (card, map) = (region.slot, region.content);
                marks.push(
                    SceneRectMark {
                        x: card.x.into(),
                        y: card.y.into(),
                        width: Some(card.width.into()),
                        height: Some(card.height.into()),
                        corner_radius: 12.0.into(),
                        fill: ColorOrGradient::Color([1.0; 4]).into(),
                        stroke: ColorOrGradient::Color(rgb(0xdbe2ec)).into(),
                        stroke_width: 1.0.into(),
                        ..Default::default()
                    }
                    .into(),
                );
                marks.push(text(
                    panel.title,
                    card.x + 18.0,
                    card.y + 30.0,
                    18.0,
                    FontWeight::BOLD,
                    INK,
                ));
                marks.push(text(
                    panel.subtitle,
                    card.x + 18.0,
                    card.y + 52.0,
                    12.0,
                    FontWeight::NORMAL,
                    MUTED,
                ));
                let size = [map.width, map.height];
                let projector = panel.graticule.projector(size);
                let mut drawn = vec![
                    make_sphere_marks(&projector, &sphere_style).into(),
                    panel.graticule.lines(size).into(),
                    projected(
                        &projector,
                        &polygon,
                        Some(with_alpha(rgb(0x35a997), 0.55)),
                        rgb(0x148675),
                        1.2,
                    ),
                ];
                for (line, color) in &routes {
                    drawn.push(projected(&projector, line, None, *color, 2.3));
                    // A regional map leaves out the ends that lie past its frame.
                    let ends: Vec<_> = line
                        .points()
                        .filter_map(|point| projector.project_point(point.x(), point.y()))
                        .map(|(x, y)| [x as f32, y as f32])
                        .filter(|end| map.contains([map.x + end[0], map.y + end[1]]))
                        .collect();
                    drawn.push(
                        SceneSymbolMark {
                            len: ends.len() as u32,
                            shapes: vec![SymbolShape::Circle],
                            x: ends.iter().map(|end| end[0]).collect::<Vec<_>>().into(),
                            y: ends.iter().map(|end| end[1]).collect::<Vec<_>>().into(),
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
                        origin: [map.x, map.y],
                        marks: drawn,
                        ..Default::default()
                    }
                    .into(),
                );
            }
            Some(PanelId::Labels(index, side)) => {
                let graticule = &panels[index].graticule;
                let rect = region.content;
                let length = if side.is_vertical() {
                    rect.height
                } else {
                    rect.width
                };
                if let Some(mut group) = graticule.part(side, length) {
                    group.origin = graticule.origin(side, slot(rect));
                    marks.push(group.into());
                }
            }
            None => {}
        }
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
        width: WIDTH,
        height: HEIGHT,
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
