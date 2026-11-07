//! Geo plots, with the `geo` feature: world maps, globes, a conic map of a box, routes, a planar
//! geo plot, tiles, geo plots that share a projection, the aspect of geo plots, the inset of marks
//! at the edge of the data, and the coordinates of geo plots.

use std::{io::Cursor, sync::Arc};

use arrow::{
    array::{ArrayRef, AsArray, Float64Array, RecordBatch, StringArray},
    compute::{concat_batches, filter_record_batch},
    datatypes::{Field, Schema},
    ipc::reader::StreamReader,
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

/// The countries of Natural Earth at 1:110m, with their names and continents, from the world data
/// of avenger-guides-geo, which Git LFS stores, without the islands of the open ocean, whose long
/// continent name would crowd a small plot's legend.
fn countries() -> RecordBatch {
    let bytes: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../avenger-guides-geo/examples/data/countries.arrows"
    ));
    let reader = StreamReader::try_new(Cursor::new(bytes), None)
        .expect("the countries, which `git lfs pull` fetches");
    let schema = reader.schema();
    let batches = reader.collect::<Result<Vec<_>, _>>().unwrap();
    let countries = concat_batches(&schema, &batches).unwrap();
    let continents = countries
        .column_by_name("continent")
        .unwrap()
        .as_string::<i32>();
    let land = continents
        .iter()
        .map(|continent| Some(continent != Some("Seven seas (open ocean)")))
        .collect();
    filter_record_batch(&countries, &land).unwrap()
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
        countries(),
        GeoShapeMark::new("geometry").color("continent"),
    );
    let places = GeoLayer::symbol(
        cities(),
        GeoSymbolMark::builder()
            .longitude("lon")
            .latitude("lat")
            .size("population"),
    );
    let options = Geo::spherical(
        Spherical::equal_earth()
            .fit_sphere()
            .graticule(
                Graticule::builder()
                    .longitude_step(30.0)
                    .latitude_step(30.0),
            )
            .sphere(SphereStyle::builder()),
    );
    let chart = Chart::builder(Plot::geo(GeoPlot::new(options, [land, places])))
        .build()
        .unwrap();
    render_both(&chart, "geo-world");

    // A geo plot of the whole sphere fits inside its plot, and keeps every city.
    let evaluated = evaluate(&chart, [640.0, 360.0]);
    assert_eq!(*clip(&evaluated, 0), Clip::None);
    let cities = symbols(&evaluated, &format!("{}/1", evaluated.plots[0].name));
    assert_eq!(cities.len, 12);
}

#[test]
fn a_globe_draws_the_places_of_its_near_side() {
    let land = GeoLayer::shape(countries(), GeoShapeMark::new("geometry"));
    let places = GeoLayer::symbol(
        cities(),
        GeoSymbolMark::builder()
            .longitude("lon")
            .latitude("lat")
            .color(Encoding::direct_value("#e45756")),
    );
    let options = Geo::spherical(
        Spherical::orthographic()
            .rotate(Rotation::builder().lambda(-20.0).phi(-30.0))
            .fit_sphere()
            .graticule(Graticule::builder())
            .sphere(SphereStyle::builder()),
    );
    let chart = Chart::builder(Plot::geo(GeoPlot::new(options, [land, places])))
        .build()
        .unwrap();
    render_both(&chart, "geo-globe");

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
        countries(),
        GeoShapeMark::new("geometry").fill(Encoding::direct_value("#c9d9a5")),
    );
    let places = GeoLayer::text(
        cities(),
        GeoTextMark::new("name").longitude("lon").latitude("lat"),
    );
    let options = Geo::spherical(
        Spherical::conic_conformal(ConicProjection::builder().parallel1(29.5).parallel2(45.5))
            .rotate(Rotation::builder().lambda(96.0))
            .fit_box(
                LonLatBox::builder()
                    .west(-125.0)
                    .south(24.0)
                    .east(-66.0)
                    .north(50.0),
            )
            .graticule(Graticule::builder()),
    );
    let chart = Chart::builder(Plot::geo(GeoPlot::new(options, [land, places])))
        .build()
        .unwrap();
    render_both(&chart, "geo-conic-box");

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
            flights(),
            GeoLineMark::builder()
                .route(route)
                .longitude("lon")
                .latitude("lat")
                .detail("route")
                .stroke(Encoding::direct_value(color)),
        )
    };
    // A rule along the parallel of 40 degrees north, from 120 west to 20 east.
    let parallel = GeoLayer::rule(
        flights().slice(0, 1),
        GeoRuleMark::new(-120.0, 40.0)
            .longitude2(20.0)
            .stroke_dash(Encoding::direct_value("4 2")),
    );
    let land = GeoLayer::shape(
        countries(),
        GeoShapeMark::new("geometry").fill(Encoding::direct_value("#dddddd")),
    );
    let options = Geo::spherical(
        Spherical::equirectangular().fit_sphere().graticule(
            Graticule::builder()
                .longitude_step(30.0)
                .latitude_step(30.0),
        ),
    );
    let layers = [
        land,
        line(Route::Coordinate, "#4c78a8"),
        line(Route::Canvas, "#e45756"),
        parallel,
    ];
    let chart = Chart::builder(Plot::geo(GeoPlot::new(options, layers)))
        .build()
        .unwrap();
    render_both(&chart, "geo-routes");
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
    let land = GeoLayer::shape(parcels, GeoShapeMark::new("geometry").color("name"));
    let options = Geo::planar(Planar::builder().reflect_y(true));
    let chart = Chart::builder(Plot::geo(GeoPlot::new(options, [land])))
        .build()
        .unwrap();
    render_both(&chart, "geo-planar");

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
        cities(),
        GeoSymbolMark::builder().longitude("lon").latitude("lat"),
    );
    let options = Geo::spherical(
        Spherical::mercator()
            .fit_box(
                LonLatBox::builder()
                    .west(-130.0)
                    .south(15.0)
                    .east(-60.0)
                    .north(60.0),
            )
            .tiles(TileLayer::new("tiles/{z}/{x}/{y}.png")),
    );
    let chart = Chart::builder(Plot::geo(GeoPlot::new(options, [places])))
        .build()
        .unwrap();
    let mut evaluator = evaluator(&chart);
    // Without a resolver, the plot draws without its tiles, with a warning.
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
        let name = format!("geo-tiles-{}x{}", size[0], size[1]);
        assert_baseline(env!("CARGO_MANIFEST_DIR"), &svg_to_png(&svg, 2.0), &name);
    }
}

#[test]
fn maps_of_one_family_share_one_projection() {
    let plot = |table: RecordBatch| {
        let places = GeoLayer::symbol(
            table,
            GeoSymbolMark::builder().longitude("lon").latitude("lat"),
        );
        let options = Geo::spherical(
            Spherical::equal_earth()
                .fit_data()
                .sphere(SphereStyle::builder()),
        )
        .family("world");
        Plot::geo(GeoPlot::new(options, [places]))
    };
    let all = cities();
    let chart = Chart::builder(
        Concat::new(Direction::Row).children([plot(all.slice(0, 6)), plot(all.slice(6, 6))]),
    )
    .build()
    .unwrap();
    render(&chart, [640.0, 280.0], "geo-shared");

    // A place draws at one point of each plot, because the plots share one view of the data of
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
        let land = GeoLayer::shape(countries(), GeoShapeMark::new("geometry"));
        Chart::builder(Plot::geo(GeoPlot::new(geo, [land])))
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
        cities(),
        GeoSymbolMark::builder()
            .longitude("lon")
            .latitude("lat")
            .size(400.0),
    );
    let options = Geo::spherical(Spherical::mercator());
    let chart = Chart::builder(Plot::geo(GeoPlot::new(options, [places])))
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
        cities(),
        GeoSymbolMark::builder().longitude("lon").latitude("lat"),
    );
    let options = Geo::spherical(Spherical::robinson().fit_sphere());
    let chart = Chart::builder(Plot::geo(GeoPlot::new(options, [places])))
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

/// A geo plot that keeps its aspect beside legends taller than it draws at its aspect, and its legends
/// run past its short side instead of widening as it shrinks.
#[test]
fn a_map_beside_legends_taller_than_it_keeps_its_aspect() {
    let land = GeoLayer::shape(
        countries(),
        GeoShapeMark::new("geometry").color("continent"),
    );
    let places = GeoLayer::symbol(
        cities(),
        GeoSymbolMark::builder()
            .longitude("lon")
            .latitude("lat")
            .size("population"),
    );
    let options = Geo::spherical(Spherical::equal_earth().fit_sphere());
    let chart = Chart::builder(Plot::geo(GeoPlot::new(options, [land, places])))
        .build()
        .unwrap();
    for size in [[320.0, 240.0], [400.0, 240.0]] {
        let evaluated = evaluate(&chart, size);
        assert!(evaluated.warnings.is_empty(), "{:?}", evaluated.warnings);
        let [_, _, width, height] = evaluated.plots[0].rect;
        assert!(
            width > 200.0 && width / height > 1.9,
            "{size:?}: {width} {height}"
        );
    }
}

/// A world map of cities in the family `world`, which fits its data.
fn city_plot(table: RecordBatch) -> Chart {
    let places = GeoLayer::symbol(
        table,
        GeoSymbolMark::builder().longitude("lon").latitude("lat"),
    );
    let options = Geo::spherical(Spherical::equal_earth().sphere(SphereStyle::builder()))
        .family("world")
        .navigation(GeoNavigation::builder().min_zoom(1.0).max_zoom(8.0));
    Chart::builder(Plot::geo(GeoPlot::new(options, [places])))
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
fn a_projection_view_shows_on_its_plot_and_scales_with_it() {
    use avenger_chart_eval::{ProjectionNavigation, ProjectionView};
    let chart = city_plot(cities());
    let mut evaluator = evaluator(&chart);
    let fitted = evaluator.evaluate(&host([480.0, 300.0])).unwrap();
    assert!(evaluator.projection_view(0).is_none());
    let Some(ProjectionNavigation::Geo {
        min_scale,
        max_scale,
        ..
    }) = evaluator.projection_navigation(0)
    else {
        panic!("a geo plot has a navigation");
    };
    assert!(max_scale.unwrap() > min_scale.unwrap() * 7.9);

    // A pan of the view by 40 pixels to the right.
    let Some(ProjectionView::Geo(projection)) = evaluator.projection_view_mut(0) else {
        panic!("a geo plot has a view");
    };
    projection.view.as_mut().unwrap().translate[0] += 40.0;
    let panned = evaluator.evaluate(&host([480.0, 300.0])).unwrap();
    assert!((origin(&panned).0 - origin(&fitted).0 - 40.0).abs() < 1e-2);
    assert!(matches!(clip(&panned, 0), Clip::Rect { .. }));
    render_view(&mut evaluator, "geo-view");

    // Without the view, the plot fits again.
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
    let mut evaluator = evaluator(&city_plot(all.slice(0, 6)));
    let fitted = evaluator.evaluate(&host([480.0, 300.0])).unwrap();
    let update = |evaluator: &mut avenger_chart_eval::ChartEvaluator, options: &UpdateOptions| {
        let prepared = PreparedChart::new(city_plot(all.clone()), &engine(), &Registry::new());
        evaluator.install(prepared.unwrap(), options).unwrap();
        evaluator.evaluate(&host([480.0, 300.0])).unwrap()
    };

    // A kept fit shows the old data's extent, and a refit the new data's.
    let mut kept = evaluator;
    let kept_plot = update(&mut kept, &UpdateOptions::new().fit(Fit::Keep));
    assert_eq!(origin(&kept_plot), origin(&fitted));
    let mut refit = crate::support::evaluator(&city_plot(all.slice(0, 6)));
    refit.evaluate(&host([480.0, 300.0])).unwrap();
    let refit_plot = update(&mut refit, &UpdateOptions::new());
    assert_ne!(origin(&refit_plot), origin(&fitted));

    // A view carries to the projection of the same family.
    let Some(ProjectionView::Geo(projection)) = refit.projection_view_mut(0) else {
        panic!("a geo plot has a view");
    };
    projection.view.as_mut().unwrap().translate[1] += 25.0;
    let panned = refit.evaluate(&host([480.0, 300.0])).unwrap();
    let carried = update(&mut refit, &UpdateOptions::new());
    assert!((origin(&carried).1 - origin(&panned).1).abs() < 1e-2);
}
