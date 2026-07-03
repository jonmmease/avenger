//! Behavior tests for the Geo coordinate system (scratch/geo phase 2).

use avenger_chart::plot::Plot;
use avenger_chart_core::{
    CoordinateDomainCellKey, CoordinateDomainCellRequest, CoordinateDomainGroupRequest,
    CoordinateDomainMaterialization, CoordinateDomainNode, CoordinateDomainProvider,
    CoordinateDomainRole, CoordinateDomainScaleState, CoordinateDomainSharingPolicy,
    CoordinateMeasureRequest, CoordinateMeasurementProvider, CoordinateSystemTransformCore,
    DomainExtent,
};
use avenger_chart_geo::{Geo, GeoCoordMeasurement, GraticuleStyle, SphereStyle};
use avenger_scales::scales::linear::LinearScale;
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
    let plot = Plot::with_coord(
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

#[tokio::test]
#[ignore]
async fn debug_guide_geometry() {
    use avenger_geo::sinks::LyonPathSink;
    use avenger_geo::streamable::Sphere;

    let ctx = SessionContext::new();
    let plot = Plot::with_coord(
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
