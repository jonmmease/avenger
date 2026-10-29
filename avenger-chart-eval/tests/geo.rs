//! Map plots, with the `geo` feature, evaluated and rendered at two sizes and compared with image
//! baselines.
//!
//! A changed image is written to `tests/output/`; accept it by copying it to `tests/baselines/`.

use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array, RecordBatch, StringArray};
use arrow::datatypes::{Field, Schema};
use avenger_chart::{payload, proto, Chart, Encode};
use avenger_chart_eval::{ChartEvaluator, EvaluatedChart, PlotCoord, Theme};
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_render_fixtures::raster::{assert_baseline, svg_to_png};
use avenger_scales::scalar::Scalar;
use avenger_scenegraph::marks::{mark::SceneMark, symbol::SceneSymbolMark};
use avenger_svg::SvgRenderer;
use avenger_typst_label::{EngineOptions, LabelEngine};
use geo_types::{Geometry, LineString, Point, Polygon};
use geoarrow_array::builder::WkbBuilder;
use geoarrow_array::GeoArrowArray;
use geoarrow_schema::WkbType;

fn engine() -> LabelEngine {
    LabelEngine::new(EngineOptions {
        fonts: avenger_render_fixtures::scene::fonts(),
    })
    .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
}

/// Evaluates `chart` at `size`, renders it, and compares it with the baseline `name`.
fn render(chart: &Chart, size: [f32; 2], name: &str) -> EvaluatedChart {
    let engine = engine();
    let mut evaluator = ChartEvaluator::new(chart, &engine, &Theme::default()).unwrap();
    let evaluated = evaluator.evaluate(size).unwrap();
    let svg = SvgRenderer::new()
        .render_scene_graph(&evaluated.scene, &engine)
        .unwrap();
    assert_baseline(env!("CARGO_MANIFEST_DIR"), &svg_to_png(&svg, 2.0), name);
    evaluated
}

/// Renders `chart` at 320 by 240 and at 640 by 360, writing both images before a failure of
/// either.
fn render_both(chart: &Chart, name: &str) {
    render_sizes(chart, &[[320.0, 240.0], [640.0, 360.0]], name);
}

/// Renders `chart` at each of `sizes`, writing every image before a failure of any.
fn render_sizes(chart: &Chart, sizes: &[[f32; 2]], name: &str) {
    let results: Vec<_> = sizes
        .iter()
        .map(|size: &[f32; 2]| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                render(chart, *size, &format!("{name}-{}x{}", size[0], size[1]));
            }))
        })
        .collect();
    for result in results {
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }
}

fn evaluate(chart: &Chart, size: [f32; 2]) -> EvaluatedChart {
    let engine = engine();
    let mut evaluator = ChartEvaluator::new(chart, &engine, &Theme::default()).unwrap();
    evaluator.evaluate(size).unwrap()
}

/// A polygon through the longitudes and latitudes of `ring`, which closes it.
fn polygon(ring: &[(f64, f64)]) -> Polygon<f64> {
    let mut ring = ring.to_vec();
    ring.push(ring[0]);
    Polygon::new(LineString::from(ring), Vec::new())
}

/// Rough outlines of five continents, as a WKB GeoArrow column beside their names.
fn continents() -> RecordBatch {
    let shapes = [
        (
            "Africa",
            polygon(&[
                (-17.0, 21.0),
                (-6.0, 35.0),
                (11.0, 37.0),
                (32.0, 31.0),
                (43.0, 12.0),
                (51.0, 11.0),
                (40.0, -15.0),
                (20.0, -35.0),
                (12.0, -6.0),
                (-17.0, 15.0),
            ]),
        ),
        (
            "South America",
            polygon(&[
                (-80.0, 9.0),
                (-60.0, 11.0),
                (-35.0, -6.0),
                (-41.0, -22.0),
                (-68.0, -55.0),
                (-74.0, -45.0),
                (-70.0, -18.0),
                (-81.0, -5.0),
            ]),
        ),
        (
            "North America",
            polygon(&[
                (-165.0, 65.0),
                (-140.0, 70.0),
                (-80.0, 73.0),
                (-55.0, 52.0),
                (-80.0, 25.0),
                (-97.0, 16.0),
                (-118.0, 32.0),
                (-125.0, 48.0),
            ]),
        ),
        (
            "Eurasia",
            polygon(&[
                (-10.0, 36.0),
                (-9.0, 43.0),
                (5.0, 58.0),
                (30.0, 70.0),
                (100.0, 77.0),
                (180.0, 68.0),
                (140.0, 40.0),
                (105.0, 10.0),
                (78.0, 8.0),
                (55.0, 25.0),
                (35.0, 36.0),
            ]),
        ),
        (
            "Australia",
            polygon(&[
                (114.0, -22.0),
                (130.0, -12.0),
                (145.0, -15.0),
                (153.0, -28.0),
                (146.0, -39.0),
                (116.0, -35.0),
            ]),
        ),
    ];
    let mut builder = WkbBuilder::<i32>::new(WkbType::default());
    for (_, shape) in &shapes {
        builder
            .push_geometry(Some(&Geometry::Polygon(shape.clone())))
            .unwrap();
    }
    let geometry = builder.finish();
    let field = geometry.data_type().to_field("geometry", true);
    let names: ArrayRef = Arc::new(StringArray::from(
        shapes.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
    ));
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("name", names.data_type().clone(), false),
            field,
        ])),
        vec![names, geometry.into_array_ref()],
    )
    .unwrap()
}

/// Cities around the world with their longitudes, latitudes, and populations in millions.
fn cities() -> RecordBatch {
    let rows = [
        ("London", -0.13, 51.51, 9.0),
        ("New York", -74.01, 40.71, 19.6),
        ("Los Angeles", -118.24, 34.05, 12.5),
        ("São Paulo", -46.63, -23.55, 22.4),
        ("Lagos", 3.38, 6.52, 15.4),
        ("Cairo", 31.24, 30.04, 21.3),
        ("Mumbai", 72.88, 19.08, 20.9),
        ("Beijing", 116.41, 39.9, 21.3),
        ("Tokyo", 139.69, 35.69, 37.2),
        ("Sydney", 151.21, -33.87, 5.3),
        ("Anchorage", -149.9, 61.22, 0.3),
        ("Honolulu", -157.86, 21.31, 1.0),
    ];
    RecordBatch::try_from_iter([
        (
            "name",
            Arc::new(StringArray::from(
                rows.iter().map(|row| row.0).collect::<Vec<_>>(),
            )) as ArrayRef,
        ),
        (
            "lon",
            Arc::new(Float64Array::from(
                rows.iter().map(|row| row.1).collect::<Vec<_>>(),
            )),
        ),
        (
            "lat",
            Arc::new(Float64Array::from(
                rows.iter().map(|row| row.2).collect::<Vec<_>>(),
            )),
        ),
        (
            "population",
            Arc::new(Float64Array::from(
                rows.iter().map(|row| row.3).collect::<Vec<_>>(),
            )),
        ),
    ])
    .unwrap()
}

/// A spherical projection with the typetag name `name` and its parameters.
fn spherical(name: &str, parameters: &[(&str, &[f64])]) -> proto::geo::Surface {
    proto::geo::Surface::Spherical(proto::Spherical {
        projection: name.into(),
        parameters: parameters
            .iter()
            .map(|(name, values)| proto::NamedOption {
                name: name.to_string(),
                value: Some(payload::encode_option(&Scalar::from_f64_list(values)).unwrap()),
            })
            .collect(),
        ..Default::default()
    })
}

fn sphere_fit() -> Option<proto::geo::Fit> {
    Some(proto::geo::Fit::FitSphere(proto::WholeSphere {}))
}

fn data_fit() -> Option<proto::geo::Fit> {
    Some(proto::geo::Fit::FitData(proto::PlotData {}))
}

/// An Equal Earth world map with a graticule and the sphere.
fn world() -> proto::Geo {
    proto::Geo {
        surface: Some(spherical("equal_earth", &[])),
        fit: sphere_fit(),
        graticule: Some(proto::Graticule {
            longitude_step: Some(30.0),
            latitude_step: Some(30.0),
            ..Default::default()
        }),
        sphere: Some(proto::SphereStyle::default()),
        ..Default::default()
    }
}

fn land() -> proto::GeoshapeMark {
    proto::GeoshapeMark::default()
}

/// The continents and the cities on a map of `geo`.
fn continents_and_cities(geo: proto::Geo) -> Chart {
    let land = Chart::unit(continents(), land())
        .geometry("geometry")
        .fill(Encode::constant("#c9c3b5"))
        .coord(geo.clone())
        .build()
        .unwrap();
    let places = Chart::unit(cities(), proto::SymbolMark::default())
        .longitude("lon")
        .latitude("lat")
        .size("population")
        .color(Encode::constant("#e45756"))
        .coord(geo)
        .build()
        .unwrap();
    Chart::overlay([land, places]).build().unwrap()
}

#[test]
fn an_equal_earth_world_map() {
    let chart = continents_and_cities(world());
    render_both(&chart, "map-equal-earth");
    let evaluated = evaluate(&chart, [640.0, 360.0]);
    let PlotCoord::Geo { projection } = &evaluated.plots[0].coord else {
        panic!("a map plot has the geo coordinate system");
    };
    assert!(projection.view.is_some());
    // Every city projects, and its symbol has its row.
    let symbols = plot_symbols(&evaluated, "root/1");
    assert_eq!(symbols.len, 12);
    assert_eq!(evaluated.data.rows("root/1").unwrap().num_rows(), 12);
    assert!(evaluated.data.rows("root/0/polygons").is_some());
    assert!(evaluated.warnings.is_empty(), "{:?}", evaluated.warnings);
}

#[test]
fn a_globe_draws_no_place_behind_it() {
    let globe = proto::Geo {
        surface: Some(proto::geo::Surface::Spherical(proto::Spherical {
            projection: "orthographic".into(),
            rotate: Some(proto::Rotation {
                lambda: 20.0,
                phi: -30.0,
                gamma: 0.0,
            }),
            ..Default::default()
        })),
        fit: sphere_fit(),
        graticule: Some(proto::Graticule {
            longitude_step: Some(30.0),
            latitude_step: Some(30.0),
            label_sides: Some(proto::LabelSides { sides: Vec::new() }),
            ..Default::default()
        }),
        sphere: Some(proto::SphereStyle::default()),
        ..Default::default()
    };
    let chart = continents_and_cities(globe);
    render_both(&chart, "map-orthographic");
    let evaluated = evaluate(&chart, [320.0, 240.0]);
    let symbols = plot_symbols(&evaluated, "root/1");
    let rows = evaluated.data.rows("root/1").unwrap();
    // The globe faces the Atlantic, so East Asia, Australia, and the Pacific are behind it.
    assert!(symbols.len > 0 && symbols.len < 12, "{}", symbols.len);
    assert_eq!(rows.num_rows(), symbols.len as usize);
    let names = rows.column_by_name("name").unwrap();
    let names = names.as_any().downcast_ref::<StringArray>().unwrap();
    let names: Vec<&str> = names.iter().flatten().collect();
    assert!(
        names.contains(&"Cairo") && !names.contains(&"Tokyo") && !names.contains(&"Sydney"),
        "{names:?}"
    );
}

#[test]
fn a_conic_map_of_a_box_clips_its_marks() {
    let europe = proto::Geo {
        surface: Some(spherical(
            "conic_conformal",
            &[("parallels", &[35.0, 65.0])],
        )),
        fit: Some(proto::geo::Fit::FitBox(proto::LonLatBox {
            west: -12.0,
            south: 34.0,
            east: 40.0,
            north: 62.0,
        })),
        graticule: Some(proto::Graticule {
            longitude_step: Some(10.0),
            latitude_step: Some(10.0),
            ..Default::default()
        }),
        ..Default::default()
    };
    let chart = continents_and_cities(europe);
    render_both(&chart, "map-conic-box");
    let evaluated = evaluate(&chart, [320.0, 240.0]);
    let SceneMark::Group(plot) = &evaluated.scene.marks[0] else {
        panic!("the first mark is the plot");
    };
    assert!(matches!(
        plot.clip,
        avenger_scenegraph::marks::group::Clip::Rect { .. }
    ));
}

/// Two flights from London to Los Angeles and a rule along the Tropic of Cancer.
fn routes() -> Chart {
    let geo = proto::Geo {
        surface: Some(spherical("equal_earth", &[])),
        fit: sphere_fit(),
        sphere: Some(proto::SphereStyle::default()),
        ..Default::default()
    };
    let flights = RecordBatch::try_from_iter([
        (
            "lon",
            Arc::new(Float64Array::from(vec![-0.13, -118.24])) as ArrayRef,
        ),
        ("lat", Arc::new(Float64Array::from(vec![51.51, 34.05]))),
    ])
    .unwrap();
    let flight = |route: proto::Route, color: &str| {
        Chart::unit(
            flights.clone(),
            proto::LineMark {
                route: route as i32,
            },
        )
        .longitude("lon")
        .latitude("lat")
        .color(Encode::constant(color))
        .coord(geo.clone())
        .build()
        .unwrap()
    };
    let tropic = RecordBatch::try_from_iter([
        (
            "lon",
            Arc::new(Float64Array::from(vec![-180.0])) as ArrayRef,
        ),
        ("lon2", Arc::new(Float64Array::from(vec![180.0]))),
        ("lat", Arc::new(Float64Array::from(vec![23.44]))),
    ])
    .unwrap();
    let rule = Chart::unit(tropic, proto::RuleMark::default())
        .longitude("lon")
        .longitude2("lon2")
        .latitude("lat")
        .color(Encode::constant("#54a24b"))
        .coord(geo.clone())
        .build()
        .unwrap();
    let land = Chart::unit(continents(), land())
        .geometry("geometry")
        .fill(Encode::constant("#e0dcd2"))
        .coord(geo.clone())
        .build()
        .unwrap();
    Chart::overlay([
        land,
        rule,
        flight(proto::Route::Coordinate, "#4c78a8"),
        flight(proto::Route::Canvas, "#f58518"),
    ])
    .build()
    .unwrap()
}

#[test]
fn routes_follow_the_great_circle_the_canvas_and_the_parallel() {
    let chart = routes();
    render_both(&chart, "map-routes");
    let evaluated = evaluate(&chart, [640.0, 360.0]);
    let lines = |name: &str| -> Vec<(f32, f32)> {
        let SceneMark::Group(plot) = &evaluated.scene.marks[0] else {
            panic!("the first mark is the plot");
        };
        plot.marks
            .iter()
            .find_map(|mark| match mark {
                SceneMark::Line(line) if line.name == name => Some(
                    line.x_iter()
                        .zip(line.y_iter())
                        .map(|(x, y)| (*x, *y))
                        .collect(),
                ),
                _ => None,
            })
            .unwrap()
    };
    // The great circle bends north of the straight line between its ends, and the canvas route
    // is that straight line.
    let circle = lines("root/2/0");
    let canvas = lines("root/3/0");
    assert_eq!(canvas.len(), 2);
    assert!(circle.len() > 2);
    let highest = circle.iter().map(|(_, y)| *y).fold(f32::INFINITY, f32::min);
    assert!(
        highest < canvas[0].1.min(canvas[1].1) - 5.0,
        "{highest} {canvas:?}"
    );
    // Each vertex of a line has a row: the first row of its segment.
    assert_eq!(
        evaluated.data.rows("root/2/0").unwrap().num_rows(),
        circle.len()
    );
    // The rule follows its parallel, which bends on Equal Earth.
    let tropic = lines("root/1/0");
    assert!(tropic.len() > 2);
}

/// The symbol marks of the plot of an evaluation named `name`.
fn plot_symbols<'a>(evaluated: &'a EvaluatedChart, name: &str) -> &'a SceneSymbolMark {
    fn find<'a>(marks: &'a [SceneMark], name: &str) -> Option<&'a SceneSymbolMark> {
        marks.iter().find_map(|mark| match mark {
            SceneMark::Symbol(symbol) if symbol.name == name => Some(symbol),
            SceneMark::Group(group) => find(&group.marks, name),
            _ => None,
        })
    }
    find(&evaluated.scene.marks, name).unwrap()
}

#[test]
fn symbols_at_the_edge_of_the_data_stay_inside_the_plot() {
    let chart = Chart::unit(cities(), proto::SymbolMark::default())
        .longitude("lon")
        .latitude("lat")
        .size(Encode::constant(900.0))
        .coord(proto::Geo {
            surface: Some(spherical("equal_earth", &[])),
            fit: data_fit(),
            ..Default::default()
        })
        .build()
        .unwrap();
    for size in [[320.0, 240.0], [640.0, 360.0]] {
        let evaluated = evaluate(&chart, size);
        let [_, _, width, height] = evaluated.plots[0].rect;
        let symbols = plot_symbols(&evaluated, "root/0");
        let radius = 15.0;
        for (x, y) in symbols.x_iter().zip(symbols.y_iter()) {
            assert!(
                x - radius >= -0.5
                    && x + radius <= width + 0.5
                    && y - radius >= -0.5
                    && y + radius <= height + 0.5,
                "({x}, {y}) in {width} by {height}"
            );
        }
        assert!(evaluated.warnings.is_empty(), "{:?}", evaluated.warnings);
    }
}

/// A map of the cities of `table` that fits its data.
fn city_map(table: RecordBatch) -> Chart {
    Chart::unit(table, proto::SymbolMark::default())
        .longitude("lon")
        .latitude("lat")
        .coord(proto::Geo {
            surface: Some(spherical("equal_earth", &[])),
            fit: data_fit(),
            ..Default::default()
        })
        .build()
        .unwrap()
}

#[test]
fn maps_that_share_a_projection_take_one_size_and_one_view() {
    let all = cities();
    let west = all.slice(0, 4);
    let east = all.slice(6, 4);
    let chart = Chart::row([city_map(west), city_map(east)])
        .projection(proto::Sharing::Shared)
        .build()
        .unwrap();
    let evaluated = evaluate(&chart, [640.0, 300.0]);
    let [a, b] = [&evaluated.plots[0], &evaluated.plots[1]];
    assert!((a.rect[2] - b.rect[2]).abs() < 1e-3 && (a.rect[3] - b.rect[3]).abs() < 1e-3);
    assert_eq!(view(a), view(b));

    // Without the rule, each map fits its own cities.
    let independent = Chart::row([city_map(all.slice(0, 4)), city_map(all.slice(6, 4))])
        .build()
        .unwrap();
    let evaluated = evaluate(&independent, [640.0, 300.0]);
    assert_ne!(view(&evaluated.plots[0]), view(&evaluated.plots[1]));
}

#[test]
fn a_map_keeps_its_aspect_and_a_free_map_fills_its_space() {
    let geo = |aspect| proto::Geo {
        surface: Some(spherical("equal_earth", &[])),
        fit: sphere_fit(),
        aspect,
        ..Default::default()
    };
    let map = |aspect| {
        Chart::unit(cities(), proto::SymbolMark::default())
            .longitude("lon")
            .latitude("lat")
            .coord(geo(aspect))
            .build()
            .unwrap()
    };
    // Equal Earth's world is about 2.05 times as wide as it is tall.
    let kept = evaluate(&map(None), [640.0, 200.0]);
    let [_, _, width, height] = kept.plots[0].rect;
    assert!((width / height - 2.05).abs() < 0.05, "{width} by {height}");
    assert!(width < 500.0, "{width}");

    let free = evaluate(
        &map(Some(proto::geo::Aspect::FreeAspect(proto::FreeAspect {}))),
        [640.0, 200.0],
    );
    let [_, _, width, height] = free.plots[0].rect;
    assert!(width > 600.0, "{width}");
    let PlotCoord::Geo { projection } = &free.plots[0].coord else {
        panic!("a map plot");
    };
    // The view centers the world across the wide plot.
    let center = projection.build().unwrap().project_point(0.0, 0.0).unwrap();
    assert!(
        (center.0 - f64::from(width) / 2.0).abs() < 1.0,
        "{center:?} in {width}"
    );
    assert!(
        (center.1 - f64::from(height) / 2.0).abs() < 1.0,
        "{center:?} in {height}"
    );
}

#[test]
fn a_map_with_tiles_needs_an_image_resolver() {
    let chart = Chart::unit(cities(), proto::SymbolMark::default())
        .longitude("lon")
        .latitude("lat")
        .coord(proto::Geo {
            surface: Some(spherical("mercator", &[])),
            fit: data_fit(),
            tiles: Some(proto::TileLayer {
                url_template: "https://tile.openstreetmap.org/{z}/{x}/{y}.png".into(),
                ..Default::default()
            }),
            ..Default::default()
        })
        .build()
        .unwrap();
    let engine = engine();
    let mut evaluator = ChartEvaluator::new(&chart, &engine, &Theme::default()).unwrap();
    let error = evaluator
        .evaluate([320.0, 240.0])
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("set_image_resolver"), "{error}");
}

#[test]
fn a_planar_map_draws_projected_coordinates() {
    let points = RecordBatch::try_from_iter([
        (
            "x",
            Arc::new(Float64Array::from(vec![0.0, 100.0, 50.0])) as ArrayRef,
        ),
        ("y", Arc::new(Float64Array::from(vec![0.0, 50.0, 100.0]))),
    ])
    .unwrap();
    let chart = Chart::unit(points, proto::SymbolMark::default())
        .longitude("x")
        .latitude("y")
        .coord(proto::Geo {
            surface: Some(proto::geo::Surface::Planar(proto::Planar {
                reflect_y: false,
            })),
            fit: data_fit(),
            aspect: Some(proto::geo::Aspect::FreeAspect(proto::FreeAspect {})),
            ..Default::default()
        })
        .build()
        .unwrap();
    let evaluated = evaluate(&chart, [300.0, 300.0]);
    let symbols = plot_symbols(&evaluated, "root/0");
    let ys: Vec<f32> = symbols.y_iter().collect();
    // Without a reflection, y grows downward, as the coordinates give it.
    assert!(ys[0] < ys[1] && ys[1] < ys[2], "{ys:?}");
}

#[test]
fn a_point_geometry_insets_the_fit_by_its_symbol() {
    let mut builder = WkbBuilder::<i32>::new(WkbType::default());
    for (lon, lat) in [(-60.0, -30.0), (60.0, 40.0)] {
        builder
            .push_geometry(Some(&Geometry::Point(Point::new(lon, lat))))
            .unwrap();
    }
    let geometry = builder.finish();
    let field = geometry.data_type().to_field("geometry", true);
    let table = RecordBatch::try_new(
        Arc::new(Schema::new(vec![field])),
        vec![geometry.into_array_ref()],
    )
    .unwrap();
    let chart = Chart::unit(table, proto::GeoshapeMark::default())
        .geometry("geometry")
        .size(Encode::constant(400.0))
        .coord(proto::Geo {
            surface: Some(spherical("equal_earth", &[])),
            fit: data_fit(),
            aspect: Some(proto::geo::Aspect::FreeAspect(proto::FreeAspect {})),
            ..Default::default()
        })
        .build()
        .unwrap();
    let evaluated = evaluate(&chart, [300.0, 200.0]);
    let [_, _, width, height] = evaluated.plots[0].rect;
    let symbols = plot_symbols(&evaluated, "root/0/points");
    for (x, y) in symbols.x_iter().zip(symbols.y_iter()) {
        assert!(
            x >= 10.0 && x <= width - 10.0 && y >= 10.0 && y <= height - 10.0,
            "({x}, {y}) in {width} by {height}"
        );
    }
}

/// The region of each city of [`cities`], in its order.
const REGIONS: [&str; 12] = [
    "Europe", "Americas", "Americas", "Americas", "Africa", "Africa", "Asia", "Asia", "Asia",
    "Oceania", "Americas", "Oceania",
];

/// The cities of [`cities`] in `regions`.
fn cities_in(regions: &[&str]) -> RecordBatch {
    let rows: arrow::array::UInt32Array = REGIONS
        .iter()
        .enumerate()
        .filter(|(_, region)| regions.contains(region))
        .map(|(row, _)| row as u32)
        .collect();
    arrow::compute::take_record_batch(&cities(), &rows).unwrap()
}

/// The equal views and sizes of the map plots of an evaluation.
fn assert_one_view(evaluated: &EvaluatedChart, plots: &[usize]) {
    let first = &evaluated.plots[plots[0]];
    for plot in &plots[1..] {
        let plot = &evaluated.plots[*plot];
        assert!(
            (plot.rect[2] - first.rect[2]).abs() < 1e-3
                && (plot.rect[3] - first.rect[3]).abs() < 1e-3,
            "{:?} and {:?}",
            first.rect,
            plot.rect
        );
        assert_eq!(view(plot), view(first), "{} and {}", plot.name, first.name);
    }
}

fn view(plot: &avenger_chart_eval::EvaluatedPlot) -> avenger_geo::View {
    match &plot.coord {
        PlotCoord::Geo { projection } => projection.view.unwrap(),
        _ => panic!("{} is a map plot", plot.name),
    }
}

/// A map of the cities of `regions` over the continents, which fits its data, with a graticule
/// and the sphere.
fn region_map(regions: &[&str]) -> avenger_chart::OverlayBuilder {
    let geo = proto::Geo {
        surface: Some(spherical("equal_earth", &[])),
        fit: data_fit(),
        graticule: Some(proto::Graticule {
            longitude_step: Some(30.0),
            latitude_step: Some(30.0),
            label_sides: Some(proto::LabelSides { sides: Vec::new() }),
            ..Default::default()
        }),
        sphere: Some(proto::SphereStyle::default()),
        ..Default::default()
    };
    let land = Chart::unit(continents(), land())
        .geometry("geometry")
        .fill(Encode::constant("#c9c3b5"))
        .coord(geo.clone())
        .build()
        .unwrap();
    let places = Chart::unit(cities_in(regions), proto::SymbolMark::default())
        .longitude("lon")
        .latitude("lat")
        .size("population")
        .color(Encode::constant("#e45756"))
        .coord(geo)
        .build()
        .unwrap();
    Chart::overlay([land, places])
}

#[test]
fn a_facet_wrap_of_maps_shares_one_projection() {
    let panels = ["Americas", "Europe", "Africa", "Asia", "Oceania"].map(|region| {
        region_map(&[region])
            .partition("region", region)
            .build()
            .unwrap()
    });
    let chart = Chart::wrap(panels).facet("region").build().unwrap();
    // The wrap reflows from two columns to three.
    let sizes = [[520.0, 420.0], [700.0, 420.0]];
    render_sizes(&chart, &sizes, "map-facet-wrap");
    for (size, columns) in sizes.into_iter().zip([2, 3]) {
        let evaluated = evaluate(&chart, size);
        assert_eq!(evaluated.plots.len(), 5);
        let mut lefts: Vec<f32> = evaluated.plots.iter().map(|plot| plot.rect[0]).collect();
        lefts.sort_by(f32::total_cmp);
        lefts.dedup_by(|a, b| (*a - *b).abs() < 0.01);
        assert_eq!(lefts.len(), columns, "{size:?}");
        // A facet shares the projection, so every panel takes one size and one view, and one
        // size legend stands for every panel.
        assert_one_view(&evaluated, &[0, 1, 2, 3, 4]);
        assert!(evaluated.warnings.is_empty(), "{:?}", evaluated.warnings);
    }
}

/// A map of the cities of `regions` that fits its data, with a graticule.
fn city_region_map(regions: &[&str]) -> Chart {
    Chart::unit(cities_in(regions), proto::SymbolMark::default())
        .longitude("lon")
        .latitude("lat")
        .coord(proto::Geo {
            surface: Some(spherical("equal_earth", &[])),
            fit: data_fit(),
            graticule: Some(proto::Graticule {
                longitude_step: Some(30.0),
                latitude_step: Some(30.0),
                ..Default::default()
            }),
            ..Default::default()
        })
        .build()
        .unwrap()
}

#[test]
fn a_row_of_maps_shares_its_projection_or_keeps_one_for_each() {
    let row = || {
        Chart::row([
            city_region_map(&["Americas"]),
            city_region_map(&["Asia", "Oceania"]),
        ])
    };
    let chart = Chart::column([
        row()
            .projection(proto::Sharing::Shared)
            .title("One projection")
            .build()
            .unwrap(),
        row().title("A projection for each map").build().unwrap(),
    ])
    .build()
    .unwrap();
    render_both(&chart, "map-row-projections");
    let evaluated = evaluate(&chart, [640.0, 480.0]);
    // The shared projection fits the cities of both maps, and the maps of the second row each
    // fit their own.
    assert_one_view(&evaluated, &[0, 1]);
    assert_ne!(view(&evaluated.plots[2]), view(&evaluated.plots[3]));
    assert_ne!(view(&evaluated.plots[0]), view(&evaluated.plots[2]));
}
