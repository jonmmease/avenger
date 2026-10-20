//! The maps: their data, their projections and pan modes, how each is drawn, and the legends.

use std::error::Error;
use std::io::Cursor;
use std::sync::Arc;

use arrow::array::{ArrayRef, AsArray, StringArray};
use arrow::compute::{concat_batches, filter_record_batch, not};
use arrow::ipc::reader::StreamReader;
use arrow::record_batch::RecordBatch;
use avenger_color::ColorOrGradient;
use avenger_common::types::SymbolShape;
use avenger_common::value::ScalarOrArray;
use avenger_geo::projections::{Mercator, Orthographic};
use avenger_geo::{FitSummary, PanMode, Projection, Projector};
use avenger_guides::axis::AxisConfig;
use avenger_guides::legend::{
    Colorbar, ColorbarConfig, Legend, LegendConfig, LegendLayout, LegendSwatches, LegendWrap,
    LineSwatches, SymbolSwatches,
};
use avenger_guides::GuideSide;
use avenger_guides_geo::{
    geometry_marks, make_sphere_marks, GeometryStyle, Graticule, GraticuleConfig, GraticuleStyle,
    MapFrame, SphereStyle, TileLayer,
};
use avenger_scales::scales::linear::LinearScale;
use avenger_scales::scales::ordinal::OrdinalScale;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::group::{Clip, SceneGroup};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_typst_label::LabelEngine;
use geoarrow_array::array::from_arrow_array;
use geoarrow_array::GeoArrowArray;

/// The world gallery's layers, embedded so that the WebAssembly build has them too. Over tiles,
/// which show the land, the maps draw no countries.
const COUNTRIES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../avenger-guides-geo/examples/data/countries.arrows"
));
const PLATES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../avenger-guides-geo/examples/data/plate_boundaries.arrows"
));
const EARTHQUAKES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../avenger-guides-geo/examples/data/earthquakes.arrows"
));

/// The sides that label each map's lines.
pub const LABEL_SIDES: [GuideSide; 2] = [GuideSide::Left, GuideSide::Bottom];

pub const INK: [f32; 4] = rgb(0x172b42);
pub const MUTED: [f32; 4] = rgb(0x52667c);
const OCEAN: [f32; 4] = rgb(0xdce9f2);
const PLATE_COLOR: [f32; 4] = [0.70, 0.16, 0.16, 0.85];
/// The stroke widths of plate boundaries, and of subduction zones among them.
const PLATE_WIDTHS: [f32; 2] = [0.7, 1.6];

/// Continents and their land colors, in legend order. The legend leaves out the open ocean.
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

/// A symbol's area in square pixels for an earthquake's magnitude.
fn magnitude_scale() -> ConfiguredScale {
    LinearScale::configured((6.0, 9.1), (4.0, 140.0))
}

/// An earthquake's color for its depth in kilometers.
fn depth_scale() -> ConfiguredScale {
    LinearScale::configured_color((0.0, 700.0), DEPTHS)
}

pub const fn rgb(hex: u32) -> [f32; 4] {
    [
        ((hex >> 16) & 0xff) as f32 / 255.0,
        ((hex >> 8) & 0xff) as f32 / 255.0,
        (hex & 0xff) as f32 / 255.0,
        1.0,
    ]
}

/// An embedded layer, `name`, as one record batch.
fn read_layer(name: &str, bytes: &[u8]) -> Result<RecordBatch, Box<dyn Error>> {
    if bytes.starts_with(b"version https://git-lfs") {
        return Err(format!(
            "the embedded {name} layer is a Git LFS pointer file: run `git lfs pull`, then build again"
        )
        .into());
    }
    let reader = StreamReader::try_new(Cursor::new(bytes), None)?;
    let schema = reader.schema();
    let batches = reader.collect::<Result<Vec<_>, _>>()?;
    Ok(concat_batches(&schema, &batches)?)
}

fn column(batch: &RecordBatch, name: &str) -> Result<ArrayRef, Box<dyn Error>> {
    Ok(batch.column(batch.schema().index_of(name)?).clone())
}

/// The batch's columns named `names`.
fn project_columns(batch: &RecordBatch, names: &[&str]) -> Result<RecordBatch, Box<dyn Error>> {
    let schema = batch.schema();
    let columns = names
        .iter()
        .map(|name| schema.index_of(name))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(batch.project(&columns)?)
}

fn geometry(batch: &RecordBatch) -> Result<Arc<dyn GeoArrowArray>, Box<dyn Error>> {
    let schema = batch.schema();
    let index = schema.index_of("geometry")?;
    Ok(from_arrow_array(batch.column(index), schema.field(index))?)
}

/// The layers that every map draws, with their styles.
pub struct Layers {
    countries: Arc<dyn GeoArrowArray>,
    countries_style: GeometryStyle,
    /// Plate boundaries other than subduction zones, and subduction zones.
    boundaries: [Arc<dyn GeoArrowArray>; 2],
    earthquakes: Arc<dyn GeoArrowArray>,
    earthquakes_style: GeometryStyle,
    /// The earthquakes' magnitude, depth, and time, one row for each, which each map's
    /// earthquakes take as scene data.
    pub quake_rows: Arc<RecordBatch>,
    /// The countries' names and continents, one row for each, which each map's countries take
    /// as scene data.
    pub country_rows: Arc<RecordBatch>,
}

impl Layers {
    pub fn read() -> Result<Self, Box<dyn Error>> {
        let countries = read_layer("countries", COUNTRIES)?;
        let country_rows = project_columns(&countries, &["name", "continent"])?;
        let continents = OrdinalScale::configured(Arc::new(StringArray::from(
            CONTINENTS.map(|(name, _)| name).to_vec(),
        )))
        .with_range_colors(CONTINENTS.map(|(_, color)| rgb(color)).to_vec())?;
        let countries_style = GeometryStyle {
            fill: continents.scale_to_color(&column(&countries, "continent")?)?,
            stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([1.0, 1.0, 1.0, 0.9])),
            stroke_width: 0.4,
            ..Default::default()
        };

        let plates = read_layer("plate boundaries", PLATES)?;
        let subduction = column(&plates, "subduction")?;
        let subduction = subduction.as_boolean();
        let boundaries = [
            geometry(&filter_record_batch(&plates, &not(subduction)?)?)?,
            geometry(&filter_record_batch(&plates, subduction)?)?,
        ];

        let earthquakes = read_layer("earthquakes", EARTHQUAKES)?;
        let magnitude = column(&earthquakes, "magnitude")?;
        let depth = column(&earthquakes, "depth_km")?;
        let point_size = magnitude_scale().scale_to_numeric(&magnitude)?;
        let quake_rows = project_columns(&earthquakes, &["magnitude", "depth_km", "time"])?;
        let earthquakes_style = GeometryStyle {
            fill: depth_scale().scale_to_color(&depth)?,
            stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([1.0, 1.0, 1.0, 0.8])),
            stroke_width: 0.4,
            point_size,
            point_shape: SymbolShape::Circle,
        };
        Ok(Self {
            countries: geometry(&countries)?,
            countries_style,
            boundaries,
            earthquakes: geometry(&earthquakes)?,
            earthquakes_style,
            quake_rows: Arc::new(quake_rows),
            country_rows: Arc::new(country_rows),
        })
    }
}

/// A map: its projection before any gesture, the mode it pans in, and its graticule.
pub struct MapSpec {
    pub title: &'static str,
    /// How the map pans, shown under it until the cursor is over it.
    pub hint: &'static str,
    /// The projection that a fit places and a double click restores.
    home: Projection,
    summary: FitSummary,
    pub mode: PanMode,
    /// How far the map zooms out from its fitted scale.
    pub zoom_out: f64,
    pub graticule: Graticule,
}

impl MapSpec {
    /// The home projection fitted to a map of `size`, clipped to it.
    pub fn fitted(&self, size: [f32; 2]) -> Result<Projection, Box<dyn Error>> {
        let rect = [[0.0, 0.0], [f64::from(size[0]), f64::from(size[1])]];
        let mut projection = self.home.clone();
        projection.fit(rect, &self.summary)?;
        Ok(projection.with_clip_extent(Some(rect)))
    }

    /// The fitted scale at `size`, which bounds how far the map zooms.
    pub fn fitted_scale(&self, size: [f32; 2]) -> f64 {
        self.fitted(size)
            .ok()
            .and_then(|projection| projection.view)
            .map_or(1.0, |view| view.scale)
    }
}

/// A globe and a Web Mercator world map, which pan in the sphere and longitude modes.
pub fn maps(engine: &LabelEngine) -> Result<Vec<MapSpec>, Box<dyn Error>> {
    let globe = Projection::new(Orthographic).with_rotate([170.0, -10.0, 0.0]);
    let world = Projection::new(Mercator);
    let specs = [
        (
            "Pacific Ring of Fire",
            "Orthographic: drag to turn the globe",
            globe.summarize_sphere()?,
            globe,
            PanMode::Sphere,
            0.6,
        ),
        (
            "The world",
            "Web Mercator: drag sideways to turn the world",
            world.summarize_sphere()?,
            world,
            PanMode::Longitude,
            // A web map zooms out no further than the world's width, so the antimeridian stays
            // past the map's sides.
            1.0,
        ),
    ];
    specs
        .into_iter()
        .map(|(title, hint, summary, home, mode, zoom_out)| {
            let config = GraticuleConfig {
                graticule: avenger_geo::Graticule::default().with_step(30.0),
                frame: MapFrame::Rectangle,
                label_sides: LABEL_SIDES.to_vec(),
                label_format: ".0f".into(),
                style: GraticuleStyle {
                    stroke: [1.0, 1.0, 1.0, 0.75],
                    stroke_width: 0.6,
                    label_color: MUTED,
                    label_font_family: "Lato".into(),
                    label_font_size: 10.0,
                    ..Default::default()
                },
            };
            let graticule = Graticule::new(&home, &summary, &config, engine)?;
            Ok(MapSpec {
                title,
                hint,
                home,
                summary,
                mode,
                zoom_out,
                graticule,
            })
        })
        .collect()
}

/// A map drawn at one projection and size, in the map's frame.
pub struct Drawn {
    /// The sphere, the basemap, the graticule's lines and the layers, clipped to the map.
    pub content: SceneGroup,
    /// The labels of each of [`LABEL_SIDES`], in their slabs' frames.
    pub parts: Vec<Option<SceneGroup>>,
}

/// The name of the earthquakes' symbol mark on map `map`, whose instances the app's hit-testing
/// tree finds: each instance's index is its earthquake's row.
pub fn quakes_mark(map: usize) -> String {
    format!("earthquakes-{map}")
}

/// The name of the countries' path mark on map `map`, whose instances the app's hit-testing
/// tree finds: each instance's index is its country's row.
pub fn countries_mark(map: usize) -> String {
    format!("countries-{map}")
}

/// Draw map `map` at `projection` and `size`: the sphere, the graticule's lines, and the layers,
/// over `tiles` when there are tiles, or else with the countries.
pub fn draw(
    map: usize,
    spec: &MapSpec,
    layers: &Layers,
    tiles: Option<&TileLayer>,
    projection: &Projection,
    size: [f32; 2],
) -> Result<Drawn, Box<dyn Error>> {
    let projector = projection.build()?;
    let view = spec.graticule.view(&projector, size);
    let project = |layer: &Arc<dyn GeoArrowArray>| projector.project_geometry(layer.as_ref());
    let boundary = |width: f32| GeometryStyle {
        stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color(PLATE_COLOR)),
        stroke_width: width,
        ..Default::default()
    };
    let ocean = SphereStyle {
        fill: OCEAN,
        stroke: rgb(0x9fb6c9),
        stroke_width: 0.8,
    };
    let earthquakes = project(&layers.earthquakes)?;
    // Only the earthquakes and the countries take hits, so the hit-testing tree holds only them.
    let mut quakes = geometry_marks(earthquakes.as_ref(), &layers.earthquakes_style)?;
    quakes.interactive = Some(true);
    for mark in &mut quakes.marks {
        if let SceneMark::Symbol(symbol) = mark {
            symbol.name = quakes_mark(map);
        }
    }
    let mut content = SceneGroup {
        clip: Clip::Rect {
            x: 0.0,
            y: 0.0,
            width: size[0],
            height: size[1],
        },
        marks: vec![make_sphere_marks(&projector, &ocean).into()],
        interactive: Some(false),
        ..Default::default()
    };
    match tiles {
        Some(tiles) => {
            let zoom = tiles.source().zoom(&projector);
            content.marks.push(tiles.group(&projector, zoom)?.into());
            content.marks.push(view.lines().into());
        }
        None => {
            content.marks.push(view.lines().into());
            let countries = project(&layers.countries)?;
            let mut countries = geometry_marks(countries.as_ref(), &layers.countries_style)?;
            countries.interactive = Some(true);
            for mark in &mut countries.marks {
                if let SceneMark::Path(path) = mark {
                    path.name = countries_mark(map);
                }
            }
            content.marks.push(countries.into());
        }
    }
    content.marks.extend([
        geometry_marks(
            project(&layers.boundaries[0])?.as_ref(),
            &boundary(PLATE_WIDTHS[0]),
        )?
        .into(),
        geometry_marks(
            project(&layers.boundaries[1])?.as_ref(),
            &boundary(PLATE_WIDTHS[1]),
        )?
        .into(),
        quakes.into(),
    ]);
    Ok(Drawn {
        content,
        parts: LABEL_SIDES.iter().map(|&side| view.part(side)).collect(),
    })
}

/// The longitude and latitude of the place under `point` in a map's frame, or `None` off the
/// map.
pub fn readout(projector: &Projector, point: [f32; 2]) -> Option<String> {
    let [x, y] = point.map(f64::from);
    let (lon, lat) = projector.invert(x, y)?;
    // A point off the map inverts to a place that doesn't project back to it.
    let (px, py) = projector.project_point(lon, lat)?;
    if (px - x).abs() > 1e-3 || (py - y).abs() > 1e-3 {
        return None;
    }
    let hemisphere = |value: f64, positive: char, negative: char| {
        format!(
            "{:.1}°{}",
            value.abs(),
            if value < 0.0 { negative } else { positive }
        )
    };
    Some(format!(
        "{}  {}",
        hemisphere(lat, 'N', 'S'),
        hemisphere(lon, 'E', 'W')
    ))
}

/// The legends of the layers' encodings, which stand in a stack on the right of the maps.
pub struct Legends {
    pub continents: Legend,
    pub magnitude: Legend,
    pub depth: Colorbar,
    pub plates: Legend,
}

impl Legends {
    pub fn new(engine: &LabelEngine) -> Result<Self, Box<dyn Error>> {
        let layout = LegendLayout {
            side: GuideSide::Right,
            wrap: LegendWrap::Flow,
        };
        let continents = &CONTINENTS[..7];
        let continents = Legend::new(
            &LegendConfig {
                title: Some("Continent".to_string()),
                text: ScalarOrArray::new_array(
                    continents
                        .iter()
                        .map(|(name, _)| name.to_string())
                        .collect(),
                ),
                swatches: LegendSwatches::Symbols(SymbolSwatches {
                    fill: ScalarOrArray::new_array(
                        continents
                            .iter()
                            .map(|(_, color)| ColorOrGradient::Color(rgb(*color)))
                            .collect(),
                    ),
                    size: ScalarOrArray::new_scalar(90.0),
                    stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color(MUTED)),
                    stroke_width: Some(0.5),
                    ..Default::default()
                }),
                layout,
                ..Default::default()
            },
            engine,
        )?;
        let magnitudes = [6.0, 7.0, 8.0, 9.0];
        let magnitude = Legend::new(
            &LegendConfig {
                title: Some("Magnitude".to_string()),
                text: ScalarOrArray::new_array(magnitudes.map(|m| format!("{m}")).to_vec()),
                swatches: LegendSwatches::Symbols(SymbolSwatches {
                    size: magnitude_scale().scale_to_numeric(
                        &(Arc::new(arrow::array::Float64Array::from(magnitudes.to_vec()))
                            as ArrayRef),
                    )?,
                    fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([1.0; 4])),
                    stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color(INK)),
                    stroke_width: Some(0.8),
                    ..Default::default()
                }),
                layout,
                ..Default::default()
            },
            engine,
        )?;
        let defaults = ColorbarConfig::default();
        let depth = Colorbar::new(
            &depth_scale(),
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
        let plates = Legend::new(
            &LegendConfig {
                title: Some("Plate boundary".to_string()),
                text: ScalarOrArray::new_array(vec![
                    "Subduction zone".to_string(),
                    "Other".to_string(),
                ]),
                swatches: LegendSwatches::Lines(LineSwatches {
                    stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color(PLATE_COLOR)),
                    stroke_width: ScalarOrArray::new_array(vec![PLATE_WIDTHS[1], PLATE_WIDTHS[0]]),
                    line_length: ScalarOrArray::new_scalar(18.0),
                    ..Default::default()
                }),
                layout,
                ..Default::default()
            },
            engine,
        )?;
        Ok(Self {
            continents,
            magnitude,
            depth,
            plates,
        })
    }
}
