//! Earthquakes of magnitude 6 and larger, from 2000 through 2024, on OpenStreetMap's tiles, in a
//! chart app with the `geo` feature.
//!
//! The map is a Web Mercator plot that fits the earthquakes and fills its space, over the tiles.
//! It declares a navigation, and its geo spec limits the zoom: a drag turns the world by longitude
//! and slides it vertically, the wheel zooms about the pointer from the fitted view to 4,096 times
//! its scale, and a double click returns the map to its fit. A tooltip describes the earthquake nearest the pointer. Beside the
//! map, a scatter plot shows the depth of each earthquake against its magnitude, and a box on it
//! selects earthquakes, which both plots show in full while they fade the others.
//!
//! The app draws the tiles through an image cache, which fetches them with the example's
//! User-Agent and keeps them in a disk cache in `target/chart-map-tiles`, as OpenStreetMap's tile
//! usage policy asks, and the map credits them. Each tile draws on the frame after it arrives,
//! over the coarser tile that the cache holds until then. `--tiles <TEMPLATE>` draws the tiles of
//! another server, where `<TEMPLATE>` is a URL with `{z}`, `{x}`, and `{y}`.
//!
//! The earthquakes are the world gallery data of avenger-guides-geo, which Git LFS stores, so run
//! `git lfs pull` when they are pointer files.

use std::io::Cursor;
use std::sync::Arc;

use arrow::array::{Array, AsArray, Float64Array, RecordBatch};
use arrow::compute::concat_batches;
use arrow::datatypes::Float64Type;
use arrow::ipc::reader::StreamReader;
use avenger_chart::{proto, Chart, Encode, Scale, Selection, Value};
use avenger_chart_app::ChartApp;
use avenger_format_datetime_d3::D3DateTimeFormatProvider;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_image::ImageCache;
use avenger_typst_label::LabelEngine;
use avenger_winit_wgpu::WinitWgpuAvengerApp;

const SIZE: [f32; 2] = [1200.0, 720.0];

/// OpenStreetMap's standard tiles.
const OSM_TILES: &str = "https://tile.openstreetmap.org/{z}/{x}/{y}.png";

/// The User-Agent of the tile requests, as OpenStreetMap's tile usage policy asks.
const USER_AGENT: &str = concat!(
    "avenger-chart-map-tiles/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/jonmmease/avenger)"
);

/// The earthquakes, embedded from the world gallery data.
const EARTHQUAKES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../avenger-guides-geo/examples/data/earthquakes.arrows"
));

/// The selection of the box on the scatter plot.
const QUAKES: &str = "quakes";

/// The earthquakes: the magnitude, the depth in kilometers, and the time of each, with the
/// longitude and the latitude of its point.
fn earthquakes() -> RecordBatch {
    assert!(
        !EARTHQUAKES.starts_with(b"version https://git-lfs"),
        "the earthquakes are a Git LFS pointer file: run `git lfs pull`, then build again"
    );
    let reader = StreamReader::try_new(Cursor::new(EARTHQUAKES), None).unwrap();
    let schema = reader.schema();
    let batches = reader.collect::<Result<Vec<_>, _>>().unwrap();
    let table = concat_batches(&schema, &batches).unwrap();
    // Each point is a pair of a longitude and a latitude.
    let points = table
        .column_by_name("geometry")
        .unwrap()
        .as_fixed_size_list();
    let coordinates = points.values().as_primitive::<Float64Type>();
    let coordinate = |axis: usize| {
        Float64Array::from_iter_values(
            (0..points.len())
                .map(|row| coordinates.value(points.value_offset(row) as usize + axis)),
        )
    };
    let column = |name: &str| table.column_by_name(name).unwrap().clone();
    RecordBatch::try_from_iter([
        ("magnitude", column("magnitude")),
        ("depth_km", column("depth_km")),
        ("time", column("time")),
        ("lon", Arc::new(coordinate(0))),
        ("lat", Arc::new(coordinate(1))),
    ])
    .unwrap()
}

/// The earthquakes that the box selects, or every earthquake without a box.
fn selected() -> proto::Predicate {
    proto::Predicate {
        predicate: Some(proto::predicate::Predicate::Selection(
            proto::SelectionTest {
                selection: QUAKES.into(),
                ..Default::default()
            },
        )),
    }
}

/// Full for the earthquakes that the box selects, and faint for the others.
fn opacity() -> Encode {
    Encode::constant(0.12).when(selected(), Value::Constant(0.9.into()))
}

/// The depth of each earthquake, from red near the surface to blue deep below it.
fn depth_color() -> Encode {
    Encode::column("depth_km")
        .scale(Scale::new().palette("redblue"))
        .title("Depth (km)")
}

/// The credit of the tiles at `template`.
fn tile_credit(template: &str) -> String {
    if template == OSM_TILES {
        return "© OpenStreetMap contributors".to_string();
    }
    let host = template.split("://").nth(1).unwrap_or(template);
    format!("Tiles: {}", host.split('/').next().unwrap_or(host))
}

/// The map of `quakes` over the tiles at `template`.
fn map(quakes: RecordBatch, template: &str) -> Chart {
    let geo = proto::Geo {
        surface: Some(proto::geo::Surface::Spherical(proto::Spherical {
            projection: "mercator".into(),
            ..Default::default()
        })),
        fit: Some(proto::geo::Fit::FitData(proto::PlotData {})),
        aspect: Some(proto::geo::Aspect::FreeAspect(proto::FreeAspect {})),
        tiles: Some(proto::TileLayer {
            url_template: template.into(),
            ..Default::default()
        }),
        navigation: Some(proto::GeoNavigation {
            min_zoom: Some(1.0),
            max_zoom: Some(4096.0),
            ..Default::default()
        }),
        ..Default::default()
    };
    let field = |column: &str, title: &str, format: &str, label: &str| proto::TooltipField {
        column: column.into(),
        title: Some(title.into()),
        format: format.into(),
        label: label.into(),
    };
    let tooltip = proto::Tooltip {
        find: Some(proto::tooltip::Find::Nearest(proto::NearestFind {
            max_radius: 12.0,
            ..Default::default()
        })),
        header: Some(field("time", "Date", "%b %-d, %Y", "")),
        fields: vec![
            field("magnitude", "Magnitude", ".1f", ""),
            field("depth_km", "Depth", ".0f", "#value km"),
        ],
        ..Default::default()
    };
    let sizes = Arc::new(Float64Array::from(vec![12.0, 360.0]));
    Chart::unit(quakes, proto::SymbolMark::default())
        .id("map")
        .longitude("lon")
        .latitude("lat")
        .size(
            Encode::column("magnitude")
                .scale(Scale::new().range(sizes))
                .title("Magnitude"),
        )
        .color(depth_color())
        .opacity(opacity())
        .coord(geo)
        .selection(Selection::new(QUAKES, proto::SelectionResolution::Global))
        .tool(proto::Navigate::default())
        .tool(tooltip)
        .attribution(proto::Attribution {
            credits: vec![
                tile_credit(template),
                "Earthquakes: U.S. Geological Survey".to_string(),
            ],
            align: proto::AttributionAlign::End as i32,
            separator: " · ".to_string(),
            ..Default::default()
        })
        .build()
        .unwrap()
}

/// The depth of each of `quakes` against its magnitude, where a box selects earthquakes.
fn scatter(quakes: RecordBatch) -> Chart {
    Chart::unit(quakes, proto::SymbolMark::default())
        .id("scatter")
        .x(Encode::column("magnitude").title("Magnitude"))
        .y(Encode::column("depth_km")
            .scale(Scale::new().reverse())
            .title("Depth (km)"))
        .color(depth_color())
        .opacity(opacity())
        .selection(Selection::new(QUAKES, proto::SelectionResolution::Global))
        .tool(proto::Select {
            selection: QUAKES.into(),
            form: Some(proto::select::Form::Interval(
                proto::IntervalSelect::default(),
            )),
            scope: Vec::new(),
        })
        .build()
        .unwrap()
}

/// The scatter plot, and the map over the tiles at `template` beside it with three times its
/// width.
fn chart(template: &str) -> Chart {
    let quakes = earthquakes();
    let share = |weight| proto::Track {
        size: Some(proto::track::Size::Share(proto::Share {
            weight,
            max: None,
        })),
    };
    Chart::row([scatter(quakes.clone()), map(quakes, template)])
        .layout(proto::ConcatLayout {
            tracks: vec![share(1.0), share(3.0)],
            ..Default::default()
        })
        .title(proto::Title {
            text: "Earthquakes of magnitude 6 and larger, 2000 through 2024".into(),
            subtitle: "Drag the map to pan, scroll to zoom, and double click to return. Drag a \
                       box of magnitudes and depths to select earthquakes."
                .into(),
            ..Default::default()
        })
        .build()
        .unwrap()
}

fn engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
        .with_datetime_formatting(Arc::new(D3DateTimeFormatProvider::new()))
}

/// The cache of the tiles, which fetches them with the example's User-Agent and keeps them on
/// disk under `target/`, as OpenStreetMap's tile usage policy asks.
fn tile_cache() -> ImageCache {
    let fetcher = avenger_image::reqwest_fetcher::ReqwestImageFetcher::new(
        USER_AGENT,
        std::time::Duration::from_secs(30),
    )
    .expect("the tile fetcher builds");
    ImageCache::with_options(avenger_image::ImageCacheOptions {
        fetcher: Arc::new(fetcher),
        disk: Some(avenger_image::DiskCacheOptions::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../target/chart-map-tiles"
        ))),
        ..Default::default()
    })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let template = args
        .iter()
        .position(|arg| arg == "--tiles")
        .and_then(|index| args.get(index + 1))
        .cloned()
        .unwrap_or_else(|| OSM_TILES.to_string());
    let app = ChartApp::new(chart(&template), engine())
        .size(SIZE)
        .images(tile_cache());
    let avenger_app = pollster::block_on(app.build()).expect("Failed to create AvengerApp");
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop(avenger_app, 2.0, tokio_runtime);
    event_loop
        .run_app(&mut app)
        .expect("Failed to run event loop");
}

#[cfg(test)]
mod tests;
