//! Every projection in avenger-geo, laid out in a wrap with `avenger-layout`, with the same
//! three GeoArrow layers on each map: Natural Earth countries by continent, PB2002 plate
//! boundaries, and the USGS earthquakes of magnitude 6 and larger from 2000 through 2024. Each
//! layer goes through `Projector::project_geometry` and `geometry_marks`. The example exports the
//! page with `avenger-svg`, to an optional output directory (default: target/geo-gallery).
//!
//! The data files in `examples/data` are Git LFS objects: run `git lfs pull` when they are
//! pointer files.

use std::error::Error;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow::array::{ArrayRef, AsArray, StringArray};
use arrow::compute::{concat_batches, filter_record_batch, not};
use arrow::ipc::reader::StreamReader;
use arrow::record_batch::RecordBatch;
use avenger_color::ColorOrGradient;
use avenger_common::types::{FontWeight, SymbolShape, TextBaseline};
use avenger_common::value::ScalarOrArray;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_geo::projections::{
    Aitoff, AzimuthalEqualArea, AzimuthalEquidistant, ConicConformal, ConicEqualArea,
    ConicEquidistant, CylindricalEqualArea, Eckert4, EqualEarth, Equirectangular, Gnomonic, Hammer,
    Mercator, Miller, Mollweide, NaturalEarth1, Orthographic, Robinson, Sinusoidal, Stereographic,
    TransverseMercator, WinkelTripel,
};
use avenger_geo::{FitSummary, Projection, RegionFrame, SphericalClip};
use avenger_guides::attribution::{Attribution, AttributionConfig};
use avenger_guides::axis::AxisConfig;
use avenger_guides::legend::{
    Colorbar, ColorbarConfig, Legend, LegendConfig, LegendLayout, LegendSwatches, LegendWrap,
    SymbolSwatches,
};
use avenger_guides::{BackgroundStyle, GuideSide, PartExtent};
use avenger_guides_geo::{
    geometry_marks, make_sphere_marks, GeometryStyle, Graticule, GraticuleConfig, GraticuleStyle,
    MapFrame, SphereStyle,
};
use avenger_layout::{
    Direction, Edges, Gap, Layout, Leaf, Measure, Measurement, Overflow, Placement, Position, Rect,
    RegionKind, Side, Slab, SlabStack, SolveOptions, Track, Wrap,
};
use avenger_scales::scales::linear::LinearScale;
use avenger_scales::scales::ordinal::OrdinalScale;
use avenger_scenegraph::marks::{
    group::SceneGroup, mark::SceneMark, rect::SceneRectMark, text::SceneTextMark,
};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::{EngineOptions, FontOptions, LabelEngine};
use geo_types::MultiPoint;
use geoarrow_array::array::from_arrow_array;
use geoarrow_array::GeoArrowArray;

const WIDTH: f32 = 1400.0;
const INK: [f32; 4] = rgb(0x172b42);
const MUTED: [f32; 4] = rgb(0x52667c);
const OCEAN: [f32; 4] = rgb(0xdce9f2);
const PLATES: [f32; 4] = [0.70, 0.16, 0.16, 0.85];

/// Continents and their land colors, in legend order.
const CONTINENTS: [(&str, u32); 8] = [
    ("Africa", 0xf0dfa8),
    ("Asia", 0xd8ccb3),
    ("Europe", 0xc4d4a8),
    ("North America", 0xa9d0b8),
    ("South America", 0x9fcbd3),
    ("Oceania", 0xcdbfe0),
    ("Antarctica", 0xf2f2ef),
    ("Seven seas (open ocean)", 0xdcd6ca),
];

/// Earthquake depth, shallow to deep.
const DEPTHS: [&str; 5] = ["#f46d43", "#d53e4f", "#9e0142", "#5e3c99", "#2d004b"];

const fn rgb(hex: u32) -> [f32; 4] {
    [
        ((hex >> 16) & 0xff) as f32 / 255.0,
        ((hex >> 8) & 0xff) as f32 / 255.0,
        (hex & 0xff) as f32 / 255.0,
        1.0,
    ]
}

/// Points along the border of the box from `west`, `south` to `east`, `north`, a degree apart,
/// for summarizing a region whose parallels curve. `east` may pass 180 for a region that spans
/// the antimeridian.
fn border(west: f64, south: f64, east: f64, north: f64) -> MultiPoint<f64> {
    let wrap = |lon: f64| if lon > 180.0 { lon - 360.0 } else { lon };
    let mut points = Vec::new();
    for step in 0..=((east - west) as usize) {
        let lon = wrap(west + step as f64);
        points.extend([(lon, south), (lon, north)]);
    }
    for step in 0..=((north - south) as usize) {
        let lat = south + step as f64;
        points.extend([(wrap(west), lat), (wrap(east), lat)]);
    }
    MultiPoint::from(points)
}

/// A layer read from `examples/data`, as one record batch.
fn read_layer(name: &str) -> Result<RecordBatch, Box<dyn Error>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/data")
        .join(format!("{name}.arrows"));
    let bytes = std::fs::read(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    if bytes.starts_with(b"version https://git-lfs") {
        return Err(format!(
            "{} is a Git LFS pointer file: run `git lfs pull` to fetch the data",
            path.display()
        )
        .into());
    }
    let reader = StreamReader::try_new(Cursor::new(bytes), None)?;
    let schema = reader.schema();
    let batches = reader.collect::<Result<Vec<_>, _>>()?;
    Ok(concat_batches(&schema, &batches)?)
}

/// A batch's `geometry` column as a GeoArrow array.
fn geometry(batch: &RecordBatch) -> Result<Arc<dyn GeoArrowArray>, Box<dyn Error>> {
    let schema = batch.schema();
    let index = schema.index_of("geometry")?;
    Ok(from_arrow_array(batch.column(index), schema.field(index))?)
}

fn column(batch: &RecordBatch, name: &str) -> Result<ArrayRef, Box<dyn Error>> {
    Ok(batch.column(batch.schema().index_of(name)?).clone())
}

/// The three layers, with the styles that every map shares.
struct Layers {
    countries: Arc<dyn GeoArrowArray>,
    countries_style: GeometryStyle,
    /// Plate boundaries other than subduction zones, and subduction zones.
    boundaries: [Arc<dyn GeoArrowArray>; 2],
    earthquakes: Arc<dyn GeoArrowArray>,
    earthquakes_style: GeometryStyle,
}

impl Layers {
    fn read() -> Result<Self, Box<dyn Error>> {
        let countries = read_layer("countries")?;
        let continents = OrdinalScale::configured(Arc::new(StringArray::from(
            CONTINENTS.map(|(name, _)| name).to_vec(),
        )))
        .with_range_colors(CONTINENTS.map(|(_, color)| rgb(color)).to_vec())?;
        let countries_style = GeometryStyle {
            fill: continents.scale_to_color(&column(&countries, "continent")?)?,
            stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([1.0, 1.0, 1.0, 0.9])),
            stroke_width: 0.35,
            ..Default::default()
        };

        let plates = read_layer("plate_boundaries")?;
        let subduction = column(&plates, "subduction")?;
        let subduction = subduction.as_boolean();
        let boundaries = [
            geometry(&filter_record_batch(&plates, &not(subduction)?)?)?,
            geometry(&filter_record_batch(&plates, subduction)?)?,
        ];

        let earthquakes = read_layer("earthquakes")?;
        let depth = LinearScale::configured_color((0.0, 700.0), DEPTHS);
        let earthquakes_style = GeometryStyle {
            fill: depth.scale_to_color(&column(&earthquakes, "depth_km")?)?,
            stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([1.0, 1.0, 1.0, 0.8])),
            stroke_width: 0.4,
            point_size: magnitude_scale().scale_to_numeric(&column(&earthquakes, "magnitude")?)?,
            point_shape: ScalarOrArray::new_scalar(SymbolShape::Circle),
        };
        Ok(Self {
            countries: geometry(&countries)?,
            countries_style,
            boundaries,
            earthquakes: geometry(&earthquakes)?,
            earthquakes_style,
        })
    }
}

/// A symbol's area for an earthquake's magnitude.
fn magnitude_scale() -> avenger_scales::scales::ConfiguredScale {
    LinearScale::configured((6.0, 9.1), (3.0, 110.0))
}

/// A map: its projection's name and its prepared graticule.
struct Panel {
    name: &'static str,
    graticule: Graticule,
}

/// How a map frames its projection.
#[derive(Clone, Copy)]
enum Framing {
    /// The part of the sphere that the projection's clip keeps, with lines `step` degrees apart.
    Outline { step: f64 },
    /// The region from west, south to east, north, in a rectangle, with lines 10° apart.
    Region([f64; 4]),
}

/// One map for each projection, in families: cylindrical, pseudocylindrical and compromise,
/// conic, and azimuthal.
fn panels(engine: &LabelEngine) -> Result<Vec<Panel>, Box<dyn Error>> {
    use Framing::{Outline, Region};
    let style = GraticuleStyle {
        stroke: [1.0, 1.0, 1.0, 0.7],
        stroke_width: 0.5,
        ..Default::default()
    };
    let world = Outline { step: 30.0 };
    let specs: [(&str, Projection, Framing); 22] = [
        ("Equirectangular", Projection::new(Equirectangular), world),
        ("Miller", Projection::new(Miller), world),
        (
            "Cylindrical equal-area",
            Projection::new(CylindricalEqualArea::default()),
            world,
        ),
        (
            "Mercator, latitudes −60° to 80°",
            Projection::new(Mercator).with_clip(Some(SphericalClip::lon_lat_box(
                RegionFrame::View,
                -180.0,
                -60.0,
                180.0,
                80.0,
            )?)),
            world,
        ),
        ("Equal Earth", Projection::new(EqualEarth), world),
        ("Natural Earth", Projection::new(NaturalEarth1), world),
        ("Robinson", Projection::new(Robinson), world),
        ("Mollweide", Projection::new(Mollweide), world),
        ("Sinusoidal", Projection::new(Sinusoidal), world),
        ("Eckert IV", Projection::new(Eckert4), world),
        ("Hammer", Projection::new(Hammer::default()), world),
        ("Aitoff", Projection::new(Aitoff), world),
        ("Winkel tripel", Projection::new(WinkelTripel), world),
        (
            "Transverse Mercator, the Andes",
            Projection::new(TransverseMercator).with_rotate([70.0, 0.0, 0.0]),
            Region([-115.0, -58.0, -25.0, 14.0]),
        ),
        (
            "Conformal conic, northwest Pacific",
            Projection::new(ConicConformal::new([30.0, 60.0])).with_rotate([-160.0, 0.0, 0.0]),
            Region([120.0, 15.0, 200.0, 65.0]),
        ),
        (
            "Equal-area conic, Alpine–Himalayan belt",
            Projection::new(ConicEqualArea::new([25.0, 50.0])).with_rotate([-45.0, 0.0, 0.0]),
            Region([-12.0, 18.0, 102.0, 56.0]),
        ),
        (
            "Equidistant conic, Caribbean",
            Projection::new(ConicEquidistant::new([8.0, 28.0])).with_rotate([88.0, 0.0, 0.0]),
            Region([-120.0, 0.0, -56.0, 34.0]),
        ),
        (
            "Orthographic, Pacific Ring of Fire",
            Projection::new(Orthographic).with_rotate([170.0, -10.0, 0.0]),
            world,
        ),
        (
            "Azimuthal equidistant, North Pole",
            Projection::new(AzimuthalEquidistant).with_rotate([0.0, -90.0, 0.0]),
            world,
        ),
        (
            "Azimuthal equal-area, South Pole",
            Projection::new(AzimuthalEqualArea).with_rotate([0.0, 90.0, 0.0]),
            world,
        ),
        (
            "Gnomonic, Japan",
            Projection::new(Gnomonic).with_rotate([-138.0, -36.0, 0.0]),
            Outline { step: 10.0 },
        ),
        (
            "Stereographic, North Pacific, a 60° cap",
            Projection::new(Stereographic)
                .with_rotate([170.0, -45.0, 0.0])
                .with_clip(Some(SphericalClip::Circle { angle: 60.0 })),
            Outline { step: 10.0 },
        ),
    ];
    specs
        .into_iter()
        .map(|(name, projection, framing)| {
            let projection = projection.with_precision(0.2);
            let (summary, frame, step): (FitSummary, _, _) = match framing {
                Region([west, south, east, north]) => (
                    projection.summarize(&border(west, south, east, north))?,
                    MapFrame::Rectangle,
                    10.0,
                ),
                Outline { step } => (projection.summarize_sphere()?, MapFrame::Outline, step),
            };
            let config = GraticuleConfig {
                graticule: avenger_geo::Graticule::default().with_step(step),
                frame,
                label_sides: Vec::new(),
                style: style.clone(),
                ..Default::default()
            };
            Ok(Panel {
                name,
                graticule: Graticule::new(&projection, &summary, &config, engine)?,
            })
        })
        .collect()
}

/// The guides beside and below the maps, and the maps' aspects.
struct Guides {
    aspects: Vec<f32>,
    continents: Legend,
    magnitude: Legend,
    depth: Colorbar,
    credits: Attribution,
}

impl Guides {
    fn new(panels: &[Panel], engine: &LabelEngine) -> Result<Self, Box<dyn Error>> {
        let right = LegendLayout {
            side: GuideSide::Right,
            wrap: LegendWrap::Flow,
        };
        let continents = Legend::new(
            &LegendConfig {
                title: Some("Continent".to_string()),
                text: ScalarOrArray::new_array(
                    CONTINENTS[..7]
                        .iter()
                        .map(|(name, _)| name.to_string())
                        .collect(),
                ),
                swatches: LegendSwatches::Symbols(SymbolSwatches {
                    fill: ScalarOrArray::new_array(
                        CONTINENTS[..7]
                            .iter()
                            .map(|(_, color)| ColorOrGradient::Color(rgb(*color)))
                            .collect(),
                    ),
                    size: ScalarOrArray::new_scalar(90.0),
                    stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color(MUTED)),
                    stroke_width: Some(0.5),
                    ..Default::default()
                }),
                layout: right,
                ..Default::default()
            },
            engine,
        )?;
        let magnitudes = [6.0, 7.0, 8.0, 9.0];
        let sizes = magnitude_scale().scale_to_numeric(
            &(Arc::new(arrow::array::Float64Array::from(magnitudes.to_vec())) as ArrayRef),
        )?;
        let magnitude = Legend::new(
            &LegendConfig {
                title: Some("Magnitude".to_string()),
                text: ScalarOrArray::new_array(magnitudes.map(|m| format!("{m}")).to_vec()),
                swatches: LegendSwatches::Symbols(SymbolSwatches {
                    size: sizes,
                    fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([1.0; 4])),
                    stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color(INK)),
                    stroke_width: Some(0.8),
                    ..Default::default()
                }),
                layout: right,
                ..Default::default()
            },
            engine,
        )?;
        let defaults = ColorbarConfig::default();
        let depth = Colorbar::new(
            &LinearScale::configured_color((0.0, 700.0), DEPTHS),
            "Depth (km)",
            &ColorbarConfig {
                axis: AxisConfig {
                    side: GuideSide::Right,
                    tick_format: ",f".into(),
                    ..defaults.axis
                },
                ..defaults
            },
            engine,
        )?;
        let credits = Attribution::new(
            &[
                "Countries: Natural Earth".to_string(),
                "Earthquakes: U.S. Geological Survey".to_string(),
                "Plate boundaries: Peter Bird (2003), Hugo Ahlenius and Nordpil, ODC-BY 1.0"
                    .to_string(),
            ],
            &AttributionConfig {
                font_family: "Lato".to_string(),
                font_size: 10.0,
                color: MUTED,
                background: BackgroundStyle {
                    padding: 10.0,
                    ..Default::default()
                },
                ..Default::default()
            },
            engine,
        )?;
        Ok(Self {
            aspects: panels
                .iter()
                .map(|panel| panel.graticule.aspect())
                .collect(),
            continents,
            magnitude,
            depth,
            credits,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Id {
    Header,
    Map(usize),
    Continents,
    Magnitude,
    Depth,
    Credits(usize),
}

/// The maps in a wrap of equal columns, under a header, with the legends in a stack on the right.
/// Each map's padding holds its name. A map answers its height at its width, so each row is as
/// tall as its tallest map, and a map keeps its aspect inside the rest of its cell. Each map
/// credits its data below it, and since every map credits the same sources, the wrap merges the
/// credits into one attribution along the bottom.
fn page_layout(panels: &[Panel], credits: &str) -> Layout<Id> {
    let maps = panels.iter().enumerate().map(|(index, panel)| {
        Leaf::new()
            .id(Id::Map(index))
            .slab(
                Side::Bottom,
                Slab::new()
                    .id(Id::Credits(index))
                    .tag("credits")
                    .content_key(credits),
            )
            .aspect(panel.graticule.aspect())
            .padding(Edges::new(0.0, 24.0, 0.0, 0.0))
            .overflow_direction(Direction::Vertical, Overflow::Contain)
            .place(Direction::Horizontal, Placement::stretch(Position::Center))
            .place(Direction::Vertical, Placement::stretch(Position::Center))
    });
    Wrap::new(maps)
        .min_column(260.0)
        .gap(Gap::new(20.0))
        .slab(
            Side::Top,
            Slab::new().id(Id::Header).min_height(70.0).full(),
        )
        .slab(Side::Right, Slab::new().min_width(24.0))
        .slab_stack(
            Side::Right,
            SlabStack::new([
                Slab::new().id(Id::Continents),
                Slab::new().id(Id::Magnitude),
                Slab::new().id(Id::Depth),
            ])
            .track(0, Track::auto())
            .track(1, Track::auto())
            .track(2, Track::px(180.0))
            .gap(Gap::new(18.0))
            .distribute(avenger_layout::Distribute::Start),
        )
        .merge_tag("credits")
        .into()
}

fn answer(extent: PartExtent) -> Measurement {
    Measurement::over(extent.thickness, extent.holds).with_overhang(extent.overhang)
}

impl Measure<Id> for Guides {
    fn height(&mut self, id: &Id, width: f32) -> Option<Measurement> {
        match id {
            Id::Map(index) => Some(Measurement::over(
                width / self.aspects[*index],
                width..=width,
            )),
            Id::Credits(_) => Some(answer(self.credits.extent(width))),
            _ => None,
        }
    }

    fn width(&mut self, id: &Id, height: f32) -> Option<Measurement> {
        match id {
            Id::Continents => Some(answer(self.continents.extent(height))),
            Id::Magnitude => Some(answer(self.magnitude.extent(height))),
            Id::Depth => Some(answer(self.depth.extent(height))),
            _ => None,
        }
    }

    fn max_content(&mut self, id: &Id, direction: Direction) -> Option<f32> {
        match (id, direction) {
            (Id::Continents, Direction::Vertical) => Some(self.continents.max_content()),
            (Id::Magnitude, Direction::Vertical) => Some(self.magnitude.max_content()),
            _ => None,
        }
    }
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

fn slot(rect: Rect) -> [f32; 4] {
    [rect.x, rect.y, rect.width, rect.height]
}

/// A map's layers at its content rectangle, through its graticule's projector.
fn map_marks(panel: &Panel, layers: &Layers, map: Rect) -> Result<SceneMark, Box<dyn Error>> {
    let size = [map.width, map.height];
    let graticule = &panel.graticule;
    let projector = graticule.projector(size);
    let ocean = SphereStyle {
        fill: OCEAN,
        stroke: rgb(0x9fb6c9),
        stroke_width: 0.8,
    };
    let project = |geometry: &Arc<dyn GeoArrowArray>| projector.project_geometry(geometry.as_ref());
    let boundary = |width: f32| GeometryStyle {
        stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color(PLATES)),
        stroke_width: width,
        ..Default::default()
    };
    let marks = vec![
        make_sphere_marks(&projector, &ocean).into(),
        graticule.lines(size).into(),
        geometry_marks(
            project(&layers.countries)?.as_ref(),
            &layers.countries_style,
        )?
        .into(),
        geometry_marks(project(&layers.boundaries[0])?.as_ref(), &boundary(0.7))?.into(),
        geometry_marks(project(&layers.boundaries[1])?.as_ref(), &boundary(1.6))?.into(),
        geometry_marks(
            project(&layers.earthquakes)?.as_ref(),
            &layers.earthquakes_style,
        )?
        .into(),
    ];
    Ok(SceneGroup {
        origin: [map.x, map.y],
        marks,
        ..Default::default()
    }
    .into())
}

fn gallery(engine: &LabelEngine) -> Result<SceneGraph, Box<dyn Error>> {
    let layers = Layers::read()?;
    let panels = panels(engine)?;
    let mut guides = Guides::new(&panels, engine)?;
    let options = SolveOptions {
        width: Some(WIDTH),
        height: None,
        padding: Edges::all(28.0),
        ..SolveOptions::default()
    };
    let solution = page_layout(&panels, &guides.credits.content_key())
        .measure(&options, &mut guides)?
        .into_solution();
    for warning in solution.warnings() {
        eprintln!("layout warning: {warning:?}");
    }
    let size = solution.size();

    let mut marks: Vec<SceneMark> = vec![SceneRectMark {
        width: Some(size.width.into()),
        height: Some(size.height.into()),
        fill: ColorOrGradient::Color(rgb(0xf7f8fa)).into(),
        ..Default::default()
    }
    .into()];
    for region in solution.regions() {
        let Some(id) = region.id else {
            continue;
        };
        if matches!(
            region.kind,
            RegionKind::Slab {
                represented_by: Some(_),
                ..
            }
        ) {
            continue;
        }
        let rect = region.content;
        match id {
            Id::Header => {
                marks.push(text(
                    "Earthquakes trace the plate boundaries",
                    rect.x,
                    rect.y + 30.0,
                    26.0,
                    FontWeight::BOLD,
                    INK,
                ));
                marks.push(text(
                    "Magnitude 6 and larger, 2000 through 2024. Red lines are plate boundaries, heavier at subduction zones.",
                    rect.x,
                    rect.y + 56.0,
                    14.0,
                    FontWeight::NORMAL,
                    MUTED,
                ));
            }
            Id::Map(index) => {
                let panel = &panels[index];
                // The name stands in the cell's padding, above the map.
                marks.push(text(
                    panel.name,
                    region.slot.x,
                    region.slot.y + 15.0,
                    13.0,
                    FontWeight::BOLD,
                    INK,
                ));
                marks.push(map_marks(panel, &layers, rect)?);
            }
            Id::Continents => {
                let mut group = guides.continents.marks(rect.height).group;
                group.origin = guides.continents.origin(slot(rect));
                marks.push(group.into());
            }
            Id::Magnitude => {
                let mut group = guides.magnitude.marks(rect.height).group;
                group.origin = guides.magnitude.origin(slot(rect));
                marks.push(group.into());
            }
            Id::Depth => {
                let mut group = guides.depth.marks(rect.height).group;
                group.origin = guides.depth.origin(slot(rect));
                marks.push(group.into());
            }
            Id::Credits(_) => {
                if let Some(mut group) = guides.credits.part(rect.width) {
                    group.origin = guides.credits.origin(slot(rect));
                    marks.push(group.into());
                }
            }
        }
    }
    Ok(SceneGraph {
        width: size.width,
        height: size.height,
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
    let scene = gallery(&engine)?;
    let svg = avenger_svg::SvgRenderer::new().render_scene_graph(&scene, &engine)?;
    std::fs::write(output.join("world.svg"), &svg)?;
    avenger_render_fixtures::raster::svg_to_png(&svg, 2.0).save(output.join("world.png"))?;
    println!("Wrote world.svg and world.png to {}", output.display());
    Ok(())
}
