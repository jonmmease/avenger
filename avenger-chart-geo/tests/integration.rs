//! Behavior tests for the Geo coordinate system (scratch/geo phase 2).

use avenger_chart::plot::Chart;
use avenger_chart_core::{
    CoordinateDomainCellKey, CoordinateDomainCellRequest, CoordinateDomainGroupRequest,
    CoordinateDomainMaterialization, CoordinateDomainNode, CoordinateDomainProvider,
    CoordinateDomainRole, CoordinateDomainScaleState, CoordinateDomainSharingPolicy,
    CoordinateMeasureRequest, CoordinateMeasurementProvider, CoordinateSystemTransformCore,
    DomainExtent, Param,
};
use avenger_chart_geo::{Geo, GeoCoordMeasurement, GraticuleStyle, SphereStyle};
use avenger_scales::scales::linear::LinearScale;
use datafusion::arrow::datatypes::DataType;
use datafusion::common::ScalarValue;
use datafusion::prelude::SessionContext;
use indexmap::IndexMap;
use std::collections::HashMap;

fn assert_close(actual: f64, expected: f64, epsilon: f64) {
    assert!(
        (actual - expected).abs() <= epsilon,
        "expected {expected}, got {actual}"
    );
}

fn nullable_float_param(name: String) -> Param {
    Param::typed(name, DataType::Float64, ScalarValue::Float64(None))
        .expect("nullable Float64 default must match its declared type")
}

fn scene_mark_names(marks: &[avenger_scenegraph::marks::mark::SceneMark]) -> Vec<String> {
    use avenger_scenegraph::marks::mark::SceneMark;
    let mut names = Vec::new();
    for mark in marks {
        match mark {
            SceneMark::Group(group) => {
                names.push(format!("group:{}", group.name));
                names.extend(scene_mark_names(&group.marks));
            }
            SceneMark::Path(path) => names.push(format!("path:{}", path.name)),
            SceneMark::Line(line) => names.push(format!("line:{}", line.name)),
            SceneMark::Symbol(symbol) => names.push(format!("symbol:{}", symbol.name)),
            other => names.push(format!("other:{other:?}").chars().take(40).collect()),
        }
    }
    names
}

#[tokio::test]
async fn empty_geo_plot_renders_guide_marks() {
    let ctx = SessionContext::new();
    let plot = Chart::with_coord(
        Geo::equal_earth()
            .sphere(SphereStyle::default())
            .graticule(GraticuleStyle::default()),
    )
    .plot_size(520.0, 320.0);
    let compiled = plot.compile(&ctx).await.expect("compile");
    let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate");
    let names = scene_mark_names(&evaluated.scene_graph.marks);
    assert!(
        names.iter().any(|n| n == "path:geo-sphere"),
        "expected geo-sphere mark, got {names:?}"
    );
    assert!(
        names.iter().any(|n| n == "path:geo-graticule"),
        "expected geo-graticule mark, got {names:?}"
    );
}

fn state(channel: &str, extent: Option<DomainExtent>) -> CoordinateDomainScaleState {
    CoordinateDomainScaleState {
        scale_name: channel.to_string(),
        coord_channel: channel.to_string(),
        role: if channel == "x" {
            CoordinateDomainRole::X
        } else {
            CoordinateDomainRole::Y
        },
        base_domain: extent,
        range: Some((0.0, 100.0)),
        node: CoordinateDomainNode::Local {
            cell_key: CoordinateDomainCellKey::new("root"),
            scale_name: channel.to_string(),
        },
        has_explicit_domain: false,
        raw_domain_param: None,
    }
}

#[test]
fn descriptor_owns_and_materializes_x_y() {
    let descriptors = Geo::new().domain_descriptors();
    assert_eq!(descriptors.len(), 1);
    let descriptor = &descriptors[0];
    assert_eq!(descriptor.id, "geo_viewport");
    assert!(descriptor.depends_on_plot_area);
    assert_eq!(
        descriptor.sharing_policy,
        CoordinateDomainSharingPolicy::FacetRepeatGroups
    );
    assert_eq!(descriptor.bindings.len(), 2);
    assert!(descriptor.bindings.iter().all(|binding| matches!(
        binding.materialize,
        CoordinateDomainMaterialization::CreateIfAbsent { .. }
    )));
}

#[test]
fn resolves_domains_from_projected_data() {
    let coord = Geo::equirectangular();
    let cell_key = CoordinateDomainCellKey::new("root");
    let states = [
        state("x", Some(DomainExtent::numeric(-1.0, 1.0))),
        state("y", Some(DomainExtent::numeric(-0.5, 0.5))),
    ];
    let params = IndexMap::new();
    let cell = CoordinateDomainCellRequest {
        cell_key: &cell_key,
        plot_area_width: 200.0,
        plot_area_height: 100.0,
        params: &params,
        scale_states: &states,
    };
    let cells = [cell];
    let resolution = coord
        .resolve_domain_group(CoordinateDomainGroupRequest {
            descriptor_id: "geo_viewport",
            cells: &cells,
        })
        .expect("resolve");
    assert_eq!(resolution.cells.len(), 1);
    // 2 units over 200px and 1 unit over 100px both fit at 0.01 upp.
    assert_eq!(
        resolution.cells[0].domain_overrides["x"],
        DomainExtent::numeric(-1.0, 1.0)
    );
    assert_eq!(
        resolution.cells[0].domain_overrides["y"],
        DomainExtent::numeric(-0.5, 0.5)
    );
}

#[test]
fn shared_group_unions_bounds_across_cells() {
    let coord = Geo::equirectangular();
    let key_a = CoordinateDomainCellKey::new("a");
    let key_b = CoordinateDomainCellKey::new("b");
    // Same nodes -> same group (shared viewport).
    let states_a = [
        state("x", Some(DomainExtent::numeric(0.1, 0.2))),
        state("y", Some(DomainExtent::numeric(0.0, 0.05))),
    ];
    let states_b = [
        state("x", Some(DomainExtent::numeric(0.4, 0.5))),
        state("y", Some(DomainExtent::numeric(0.0, 0.05))),
    ];
    let params = IndexMap::new();
    let cells = [
        CoordinateDomainCellRequest {
            cell_key: &key_a,
            plot_area_width: 100.0,
            plot_area_height: 100.0,
            params: &params,
            scale_states: &states_a,
        },
        CoordinateDomainCellRequest {
            cell_key: &key_b,
            plot_area_width: 100.0,
            plot_area_height: 100.0,
            params: &params,
            scale_states: &states_b,
        },
    ];
    let resolution = coord
        .resolve_domain_group(CoordinateDomainGroupRequest {
            descriptor_id: "geo_viewport",
            cells: &cells,
        })
        .expect("resolve");
    // Both cells fit the union [0.1, 0.5] x [0, 0.05]: center 0.3, 0.4
    // units over 100 px -> 0.004 upp -> half width 0.2.
    for cell in resolution.cells {
        let (min, max) = cell.domain_overrides["x"]
            .numeric_bounds()
            .expect("numeric");
        assert_close(min, 0.1, 1e-9);
        assert_close(max, 0.5, 1e-9);
    }
}

#[tokio::test]
async fn measurement_carries_projection_and_view() {
    let coord = Geo::albers_usa_conus()
        .viewport_id("map")
        .graticule(GraticuleStyle::default());
    let ctx = SessionContext::new();
    let params = IndexMap::new();
    let mut scales = HashMap::new();
    scales.insert(
        "x".to_string(),
        LinearScale::configured((-0.3, 0.3), (0.0, 300.0)),
    );
    scales.insert(
        "y".to_string(),
        LinearScale::configured((-0.2, 0.2), (200.0, 0.0)),
    );
    let request = CoordinateMeasureRequest {
        plot_width: 300.0,
        plot_height: 200.0,
        params: &params,
        session_context: &ctx,
        data: None,
        compiled_marks: &[],
        facet_path: &[],
        scales,
    };
    let measurement = coord
        .measure_coordinate(request)
        .await
        .expect("measure")
        .expect("measurement");
    let measurement = GeoCoordMeasurement::downcast(measurement.as_ref()).expect("geo measurement");
    assert_eq!(measurement.viewport_id, "map");
    assert!(measurement.graticule.is_some());
    assert!(measurement.sphere.is_none());
    assert_close(measurement.view.center_x, 0.0, 1e-9);
    assert_close(measurement.view.center_y, 0.0, 1e-9);
    // 0.6 units over 300 px -> 0.002 upp (f32 scale domains limit precision).
    assert_close(measurement.view.units_per_pixel, 0.002, 1e-9);
}

#[tokio::test]
async fn runtime_params_override_authored_view() {
    let coord = Geo::equal_earth().viewport_id("main");
    assert_eq!(
        coord.runtime_param_dependencies(),
        vec![
            "__geo_main_center_x",
            "__geo_main_center_y",
            "__geo_main_units_per_pixel",
            "__geo_main_focus_x",
            "__geo_main_focus_y",
        ]
    );
    let ctx = SessionContext::new();
    let mut params = IndexMap::new();
    params.insert(coord.center_x_param(), ScalarValue::Float64(Some(1.5)));
    params.insert(coord.center_y_param(), ScalarValue::Float64(Some(-0.5)));
    params.insert(
        coord.units_per_pixel_param(),
        ScalarValue::Float64(Some(0.01)),
    );
    let request = CoordinateMeasureRequest {
        plot_width: 100.0,
        plot_height: 100.0,
        params: &params,
        session_context: &ctx,
        data: None,
        compiled_marks: &[],
        facet_path: &[],
        scales: HashMap::new(),
    };
    let measurement = coord
        .measure_coordinate(request)
        .await
        .expect("measure")
        .expect("measurement");
    let measurement = GeoCoordMeasurement::downcast(measurement.as_ref()).expect("geo measurement");
    assert_close(measurement.view.center_x, 1.5, 1e-12);
    assert_close(measurement.view.center_y, -0.5, 1e-12);
    assert_close(measurement.view.units_per_pixel, 0.01, 1e-12);
    assert_close(measurement.view.x_domain.0, 1.0, 1e-9);
    assert_close(measurement.view.x_domain.1, 2.0, 1e-9);
}

#[test]
fn center_lon_lat_projects_through_rotation() {
    let coord = Geo::albers_usa_conus().center_lon_lat(-96.0, 38.5);
    // The CONUS aspect rotates [96, 0, 0], so lon -96 maps to rotated
    // lon 0: the raw x must be ~0.
    let projection = coord.projection();
    let (x, y) = projection.project_raw_units(-96.0, 38.5);
    assert!(x.abs() < 1e-9, "x = {x}");
    assert!(y.is_finite());
    // And unproject round-trips.
    let (lon, lat) = coord.unproject(x, y).expect("unproject");
    assert_close(lon, -96.0, 1e-9);
    assert_close(lat, 38.5, 1e-9);
}

#[test]
fn coordinate_serializes_round_trip() {
    let coord = Geo::conic_conformal((35.0, 65.0))
        .viewport_id("map")
        .rotate([-15.0, 0.0, 0.0])
        .center_lon_lat(15.0, 52.0)
        .zoom(2.5)
        .graticule(GraticuleStyle::default())
        .sphere(SphereStyle::default());
    let json = serde_json::to_string(&coord).expect("serialize");
    let restored: Geo = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(
        restored.projection_kind(),
        &avenger_chart_geo::ProjectionKind::ConicConformal {
            parallels: (35.0, 65.0)
        }
    );

    let bytes = bincode::serialize(&coord).expect("bincode serialize");
    let _restored: Geo = bincode::deserialize(&bytes).expect("bincode deserialize");
}

/// Geo coordinate-param previews retarget cached data marks — the
/// Cartesian raw-domain behavior. Center/units-per-pixel changes move
/// positions only through the coordinate-owned x/y linear scales
/// (`runtime_params_retarget_cached_marks`), so a preview evaluation
/// reuses the measured profile and applies affine scale adjustments to
/// the cached marks instead of rebuilding mark data.
#[tokio::test]
async fn geo_param_preview_retargets_cached_data_marks() {
    use avenger_chart::prelude::{EvaluationRequest, Symbol};
    use avenger_chart_geo::GeoPositionChannels;
    use datafusion::prelude::col;
    use std::sync::Arc;

    let geo = Geo::mercator().viewport_id("main");
    let ctx = Arc::new(SessionContext::new());
    let df = ctx
        .sql("SELECT * FROM (VALUES (1.4, 0.4), (1.5, 0.5), (1.6, 0.6)) AS t(x, y)")
        .await
        .expect("raw-unit point data");
    let plot = avenger_chart::plot::Chart::with_coord(geo.clone())
        .plot_size(200.0, 200.0)
        .param(nullable_float_param(geo.center_x_param()))
        .param(nullable_float_param(geo.center_y_param()))
        .param(nullable_float_param(geo.units_per_pixel_param()))
        .data(df)
        .mark(
            Symbol::new()
                .projected_x(col("x"))
                .projected_y(col("y"))
                .size(25.0),
        );
    let compiled = Arc::new(plot.compile(&ctx).await.expect("compile"));
    let mut session = compiled.instantiate(ctx);

    let mut params = IndexMap::new();
    params.insert(geo.center_x_param(), ScalarValue::Float64(Some(1.5)));
    params.insert(geo.center_y_param(), ScalarValue::Float64(Some(0.5)));
    params.insert(
        geo.units_per_pixel_param(),
        ScalarValue::Float64(Some(0.01)),
    );
    let (_evaluated, exact) = session
        .evaluate_with_metrics(EvaluationRequest::new().exact().param_patch(params))
        .await
        .expect("exact evaluation");
    assert!(exact.facet_layout.plot_component_measure_calls > 0);

    // Zoom in 20% around the same center: a pure coordinate-param change.
    let mut patch = IndexMap::new();
    patch.insert(
        geo.units_per_pixel_param(),
        ScalarValue::Float64(Some(0.008)),
    );
    let (evaluated, preview) = session
        .evaluate_with_metrics(EvaluationRequest::new().preview().param_patch(patch))
        .await
        .expect("preview evaluation");

    assert_eq!(preview.pipeline.preview_profile_reuses, 1);
    assert_eq!(
        preview.pipeline.preview_data_mark_reuses, 1,
        "geo coordinate-param preview should retarget cached data marks"
    );
    assert_eq!(preview.pipeline.preview_data_mark_reuse_misses, 0);
    assert_eq!(
        preview.pipeline.mark_data_collects, 0,
        "geo coordinate-param preview should not recollect mark data"
    );

    // The retargeted symbols carry affine scale adjustments for the renderer.
    fn symbol_has_adjustment(marks: &[avenger_scenegraph::marks::mark::SceneMark]) -> bool {
        use avenger_scenegraph::marks::mark::SceneMark;
        marks.iter().any(|mark| match mark {
            SceneMark::Group(group) => symbol_has_adjustment(&group.marks),
            SceneMark::Symbol(symbol) => {
                symbol.x_adjustment.is_some() || symbol.y_adjustment.is_some()
            }
            _ => false,
        })
    }
    assert!(
        symbol_has_adjustment(&evaluated.scene_graph.marks),
        "retargeted symbols should carry scale adjustments"
    );
}

/// View params (`v.x().domain_start()` etc.) resolve on `Plot<Geo>` marks: the
/// x/y scales the Geo coordinate system installs are linear raw-projected-unit
/// scales, so the view-param resolver reads their domains exactly as it does
/// on Cartesian.
///
/// The runtime view is pinned via params (center_x 1.5, 0.01 units/px, 100 px
/// plot -> x domain [1.0, 2.0]); of three symbols at raw x 0.5 / 1.5 / 2.5,
/// a `[domain_start, domain_end]` filter must keep exactly the middle one.
#[tokio::test]
async fn view_params_resolve_on_geo_marks() {
    use avenger_chart::prelude::{Filter, Symbol, View};
    use avenger_chart_geo::GeoPositionChannels;
    use datafusion::prelude::col;

    let geo = Geo::mercator().viewport_id("main");
    let ctx = SessionContext::new();
    let df = ctx
        .sql("SELECT * FROM (VALUES (0.5, 0.0), (1.5, 0.0), (2.5, 0.0)) AS t(x, y)")
        .await
        .expect("raw-unit point data");

    let plot = Chart::with_coord(geo.clone())
        .plot_size(100.0, 100.0)
        .param(nullable_float_param(geo.center_x_param()))
        .param(nullable_float_param(geo.center_y_param()))
        .param(nullable_float_param(geo.units_per_pixel_param()))
        .data(df)
        .mark(
            Symbol::new()
                .view(
                    View::cartesian()
                        .id("v")
                        .x_domain(col("x"))
                        .y_domain(col("y")),
                    |mark, v| {
                        mark.transform(
                            Filter::new(
                                col("x")
                                    .gt_eq(v.x().domain_start())
                                    .and(col("x").lt_eq(v.x().domain_end())),
                            ),
                            |mark, _| mark,
                        )
                    },
                )
                .projected_x(col("x"))
                .projected_y(col("y"))
                .size(25.0),
        );
    let compiled = plot.compile(&ctx).await.expect("compile");

    let mut params = IndexMap::new();
    params.insert(geo.center_x_param(), ScalarValue::Float64(Some(1.5)));
    params.insert(geo.center_y_param(), ScalarValue::Float64(Some(0.0)));
    params.insert(
        geo.units_per_pixel_param(),
        ScalarValue::Float64(Some(0.01)),
    );
    let evaluated = compiled
        .evaluate(&ctx, Some(params.clone()))
        .await
        .expect("evaluate with view params");

    fn count_symbols(marks: &[avenger_scenegraph::marks::mark::SceneMark]) -> usize {
        use avenger_scenegraph::marks::mark::SceneMark;
        marks
            .iter()
            .map(|mark| match mark {
                SceneMark::Group(group) => count_symbols(&group.marks),
                SceneMark::Symbol(symbol) => symbol.len as usize,
                _ => 0,
            })
            .sum()
    }
    assert_eq!(
        count_symbols(&evaluated.scene_graph.marks),
        1,
        "the [domain_start, domain_end] filter must keep exactly the x=1.5 point"
    );

    // The domain the params resolved against is the realized GeoView's.
    let request = CoordinateMeasureRequest {
        plot_width: 100.0,
        plot_height: 100.0,
        params: &params,
        session_context: &ctx,
        data: None,
        compiled_marks: &[],
        facet_path: &[],
        scales: HashMap::new(),
    };
    let measurement = geo
        .measure_coordinate(request)
        .await
        .expect("measure")
        .expect("measurement");
    let measurement = GeoCoordMeasurement::downcast(measurement.as_ref()).expect("geo measurement");
    assert_close(measurement.view.x_domain.0, 1.0, 1e-9);
    assert_close(measurement.view.x_domain.1, 2.0, 1e-9);
}

#[tokio::test]
#[ignore]
async fn debug_guide_geometry() {
    use avenger_geo::sinks::LyonPathSink;
    use avenger_geo::streamable::Sphere;

    let ctx = SessionContext::new();
    let plot = Chart::with_coord(
        Geo::equal_earth()
            .sphere(SphereStyle::default())
            .graticule(GraticuleStyle::default()),
    )
    .plot_size(520.0, 320.0);
    let compiled = plot.compile(&ctx).await.expect("compile");
    let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate");

    fn dump(marks: &[avenger_scenegraph::marks::mark::SceneMark], depth: usize) {
        use avenger_scenegraph::marks::mark::SceneMark;
        for mark in marks {
            match mark {
                SceneMark::Group(group) => {
                    println!(
                        "{:indent$}group {} origin {:?} clip {:?}",
                        "",
                        group.name,
                        group.origin,
                        group.clip,
                        indent = depth * 2
                    );
                    dump(&group.marks, depth + 1);
                }
                SceneMark::Path(path) => {
                    let p = path.path_vec();
                    let first = p.first().map(|path| {
                        let pts: Vec<_> = path
                            .iter()
                            .take(3)
                            .map(|ev| format!("{ev:?}").chars().take(60).collect::<String>())
                            .collect();
                        pts
                    });
                    println!(
                        "{:indent$}path {} n_paths {} first_events {:?}",
                        "",
                        path.name,
                        p.len(),
                        first,
                        indent = depth * 2
                    );
                }
                other => println!(
                    "{:indent$}other {:?}",
                    "",
                    &format!("{other:?}")[..60.min(format!("{other:?}").len())],
                    indent = depth * 2
                ),
            }
        }
    }
    dump(&evaluated.scene_graph.marks, 0);

    // Also project the sphere directly through a hand-built measurement.
    let coord = Geo::equal_earth();
    let params = IndexMap::new();
    let request = CoordinateMeasureRequest {
        plot_width: 520.0,
        plot_height: 320.0,
        params: &params,
        session_context: &ctx,
        data: None,
        compiled_marks: &[],
        facet_path: &[],
        scales: HashMap::new(),
    };
    let measurement = coord.measure_coordinate(request).await.unwrap().unwrap();
    let measurement = GeoCoordMeasurement::downcast(measurement.as_ref()).unwrap();
    println!("view: {:?}", measurement.view);
    let projector = measurement.view_projector();
    println!("proj (0,0) -> {:?}", projector.project(0.0, 0.0));
    println!("proj (180,0) -> {:?}", projector.project(180.0, 0.0));
    let mut sink = LyonPathSink::fill();
    projector.stream(&Sphere, &mut sink);
    println!("sphere has_content: {}", sink.has_content());
    let path = sink.finish();
    let mut n = 0;
    for ev in path.iter() {
        if n < 4 {
            println!("  ev: {ev:?}");
        }
        n += 1;
    }
    println!("sphere path events: {n}");
}

#[test]
#[ignore]
fn debug_compute_zooms() {
    use avenger_chart_geo::Geo;
    // CONUS on the albers aspect
    let geo = Geo::albers_usa_conus();
    let projection = geo.projection();
    let world = {
        // world span via the public path: realize a world view at known plot
        // and back out spans from upp math is convoluted; just project bboxes.
        let corners = [
            (-124.7, 24.5),
            (-124.7, 49.4),
            (-66.9, 24.5),
            (-66.9, 49.4),
            (-95.0, 49.4),
            (-95.0, 24.5),
        ];
        let mut xmin = f64::INFINITY;
        let mut xmax = f64::NEG_INFINITY;
        let mut ymin = f64::INFINITY;
        let mut ymax = f64::NEG_INFINITY;
        for (lon, lat) in corners {
            let (x, y) = projection.project_raw_units(lon, lat);
            xmin = xmin.min(x);
            xmax = xmax.max(x);
            ymin = ymin.min(y);
            ymax = ymax.max(y);
        }
        println!("CONUS raw bbox: x [{xmin:.4}, {xmax:.4}] y [{ymin:.4}, {ymax:.4}]");
        println!(
            "CONUS center: ({:.4}, {:.4})",
            (xmin + xmax) / 2.0,
            (ymin + ymax) / 2.0
        );
        let upp = ((xmax - xmin) / 520.0_f64).max((ymax - ymin) / 360.0);
        println!("CONUS upp for 520x360: {upp:.6}");
        upp
    };
    // zoom = log2(world_max / (256*upp))
    let ws = avenger_chart_geo_world_span(&projection);
    println!("albers world span: {ws:?}");
    println!("CONUS zoom: {:.3}", (ws / (256.0 * world)).log2());

    // Europe on conic conformal
    let geo = Geo::conic_conformal((35.0, 65.0)).rotate([-15.0, 0.0, 0.0]);
    let projection = geo.projection();
    let mut xmin = f64::INFINITY;
    let mut xmax = f64::NEG_INFINITY;
    let mut ymin = f64::INFINITY;
    let mut ymax = f64::NEG_INFINITY;
    for (lon, lat) in [
        (-11.0, 35.0),
        (-11.0, 62.0),
        (35.0, 35.0),
        (35.0, 62.0),
        (15.0, 71.0),
        (15.0, 35.0),
    ] {
        let (x, y) = projection.project_raw_units(lon, lat);
        xmin = xmin.min(x);
        xmax = xmax.max(x);
        ymin = ymin.min(y);
        ymax = ymax.max(y);
    }
    println!("Europe raw bbox: x [{xmin:.4}, {xmax:.4}] y [{ymin:.4}, {ymax:.4}]");
    println!(
        "Europe center: ({:.4}, {:.4})",
        (xmin + xmax) / 2.0,
        (ymin + ymax) / 2.0
    );
    let upp = ((xmax - xmin) / 460.0_f64).max((ymax - ymin) / 400.0);
    let ws = avenger_chart_geo_world_span(&projection);
    println!("cc world span: {ws:?}");
    println!("Europe zoom: {:.3}", (ws / (256.0 * upp)).log2());
}

fn avenger_chart_geo_world_span(projection: &avenger_geo::projector::Projection) -> f64 {
    let world = avenger_chart_geo::view::world_span(projection);
    world.width.max(world.height)
}

#[tokio::test]
#[ignore]
async fn debug_route_plot_scene() {
    use avenger_chart_geo::{GeoPositionChannels, GraticuleStyle, SphereStyle};
    use avenger_chart_marks::{Line, Symbol};
    use datafusion::prelude::col;

    let ctx = SessionContext::new();
    let df = ctx
        .sql(
            "SELECT * FROM (VALUES
                ('JFK-LHR', 0, -73.78, 40.64), ('JFK-LHR', 1, -0.45, 51.47),
                ('JFK-NRT', 0, -73.78, 40.64), ('JFK-NRT', 1, 140.39, 35.76)
            ) AS t(route, seq, lon, lat)",
        )
        .await
        .expect("route data");
    let geo = Geo::equal_earth()
        .sphere(SphereStyle::default())
        .graticule(GraticuleStyle::default());
    let plot = Chart::with_coord(geo.clone())
        .plot_size(520.0, 320.0)
        .data(df)
        .mark(
            Line::new()
                .lon_lat(&geo, "lon", "lat")
                .details(["route"])
                .order(col("seq"))
                .stroke("#1d4ed8")
                .stroke_width(1.6),
        )
        .mark(
            Symbol::new()
                .lon_lat(&geo, "lon", "lat")
                .size(24.0)
                .fill("#111827"),
        );
    let compiled = plot.compile(&ctx).await.expect("compile");
    let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate");

    fn dump(marks: &[avenger_scenegraph::marks::mark::SceneMark], depth: usize) {
        use avenger_scenegraph::marks::mark::SceneMark;
        for mark in marks {
            match mark {
                SceneMark::Group(group) => {
                    println!(
                        "{:indent$}group '{}' origin {:?}",
                        "",
                        group.name,
                        group.origin,
                        indent = depth * 2
                    );
                    dump(&group.marks, depth + 1);
                }
                SceneMark::Line(line) => {
                    let xs = line.x.as_vec(line.len as usize, None);
                    let ys = line.y.as_vec(line.len as usize, None);
                    println!(
                        "{:indent$}line '{}' len {} first {:?} defined {:?}",
                        "",
                        line.name,
                        line.len,
                        xs.iter().zip(ys.iter()).take(3).collect::<Vec<_>>(),
                        line.defined
                            .as_vec(line.len as usize, None)
                            .iter()
                            .take(6)
                            .collect::<Vec<_>>(),
                        indent = depth * 2
                    );
                }
                SceneMark::Symbol(sym) => {
                    let xs = sym.x.as_vec(sym.len as usize, None);
                    let ys = sym.y.as_vec(sym.len as usize, None);
                    println!(
                        "{:indent$}symbol '{}' len {} xy {:?}",
                        "",
                        sym.name,
                        sym.len,
                        xs.iter().zip(ys.iter()).take(4).collect::<Vec<_>>(),
                        indent = depth * 2
                    );
                }
                other => {
                    let s = format!("{other:?}");
                    println!(
                        "{:indent$}other {}",
                        "",
                        &s[..60.min(s.len())],
                        indent = depth * 2
                    );
                }
            }
        }
    }
    dump(&evaluated.scene_graph.marks, 0);
}

#[tokio::test]
#[ignore]
async fn debug_us_states_shapes() {
    use avenger_geo::ingest::{geojson_to_features, stream_wkb_through};
    use avenger_geo::projector::{BoundsSink, Projection};
    use avenger_geo::raw::ProjectionKind;

    let path = format!(
        "{}/../avenger-chart/tests/data/geo/us-states.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let json = std::fs::read_to_string(path).unwrap();
    let features = geojson_to_features(&json).unwrap();

    // View-like projector: albers CONUS at world-ish scale.
    let projection = Projection::new(ProjectionKind::albers()).with_rotate([96.0, 0.0, 0.0]);
    let projector = projection.build_view((0.0031, 0.6410), 0.002, 560.0, 380.0);

    for feature in &features {
        let name = feature
            .properties
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let Some(wkb) = &feature.wkb else { continue };
        let mut bounds = BoundsSink::default();
        stream_wkb_through(&projector, wkb, &mut bounds).unwrap();
        if let Some([[x0, y0], [x1, y1]]) = bounds.result() {
            let w = x1 - x0;
            let h = y1 - y0;
            if w > 500.0 || h > 350.0 {
                println!("FLOOD {name}: {w:.0} x {h:.0}");
            }
        }
    }
    println!("done");
}

mod blend {
    use super::*;
    use avenger_chart_geo::BlendConfig;

    async fn measurement_for(geo: Geo, plot: (f32, f32)) -> avenger_chart_geo::GeoCoordMeasurement {
        let ctx = SessionContext::new();
        let params = IndexMap::new();
        let request = CoordinateMeasureRequest {
            plot_width: plot.0,
            plot_height: plot.1,
            params: &params,
            session_context: &ctx,
            data: None,
            compiled_marks: &[],
            facet_path: &[],
            scales: HashMap::new(),
        };
        let measurement = coord_measure(&geo, request).await;
        GeoCoordMeasurement::downcast(measurement.as_ref())
            .expect("geo measurement")
            .clone()
    }

    async fn coord_measure(
        geo: &Geo,
        request: CoordinateMeasureRequest<'_>,
    ) -> Box<dyn avenger_chart_core::CoordMeasurement> {
        geo.measure_coordinate(request)
            .await
            .expect("measure")
            .expect("measurement")
    }

    #[tokio::test]
    async fn blend_t_zero_matches_unblended_projector() {
        let geo = Geo::albers_usa_conus()
            .center_lon_lat(-98.0, 38.5)
            .zoom(3.0)
            .adaptive_blend(BlendConfig {
                force_t: Some(0.0),
                ..Default::default()
            });
        let with_blend = measurement_for(geo, (500.0, 400.0)).await;
        let geo_plain = Geo::albers_usa_conus()
            .center_lon_lat(-98.0, 38.5)
            .zoom(3.0);
        let without = measurement_for(geo_plain, (500.0, 400.0)).await;

        let a = with_blend.view_projector();
        let b = without.view_projector();
        for &(lon, lat) in &[(-98.0, 38.5), (-120.0, 45.0), (-80.0, 28.0)] {
            let pa = a.project(lon, lat).expect("project");
            let pb = b.project(lon, lat).expect("project");
            assert_close(pa.0, pb.0, 1e-9);
            assert_close(pa.1, pb.1, 1e-9);
        }
    }

    #[tokio::test]
    async fn anchoring_keeps_center_fixed_across_t() {
        let anchor = (-98.0, 38.5);
        let mut reference: Option<(f64, f64)> = None;
        for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let geo = Geo::albers_usa_conus()
                .center_lon_lat(anchor.0, anchor.1)
                .zoom(5.0)
                .adaptive_blend(BlendConfig {
                    force_t: Some(t),
                    ..Default::default()
                });
            let measurement = measurement_for(geo, (500.0, 400.0)).await;
            let projector = measurement.view_projector();
            let p = projector
                .project(anchor.0, anchor.1)
                .expect("project anchor");
            // The anchor must stay at the plot center for every t.
            assert_close(p.0, 250.0, 0.5);
            assert_close(p.1, 200.0, 0.5);
            // And a nearby landmark must move continuously (small deltas).
            let landmark = projector.project(-97.0, 39.0).expect("project landmark");
            if let Some(prev) = reference {
                let drift = ((landmark.0 - prev.0).powi(2) + (landmark.1 - prev.1).powi(2)).sqrt();
                assert!(drift < 25.0, "t={t}: landmark jumped {drift}px");
            }
            reference = Some(landmark);
        }
    }

    #[tokio::test]
    async fn polar_view_clamps_blend_to_authored() {
        // A world view spans beyond ±85°: t must clamp to 0 even past z1.
        let geo = Geo::equal_earth().adaptive_blend(BlendConfig {
            z0: -10.0,
            z1: -5.0,
            force_t: None,
        });
        let measurement = measurement_for(geo, (500.0, 300.0)).await;
        assert!(measurement.view.zoom > -5.0);
        assert_eq!(measurement.blend_t(), 0.0);
    }
}
