//! Earthquakes on a globe and a world map over OpenStreetMap tiles, built with the builders of
//! avenger-chart and shown by a chart app.
//!
//! The globe, in an orthographic projection, faces the Pacific Ring of Fire, and the world map is
//! in the Mercator projection. Each draws the plate boundaries and the earthquakes of magnitude 6
//! and larger since 2000, sized by magnitude and colored by depth, over the tiles. Each map has its
//! own projection: a drag turns the globe as a trackball turns, and a sideways drag turns the world
//! map. The wheel zooms either.
//!
//! The chart has three modes of the group `drag`, Pan, Box, and Lasso, and a mode widget above the
//! maps chooses one. In the Pan mode, a drag pans a map. In the Box mode, a drag draws a box between two meridians and
//! two parallels, and in the Lasso mode, a drag draws a closed path around earthquakes. The box and
//! the lasso write the selection `quakes`, whose resolution is global, so both maps draw the one
//! box or lasso, and both ring the earthquakes inside it. A click clears it. The wheel zooms in
//! every mode, and a double click returns a map to its first view.
//!
//! The tooltip of an earthquake near the pointer shows its magnitude, its depth, and its date, while
//! the toggle Tooltips is on. A check box beside the modes turns it on and off.
//!
//! The example fetches the tiles with its own User-Agent and keeps them on disk in
//! `target/chart-geo-tiles`, as OpenStreetMap's tile usage policy asks. `--svg <PATH>` writes the
//! chart as SVG, over its tiles, instead of opening a window.

use std::{io::Cursor, sync::Arc, time::Duration};

use arrow::{
    array::{ArrayRef, AsArray, BooleanArray, Float64Array, RecordBatch, StringArray},
    compute::concat_batches,
    datatypes::{DataType, Field, Float64Type, Schema},
    ipc::reader::StreamReader,
};
use avenger_chart::prelude::*;
use avenger_chart_app::{ChartApp, ChartChange, Reports};
use avenger_chart_eval::{ChartEvaluator, Registry};
use avenger_format_datetime_d3::D3DateTimeFormatProvider;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_image::ImageCache;
use avenger_style::{Cascade, Env, HostInputs, ParseOptions, Sheet, SourceId, StyleTable};
use avenger_svg::{AvengerSvgError, SvgRenderer};
use avenger_typst_label::LabelEngine;
use avenger_winit_wgpu::WinitWgpuAvengerApp;

/// The layers of the world gallery of avenger-guides-geo, which Git LFS stores.
const EARTHQUAKES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../avenger-guides-geo/examples/data/earthquakes.arrows"
));
const PLATES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../avenger-guides-geo/examples/data/plate_boundaries.arrows"
));

/// OpenStreetMap's tiles.
const OSM_TILES: &str = "https://tile.openstreetmap.org/{z}/{x}/{y}.png";

/// The User-Agent of the tile requests, as OpenStreetMap's tile usage policy asks.
const USER_AGENT: &str = concat!(
    "avenger-chart-geo/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/jonmmease/avenger)"
);

/// The selection of the box and the lasso.
const QUAKES: &str = "quakes";

/// The size of the window.
const SIZE: [f32; 2] = [1200.0, 540.0];

/// Earthquake depth, shallow to deep.
const DEPTHS: [&str; 5] = ["#f46d43", "#d53e4f", "#9e0142", "#5e3c99", "#2d004b"];

/// The color of the rings around the selected earthquakes.
const INK: &str = "#172b42";

/// The layer `name`, an IPC stream, as one record batch.
fn read_layer(name: &str, bytes: &[u8]) -> RecordBatch {
    assert!(
        !bytes.starts_with(b"version https://git-lfs"),
        "the {name} layer is a Git LFS pointer file: run `git lfs pull`, then build again"
    );
    let reader = StreamReader::try_new(Cursor::new(bytes), None).unwrap();
    let schema = reader.schema();
    let batches = reader.collect::<Result<Vec<_>, _>>().unwrap();
    concat_batches(&schema, &batches).unwrap()
}

/// The earthquakes: the longitude and the latitude of each, its magnitude, its depth in
/// kilometers, and its time, largest first, so that small ones draw on top.
fn earthquakes() -> RecordBatch {
    let quakes = read_layer("earthquakes", EARTHQUAKES);
    let column = |name: &str| quakes.column_by_name(name).unwrap().clone();
    // The earthquakes are native points with interleaved coordinates.
    let points = column("geometry");
    let coordinates = (points.as_fixed_size_list().values()).as_primitive::<Float64Type>();
    let [lon, lat]: [Float64Array; 2] = [0, 1].map(|axis| {
        (coordinates.values().iter().skip(axis).step_by(2))
            .copied()
            .collect()
    });
    RecordBatch::try_from_iter([
        ("lon", Arc::new(lon) as ArrayRef),
        ("lat", Arc::new(lat)),
        ("magnitude", column("magnitude")),
        ("depth_km", column("depth_km")),
        ("time", column("time")),
    ])
    .unwrap()
}

/// The plate boundaries: the line of each, as WKB, and whether it is a subduction zone.
fn plates() -> RecordBatch {
    let plates = read_layer("plate boundaries", PLATES);
    let subduction: &BooleanArray = plates.column_by_name("subduction").unwrap().as_boolean();
    let boundary: StringArray = (subduction.iter())
        .map(|subduction| match subduction {
            Some(true) => Some("Subduction zone"),
            _ => Some("Other"),
        })
        .collect();
    // The field of the geometry keeps its GeoArrow extension type.
    let schema = plates.schema();
    let geometry = schema.index_of("geometry").unwrap();
    let schema = Schema::new(vec![
        schema.field(geometry).clone(),
        Field::new("boundary", DataType::Utf8, false),
    ]);
    let columns = vec![
        plates.column(geometry).clone(),
        Arc::new(boundary) as ArrayRef,
    ];
    RecordBatch::try_new(Arc::new(schema), columns).unwrap()
}

/// The tooltip of an earthquake near the pointer: its magnitude, its depth, and its date.
fn tooltip() -> TooltipBuilder {
    let field = |column: &str, title: &str, format: &str| {
        TooltipField::builder()
            .column(column)
            .title(title)
            .labels(Labels::builder().format(format))
    };
    Tooltip::nearest(NearestFind::builder().max_radius(10.0)).fields([
        field("magnitude", "Magnitude", ".1f"),
        field("depth_km", "Depth (km)", ",.0f"),
        field("time", "Date", "%Y-%m-%d"),
    ])
}

/// The layers of a map `id`: the plate boundaries, colored by kind, and the earthquakes, sized by
/// magnitude, colored by depth, and ringed inside the selection, with the tooltip of an earthquake
/// while the toggle `tooltips` is on, and the widgets of the chart's modes.
fn layers(id: &str, plates: &RecordBatch, quakes: &RecordBatch) -> [GeoLayerBuilder; 2] {
    let boundaries = GeoLayer::shape(
        plates.clone(),
        GeoShapeMark::new("geometry")
            .stroke(
                Encoding::scaled_field("boundary").scale(
                    Scale::builder()
                        .range(Range::values(vec!["#b32929", "#e39a9a"]))
                        .legend(Legend::builder().title("Plate boundary")),
                ),
            )
            .stroke_width(1.2),
    );
    // A selection without clauses selects no earthquake, so no ring draws before the first box.
    let selected = || Predicate::selected(Selected::new(QUAKES).empty_selects_none(true));
    let magnitude = Scale::builder()
        .domain(Domain::data(DataDomain::builder().fit(
            DomainFit::interval(IntervalFit::builder().zero(false).nice(false)),
        )))
        .range(Range::values(vec![4.0, 140.0]))
        .legend(Legend::builder().title(Title::new("Magnitude $M_w$").syntax(TextSyntax::Typst)));
    let depth = Scale::builder()
        .range(Range::values(DEPTHS.to_vec()))
        .legend(Legend::builder().title("Depth (km)"));
    let earthquakes = GeoLayer::symbol(
        quakes.clone(),
        GeoSymbolMark::builder()
            .longitude("lon")
            .latitude("lat")
            .size(Encoding::scaled_field("magnitude").scale(magnitude))
            .color(Encoding::scaled_field("depth_km").scale(depth))
            .stroke(
                Encoding::direct_value("white")
                    .conditional([ConditionalValue::direct_value(selected(), INK)]),
            )
            .stroke_width(
                Encoding::direct_value(0.4)
                    .conditional([ConditionalValue::direct_value(selected(), 2.0)]),
            ),
    )
    .id(format!("{id} quakes"))
    .tools([LayerToolBuilder::from(tooltip()).modes(["tooltips"])])
    .widgets([
        Widget::mode("drag", ModeWidget::group("drag")),
        Widget::mode("tooltips", ModeWidget::toggle("tooltips")),
    ]);
    [boundaries, earthquakes]
}

/// The tools of a map: a navigation in the Pan mode, a box in the Box mode, and a lasso in the
/// Lasso mode, which write the one selection `quakes`.
fn tools() -> [ToolBuilder; 3] {
    [
        ToolBuilder::from(Navigate::builder()).modes(["pan"]),
        ToolBuilder::from(Select::interval(QUAKES, IntervalSelect::builder())).modes(["box"]),
        ToolBuilder::from(Select::lasso(QUAKES, Vec::<String>::new())).modes(["lasso"]),
    ]
}

/// The lines of longitude and latitude every 30 degrees.
fn graticule() -> GraticuleBuilder {
    Graticule::builder()
        .longitude_step(30.0)
        .latitude_step(30.0)
}

/// A map `id` titled `title` in the projection of `spherical`, with its graticule, which pans in
/// `pan_mode` and zooms out to `min_zoom` of its fitted scale, over OpenStreetMap's tiles.
fn map(
    id: &str,
    title: &str,
    spherical: SphericalBuilder,
    pan_mode: PanMode,
    min_zoom: f64,
) -> PlotBuilder {
    let options = Geo::spherical(
        spherical
            .sphere(SphereStyle::builder())
            .tiles(TileLayer::new(OSM_TILES)),
    )
    .navigation(
        GeoNavigation::builder()
            .pan_mode(pan_mode)
            .min_zoom(min_zoom)
            .max_zoom(256.0),
    );
    let (plates, quakes) = (plates(), earthquakes());
    Plot::geo(GeoPlot::new(options, layers(id, &plates, &quakes)))
        .id(id)
        .title(title)
        .tools(tools())
        .attribution(Attribution::builder().credits(["© OpenStreetMap contributors"]))
}

/// The chart: the globe beside the world map, with the modes Pan, Box, and Lasso of the group
/// `drag`, and the toggle Tooltips.
fn chart() -> Chart {
    let globe = map(
        "globe",
        "Pacific Ring of Fire",
        // The globe's lines are unlabeled, since their labels crowd its edge.
        Spherical::orthographic()
            .rotate(Rotation::builder().lambda(170.0).phi(-10.0))
            .fit_sphere()
            .graticule(graticule().label_sides(LabelSides::builder())),
        PanMode::Sphere,
        0.6,
    );
    // The world map fits the latitudes of the earthquakes, and zooms out no further.
    let world = map(
        "world",
        "The world",
        Spherical::mercator()
            .fit_box(
                LonLatBox::builder()
                    .west(-180.0)
                    .south(-65.0)
                    .east(180.0)
                    .north(80.0),
            )
            .graticule(graticule()),
        PanMode::Longitude,
        1.0,
    );
    // The world map's column is the wider, since the map is wider than it is tall.
    let share = |weight: f32| Track::builder().share(Share::builder().weight(weight));
    let layout = ConcatLayout::builder().tracks([share(1.0), share(1.6)]);
    Chart::builder(
        Concat::new(Direction::Row)
            .layout(layout)
            .children([globe, world]),
    )
    .number_format_syntax("d3")
    .datetime_format_syntax("d3")
    .selections([Selection::new(QUAKES, SelectionResolution::Global)])
    .modes([
        Mode::new("pan").label("Pan").group("drag"),
        Mode::new("box").label("Box").group("drag"),
        Mode::new("lasso").label("Lasso").group("drag"),
        Mode::new("tooltips").label("Tooltips").on(true),
    ])
    .build()
    .unwrap()
}

fn engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
        .with_datetime_formatting(Arc::new(D3DateTimeFormatProvider::new()))
}

/// The cascade of the base stylesheet.
fn cascade() -> Arc<Cascade> {
    let mut no_files = |url: &str, _: Option<&str>| Err(format!("no file `{url}`"));
    let options = ParseOptions {
        allow_import: true,
        ..Default::default()
    };
    let (sheet, diagnostics) = Sheet::load(
        "@import 'std:base.css';",
        None,
        SourceId(1),
        options,
        &mut no_files,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    Arc::new(Cascade::new(
        &StyleTable::new().with_author(sheet, Env::new()),
    ))
}

/// The cache of the tiles, which fetches them with the example's User-Agent and keeps them on disk
/// under `target/`, as OpenStreetMap's tile usage policy asks.
fn tile_cache() -> ImageCache {
    let fetcher = avenger_image::reqwest_fetcher::ReqwestImageFetcher::new(
        USER_AGENT,
        Duration::from_secs(30),
    )
    .expect("the tile fetcher builds");
    ImageCache::with_options(avenger_image::ImageCacheOptions {
        fetcher: Arc::new(fetcher),
        disk: Some(avenger_image::DiskCacheOptions::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../target/chart-geo-tiles"
        ))),
        ..Default::default()
    })
}

/// Prints the modes that are on after a click on a mode widget, and the count of the conditions
/// of the selection when a box or a lasso settles.
fn report(change: &ChartChange) {
    if let Some(modes) = &change.modes {
        println!("modes: {}", modes.join(", "));
    }
    if let Some(clauses) = change.selection(QUAKES) {
        match clauses.first() {
            Some(clause) => println!("{QUAKES}: {} conditions", clause.conditions.len()),
            None => println!("{QUAKES}: cleared"),
        }
    }
}

/// Writes the chart at `SIZE` as SVG to `path`, over its tiles, which it waits for.
fn write_svg(path: &str) {
    let engine = engine();
    let cache = tile_cache();
    let mut evaluator = ChartEvaluator::new(chart(), cascade(), engine.clone(), Registry::new())
        .expect("the chart prepares");
    evaluator.set_image_resolver(Arc::new(cache.clone()));
    let [width, height] = SIZE;
    let scene = (evaluator.evaluate(&HostInputs::sized(width, height)))
        .expect("the chart evaluates")
        .scene;
    let svg = loop {
        let rendered = SvgRenderer::new()
            .with_images(Arc::new(cache.clone()))
            .render_scene_graph(&scene, &engine);
        match rendered {
            Ok(svg) => break svg,
            // The tiles that the scene draws load in the background.
            Err(AvengerSvgError::ImagesNotReady(urls)) => {
                let urls: Vec<&str> = urls.iter().map(String::as_str).collect();
                if let Err(error) = cache.wait(&urls, Duration::from_secs(60)) {
                    panic!("the tiles did not load: {error:?}");
                }
            }
            Err(error) => panic!("{error}"),
        }
    };
    std::fs::write(path, svg).unwrap();
}

fn main() {
    if let [_, flag, path] = std::env::args().collect::<Vec<_>>().as_slice() {
        if flag == "--svg" {
            write_svg(path);
            return;
        }
    }
    let app = ChartApp::new(chart(), cascade(), engine())
        .images(tile_cache())
        .size(SIZE)
        .reports(Reports::Settled)
        .on_change(report);
    let avenger_app = pollster::block_on(app.build()).expect("Failed to create AvengerApp");
    let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let avenger_app =
        (avenger_app.with_runtime(tokio_runtime.handle().clone())).expect("a multi-thread runtime");
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop(avenger_app, 2.0, tokio_runtime);
    event_loop
        .run_app(&mut app)
        .expect("Failed to run event loop");
}

#[cfg(test)]
mod tests;
