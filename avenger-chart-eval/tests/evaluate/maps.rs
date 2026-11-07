//! Map plots, with the `geo` feature: world maps, globes, a conic map of a box, routes, a planar
//! map, tiles, maps that share a projection, the aspect of maps, the inset of marks at the edge of
//! the data, and the coordinates of maps.

use std::sync::Arc;

use arrow::{
    array::{ArrayRef, Float64Array, RecordBatch, StringArray},
    datatypes::{Field, Schema},
};
use avenger_chart::prelude::*;
use avenger_chart_eval::EvaluatedChart;
use avenger_image::{ImageLease, ImageResolver, ImageState, RgbaImage};
use avenger_render_fixtures::raster::{assert_baseline, svg_to_png};
use avenger_scenegraph::marks::{group::Clip, mark::SceneMark, symbol::SceneSymbolMark};
use avenger_svg::SvgRenderer;
use geo_types::{Geometry, LineString, Polygon};
use geoarrow_array::{builder::WkbBuilder, GeoArrowArray};
use geoarrow_schema::WkbType;

use crate::support::*;

/// A polygon through the places of `ring`, which closes it.
fn polygon(ring: &[(f64, f64)]) -> Polygon<f64> {
    let mut ring = ring.to_vec();
    ring.push(ring[0]);
    Polygon::new(LineString::from(ring), Vec::new())
}

/// A table of `shapes` with their names, as a WKB GeoArrow `geometry` column beside a `name`.
fn shapes_table(shapes: &[(&str, Polygon<f64>)]) -> RecordBatch {
    let mut builder = WkbBuilder::<i32>::new(WkbType::default());
    for (_, shape) in shapes {
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

/// Rough outlines of five continents.
fn continents() -> RecordBatch {
    shapes_table(&[
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
    ])
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

/// The symbol mark named `name` in the scene.
fn symbols<'a>(evaluated: &'a EvaluatedChart, name: &str) -> &'a SceneSymbolMark {
    fn find<'a>(marks: &'a [SceneMark], name: &str) -> Option<&'a SceneSymbolMark> {
        marks.iter().find_map(|mark| match mark {
            SceneMark::Symbol(symbol) if symbol.name == name => Some(symbol),
            SceneMark::Group(group) => find(&group.marks, name),
            _ => None,
        })
    }
    find(&evaluated.scene.marks, name).expect("the symbol mark")
}

/// The clip of the group of the plot `plot` of an evaluation.
fn clip(evaluated: &EvaluatedChart, plot: usize) -> &Clip {
    let name = &evaluated.plots[plot].name;
    (evaluated.scene.marks.iter())
        .find_map(|mark| match mark {
            SceneMark::Group(group) if &group.name == name => Some(&group.clip),
            _ => None,
        })
        .expect("the plot's group")
}

#[test]
fn an_equal_earth_world_map_draws_its_sphere_graticule_shapes_and_cities() {
    let land = GeoLayer::shape(
        GeoShapeMark::builder()
            .geometry(Encoding::column("geometry"))
            .color(Encoding::scaled_column("name")),
    )
    .table(continents());
    let places = GeoLayer::symbol(
        GeoSymbolMark::builder()
            .longitude(Encoding::column("lon"))
            .latitude(Encoding::column("lat"))
            .size(Encoding::scaled_column("population")),
    )
    .table(cities());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::geo(
            GeoPlot::builder()
                .options(Geo::spherical(
                    Spherical::equal_earth()
                        .fit_sphere()
                        .graticule(
                            Graticule::builder()
                                .longitude_step(30.0)
                                .latitude_step(30.0),
                        )
                        .sphere(SphereStyle::builder()),
                ))
                .layers([land, places]),
        ))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&chart, "map-world");

    // A map of the whole sphere fits inside its plot, and keeps every city.
    let evaluated = evaluate(&chart, [640.0, 360.0]);
    assert_eq!(*clip(&evaluated, 0), Clip::None);
    let cities = symbols(&evaluated, &format!("{}/1", evaluated.plots[0].name));
    assert_eq!(cities.len, 12);
}

#[test]
fn a_globe_draws_the_places_of_its_near_side() {
    let land = GeoLayer::shape(GeoShapeMark::builder().geometry(Encoding::column("geometry")))
        .table(continents());
    let places = GeoLayer::symbol(
        GeoSymbolMark::builder()
            .longitude(Encoding::column("lon"))
            .latitude(Encoding::column("lat"))
            .color(Encoding::constant("#e45756")),
    )
    .table(cities());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::geo(
            GeoPlot::builder()
                .options(Geo::spherical(
                    Spherical::orthographic()
                        .rotate(Rotation::builder().lambda(-20.0).phi(-30.0))
                        .fit_sphere()
                        .graticule(Graticule::builder())
                        .sphere(SphereStyle::builder()),
                ))
                .layers([land, places]),
        ))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&chart, "map-globe");

    // The far side of the globe hides the cities of the Americas and the Pacific, and the data
    // of the evaluation holds the rows that draw.
    let evaluated = evaluate(&chart, [320.0, 240.0]);
    let name = format!("{}/1", evaluated.plots[0].name);
    let drawn = symbols(&evaluated, &name).len as usize;
    assert!(drawn > 0 && drawn < 12, "{drawn}");
    assert_eq!(evaluated.data.rows(&name).unwrap().num_rows(), drawn);
}

#[test]
fn a_conic_map_of_a_box_clips_to_its_plot() {
    let land = GeoLayer::shape(
        GeoShapeMark::builder()
            .geometry(Encoding::column("geometry"))
            .fill(Encoding::constant("#c9d9a5")),
    )
    .table(continents());
    let places = GeoLayer::text(
        GeoTextMark::builder()
            .longitude(Encoding::column("lon"))
            .latitude(Encoding::column("lat"))
            .text(Encoding::column("name")),
    )
    .table(cities());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::geo(
            GeoPlot::builder()
                .options(Geo::spherical(
                    Spherical::builder()
                        .conic_conformal(ConicProjection::builder().parallel1(29.5).parallel2(45.5))
                        .rotate(Rotation::builder().lambda(96.0))
                        .fit_box(
                            LonLatBox::builder()
                                .west(-125.0)
                                .south(24.0)
                                .east(-66.0)
                                .north(50.0),
                        )
                        .graticule(Graticule::builder()),
                ))
                .layers([land, places]),
        ))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&chart, "map-conic-box");

    let evaluated = evaluate(&chart, [640.0, 360.0]);
    assert!(matches!(clip(&evaluated, 0), Clip::Rect { .. }));
}

#[test]
fn routes_follow_the_great_circle_the_canvas_and_the_parallel() {
    let flights = || {
        RecordBatch::try_from_iter([
            (
                "route",
                Arc::new(StringArray::from(vec!["a", "a", "b", "b"])) as ArrayRef,
            ),
            ("lon", numbers(&[-0.13, 139.69, -74.01, -118.24])),
            ("lat", numbers(&[51.51, 35.69, 40.71, 34.05])),
        ])
        .unwrap()
    };
    let line = |route: Route, color: &str| {
        GeoLayer::line(
            GeoLineMark::builder()
                .route(route)
                .longitude(Encoding::column("lon"))
                .latitude(Encoding::column("lat"))
                .detail(Encoding::column("route"))
                .stroke(Encoding::constant(color)),
        )
        .table(flights())
    };
    // A rule along the parallel of 40 degrees north, from 120 west to 20 east.
    let parallel = GeoLayer::rule(
        GeoRuleMark::builder()
            .longitude(Encoding::constant(-120.0))
            .longitude2(Encoding::constant(20.0))
            .latitude(Encoding::constant(40.0))
            .stroke_dash(Encoding::constant("4 2")),
    )
    .table(flights().slice(0, 1));
    let land = GeoLayer::shape(
        GeoShapeMark::builder()
            .geometry(Encoding::column("geometry"))
            .fill(Encoding::constant("#dddddd")),
    )
    .table(continents());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::geo(
            GeoPlot::builder()
                .options(Geo::spherical(
                    Spherical::equirectangular().fit_sphere().graticule(
                        Graticule::builder()
                            .longitude_step(30.0)
                            .latitude_step(30.0),
                    ),
                ))
                .layers([
                    land,
                    line(Route::Coordinate, "#4c78a8"),
                    line(Route::Canvas, "#e45756"),
                    parallel,
                ]),
        ))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&chart, "map-routes");
}

#[test]
fn a_planar_map_draws_projected_coordinates() {
    let square = |x: f64, y: f64, side: f64| {
        polygon(&[(x, y), (x + side, y), (x + side, y + side), (x, y + side)])
    };
    let parcels = shapes_table(&[
        ("north", square(0.0, 60.0, 40.0)),
        ("east", square(60.0, 20.0, 30.0)),
        ("south", square(10.0, 0.0, 50.0)),
    ]);
    let land = GeoLayer::shape(
        GeoShapeMark::builder()
            .geometry(Encoding::column("geometry"))
            .color(Encoding::scaled_column("name")),
    )
    .table(parcels);
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::geo(
            GeoPlot::builder()
                .options(Geo::planar(Planar::builder().reflect_y(true)))
                .layers([land]),
        ))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&chart, "map-planar");

    // The y axis flips, so the northern parcel stands at the top.
    let evaluated = evaluate(&chart, [320.0, 240.0]);
    let [x, y] = evaluated.plots[0]
        .coords
        .pixels([&numbers(&[20.0, 20.0]), &numbers(&[80.0, 10.0])])
        .unwrap();
    assert!((x.value(0) - x.value(1)).abs() < 1e-3);
    assert!(y.value(0) < y.value(1));
}

/// Tiles of a checkerboard by their column and row, which answer at once.
struct Checkerboard;

impl ImageResolver for Checkerboard {
    fn image(&self, url: &str) -> ImageState {
        ImageState::Ready(Arc::new(self.tile(url)))
    }

    fn prefetch(&self, _url: &str) {}

    fn ready(&self, url: &str) -> Option<Arc<RgbaImage>> {
        Some(Arc::new(self.tile(url)))
    }

    fn retain(&self, _urls: &[&str]) -> ImageLease {
        ImageLease::default()
    }
}

impl Checkerboard {
    /// A tile of 8 by 8 pixels, light or dark by the parity of its column and row in `url`,
    /// `tiles/{z}/{x}/{y}.png`.
    fn tile(&self, url: &str) -> RgbaImage {
        let parts: Vec<u32> = (url.trim_end_matches(".png").rsplit('/').take(2))
            .filter_map(|part| part.parse().ok())
            .collect();
        let dark = parts.iter().sum::<u32>() % 2 == 1;
        let color = match dark {
            true => [158, 188, 218, 255],
            false => [222, 235, 247, 255],
        };
        RgbaImage {
            width: 8,
            height: 8,
            data: color.repeat(64),
        }
    }
}

#[test]
fn a_map_draws_its_tiles_through_the_image_resolver() {
    let places = GeoLayer::symbol(
        GeoSymbolMark::builder()
            .longitude(Encoding::column("lon"))
            .latitude(Encoding::column("lat")),
    )
    .table(cities());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::geo(
            GeoPlot::builder()
                .options(Geo::spherical(
                    Spherical::mercator()
                        .fit_box(
                            LonLatBox::builder()
                                .west(-130.0)
                                .south(15.0)
                                .east(-60.0)
                                .north(60.0),
                        )
                        .tiles(TileLayer::builder().url_template("tiles/{z}/{x}/{y}.png")),
                ))
                .layers([places]),
        ))
        .infer_scales()
        .build()
        .unwrap();
    let mut evaluator = evaluator(&chart);
    // Without a resolver, the map draws without its tiles, with a warning.
    let bare = evaluator.evaluate(&host([320.0, 240.0])).unwrap();
    assert!(warns(&bare, "image resolver"));

    let resolver: Arc<dyn ImageResolver> = Arc::new(Checkerboard);
    evaluator.set_image_resolver(resolver.clone());
    for size in [[320.0, 240.0], [640.0, 360.0]] {
        let evaluated = evaluator.evaluate(&host(size)).unwrap();
        assert!(!warns(&evaluated, "image resolver"));
        let svg = SvgRenderer::new()
            .with_images(resolver.clone())
            .render_scene_graph(&evaluated.scene, &engine())
            .unwrap();
        let name = format!("map-tiles-{}x{}", size[0], size[1]);
        assert_baseline(env!("CARGO_MANIFEST_DIR"), &svg_to_png(&svg, 2.0), &name);
    }
}

#[test]
fn maps_of_one_family_share_one_projection() {
    let map = |table: RecordBatch| {
        let places = GeoLayer::symbol(
            GeoSymbolMark::builder()
                .longitude(Encoding::column("lon"))
                .latitude(Encoding::column("lat")),
        )
        .table(table);
        Plot::geo(
            GeoPlot::builder()
                .options(
                    Geo::spherical(
                        Spherical::equal_earth()
                            .fit_data()
                            .sphere(SphereStyle::builder()),
                    )
                    .family("world"),
                )
                .layers([places]),
        )
    };
    let all = cities();
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(
            Concat::builder()
                .direction(Direction::Row)
                .children([map(all.slice(0, 6)), map(all.slice(6, 6))]),
        )
        .infer_scales()
        .build()
        .unwrap();
    render(&chart, [640.0, 280.0], "map-shared");

    // A place draws at one point of each plot, because the maps share one view of the data of
    // both.
    let evaluated = evaluate(&chart, [640.0, 280.0]);
    let place = |plot: usize| {
        let [x, y] = (evaluated.plots[plot].coords)
            .pixels([&numbers(&[0.0]), &numbers(&[0.0])])
            .unwrap();
        (x.value(0), y.value(0))
    };
    let (first, second) = (place(0), place(1));
    assert!((first.0 - second.0).abs() < 1e-3 && (first.1 - second.1).abs() < 1e-3);
    let [_, _, width, height] = evaluated.plots[0].rect;
    assert_eq!([width, height], {
        let [_, _, width, height] = evaluated.plots[1].rect;
        [width, height]
    });
}

#[test]
fn a_map_keeps_its_aspect_and_a_free_map_fills_its_space() {
    let chart = |geo: GeoBuilder| {
        let land = GeoLayer::shape(GeoShapeMark::builder().geometry(Encoding::column("geometry")))
            .table(continents());
        Chart::builder()
            .number_format_syntax("d3")
            .datetime_format_syntax("d3")
            .root(Plot::geo(GeoPlot::builder().options(geo).layers([land])))
            .infer_scales()
            .build()
            .unwrap()
    };
    let world = || Spherical::equal_earth().fit_sphere();
    let kept = evaluate(&chart(Geo::spherical(world())), [600.0, 600.0]);
    let [_, _, width, height] = kept.plots[0].rect;
    assert!(width / height > 1.5, "{width} {height}");
    let free = evaluate(
        &chart(Geo::spherical(world()).free_aspect()),
        [600.0, 600.0],
    );
    let [_, _, width, height] = free.plots[0].rect;
    assert!((width - height).abs() < 1.0, "{width} {height}");
}

#[test]
fn symbols_at_the_edge_of_the_data_stay_inside_the_plot() {
    let places = GeoLayer::symbol(
        GeoSymbolMark::builder()
            .longitude(Encoding::column("lon"))
            .latitude(Encoding::column("lat"))
            .size(Encoding::constant(400.0)),
    )
    .table(cities());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::geo(
            GeoPlot::builder()
                .options(Geo::spherical(Spherical::mercator()))
                .layers([places]),
        ))
        .infer_scales()
        .build()
        .unwrap();
    let evaluated = evaluate(&chart, [400.0, 300.0]);
    let [_, _, width, height] = evaluated.plots[0].rect;
    let symbols = symbols(&evaluated, &format!("{}/0", evaluated.plots[0].name));
    let radius = 10.0;
    for (x, y) in (symbols.x.as_vec(12, None).into_iter()).zip(symbols.y.as_vec(12, None)) {
        assert!(
            x - radius >= -0.5 && x + radius <= width + 0.5,
            "{x} {width}"
        );
        assert!(
            y - radius >= -0.5 && y + radius <= height + 0.5,
            "{y} {height}"
        );
    }
}

#[test]
fn the_coordinates_of_a_map_place_its_symbols() {
    let places = GeoLayer::symbol(
        GeoSymbolMark::builder()
            .longitude(Encoding::column("lon"))
            .latitude(Encoding::column("lat")),
    )
    .table(cities());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::geo(
            GeoPlot::builder()
                .options(Geo::spherical(Spherical::robinson().fit_sphere()))
                .layers([places]),
        ))
        .infer_scales()
        .build()
        .unwrap();
    let evaluated = evaluate(&chart, [480.0, 300.0]);
    let table = cities();
    let [x, y] = evaluated.plots[0]
        .coords
        .pixels([
            table.column_by_name("lon").unwrap(),
            table.column_by_name("lat").unwrap(),
        ])
        .unwrap();
    let symbols = symbols(&evaluated, &format!("{}/0", evaluated.plots[0].name));
    for (row, (sx, sy)) in (symbols.x.as_vec(12, None).into_iter())
        .zip(symbols.y.as_vec(12, None))
        .enumerate()
    {
        assert!((x.value(row) - sx).abs() < 1e-2, "{row}");
        assert!((y.value(row) - sy).abs() < 1e-2, "{row}");
    }
}

/// A world map of cities in the family `world`, which fits its data.
fn city_map(table: RecordBatch) -> Chart {
    let places = GeoLayer::symbol(
        GeoSymbolMark::builder()
            .longitude(Encoding::column("lon"))
            .latitude(Encoding::column("lat")),
    )
    .table(table);
    Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::geo(
            GeoPlot::builder()
                .options(
                    Geo::spherical(Spherical::equal_earth().sphere(SphereStyle::builder()))
                        .family("world")
                        .navigation(GeoNavigation::builder().min_zoom(1.0).max_zoom(8.0)),
                )
                .layers([places]),
        ))
        .infer_scales()
        .build()
        .unwrap()
}

/// The pixel of the place at longitude 0 and latitude 0 in the first plot of an evaluation.
fn origin(evaluated: &EvaluatedChart) -> (f32, f32) {
    let [x, y] = (evaluated.plots[0].coords)
        .pixels([&numbers(&[0.0]), &numbers(&[0.0])])
        .unwrap();
    (x.value(0), y.value(0))
}

#[test]
fn a_projection_view_shows_on_its_map_and_scales_with_it() {
    use avenger_chart_eval::{ProjectionNavigation, ProjectionView};
    let chart = city_map(cities());
    let mut evaluator = evaluator(&chart);
    let fitted = evaluator.evaluate(&host([480.0, 300.0])).unwrap();
    assert!(evaluator.projection_view(0).is_none());
    let Some(ProjectionNavigation::Geo {
        min_scale,
        max_scale,
        ..
    }) = evaluator.projection_navigation(0)
    else {
        panic!("a map plot has a navigation");
    };
    assert!(max_scale.unwrap() > min_scale.unwrap() * 7.9);

    // A pan of the view by 40 pixels to the right.
    let Some(ProjectionView::Geo(projection)) = evaluator.projection_view_mut(0) else {
        panic!("a map plot has a view");
    };
    projection.view.as_mut().unwrap().translate[0] += 40.0;
    let panned = evaluator.evaluate(&host([480.0, 300.0])).unwrap();
    assert!((origin(&panned).0 - origin(&fitted).0 - 40.0).abs() < 1e-2);
    assert!(matches!(clip(&panned, 0), Clip::Rect { .. }));
    render_view(&mut evaluator, "map-view");

    // Without the view, the map fits again.
    evaluator.clear_projection_view(0);
    let cleared = evaluator.evaluate(&host([480.0, 300.0])).unwrap();
    assert_eq!(origin(&cleared), origin(&fitted));
}

/// Renders the evaluation of `evaluator` at 480 by 300 and compares it with the baseline `name`.
fn render_view(evaluator: &mut avenger_chart_eval::ChartEvaluator, name: &str) {
    let evaluated = evaluator.evaluate(&host([480.0, 300.0])).unwrap();
    assert_image(&evaluated, name);
}

#[test]
fn an_update_carries_the_view_and_the_fit_of_a_projection() {
    use avenger_chart_eval::{Fit, PreparedChart, ProjectionView, Registry, UpdateOptions};
    let all = cities();
    let mut evaluator = evaluator(&city_map(all.slice(0, 6)));
    let fitted = evaluator.evaluate(&host([480.0, 300.0])).unwrap();
    let update = |evaluator: &mut avenger_chart_eval::ChartEvaluator, options: &UpdateOptions| {
        let prepared = PreparedChart::new(city_map(all.clone()), &engine(), &Registry::new());
        evaluator.install(prepared.unwrap(), options).unwrap();
        evaluator.evaluate(&host([480.0, 300.0])).unwrap()
    };

    // A kept fit shows the old data's extent, and a refit the new data's.
    let mut kept = evaluator;
    let kept_map = update(&mut kept, &UpdateOptions::new().fit(Fit::Keep));
    assert_eq!(origin(&kept_map), origin(&fitted));
    let mut refit = crate::support::evaluator(&city_map(all.slice(0, 6)));
    refit.evaluate(&host([480.0, 300.0])).unwrap();
    let refit_map = update(&mut refit, &UpdateOptions::new());
    assert_ne!(origin(&refit_map), origin(&fitted));

    // A view carries to the projection of the same family.
    let Some(ProjectionView::Geo(projection)) = refit.projection_view_mut(0) else {
        panic!("a map plot has a view");
    };
    projection.view.as_mut().unwrap().translate[1] += 25.0;
    let panned = refit.evaluate(&host([480.0, 300.0])).unwrap();
    let carried = update(&mut refit, &UpdateOptions::new());
    assert!((origin(&carried).1 - origin(&panned).1).abs() < 1e-2);
}
