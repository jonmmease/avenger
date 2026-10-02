use avenger_geometry::{
    rtree::SceneGraphRTree, GeometryInstance, GeometryQueryHitPolicy as Policy,
    GeometryQueryShape as Shape,
};
use avenger_scenegraph::{marks::mark::MarkInstance, scene_graph::SceneGraph};
use geo_types::{Geometry, LineString, Polygon};

fn tree(geometry: Geometry<f32>, half_stroke_width: f32) -> SceneGraphRTree {
    let mut tree = SceneGraphRTree::from_scene_graph(&SceneGraph {
        width: 100.0,
        height: 100.0,
        origin: [0.0, 0.0],
        marks: vec![],
    });
    tree.insert(GeometryInstance {
        mark_instance: MarkInstance {
            name: "probe".into(),
            mark_path: vec![0],
            instance_index: Some(0),
        },
        interactive: true,
        anchor: None,
        geometry,
        half_stroke_width,
    });
    tree
}

#[test]
fn centroid_query_uses_triangle_centroid() {
    let triangle = Polygon::new(
        LineString::from(vec![(0.0, 0.0), (9.0, 0.0), (0.0, 9.0), (0.0, 0.0)]),
        vec![],
    );
    let tree = tree(triangle.into(), 0.0);
    let hits = tree.query_shape(
        &Shape::Circle {
            cx: 3.0,
            cy: 3.0,
            radius: 0.1,
        },
        Policy::CentroidInside,
    );
    assert_eq!(hits.len(), 1, "triangle centroid is (3, 3)");
}

#[test]
fn circle_query_contains_diamond() {
    let diamond = Polygon::new(
        LineString::from(vec![
            (0.0, -9.0),
            (9.0, 0.0),
            (0.0, 9.0),
            (-9.0, 0.0),
            (0.0, -9.0),
        ]),
        vec![],
    );
    let tree = tree(diamond.into(), 0.0);
    let hits = tree.query_shape(
        &Shape::Circle {
            cx: 0.0,
            cy: 0.0,
            radius: 10.0,
        },
        Policy::GeometryContained,
    );
    assert_eq!(hits.len(), 1, "every point of diamond is within radius 9");
}

#[test]
fn rectangle_query_intersects_visible_stroke() {
    let tree = tree(LineString::from(vec![(0.0, 0.0), (20.0, 0.0)]).into(), 5.0);
    let rect = tree.query_shape(
        &Shape::Rect {
            x0: 8.0,
            y0: 3.0,
            x1: 12.0,
            y1: 4.0,
        },
        Policy::GeometryIntersects,
    );
    let circle = tree.query_shape(
        &Shape::Circle {
            cx: 10.0,
            cy: 3.5,
            radius: 0.1,
        },
        Policy::GeometryIntersects,
    );
    let polygon = tree.query_shape(
        &Shape::Polygon {
            points: vec![[8.0, 3.0], [12.0, 3.0], [12.0, 4.0], [8.0, 4.0]],
        },
        Policy::GeometryIntersects,
    );
    assert_eq!(polygon.len(), 1);
    assert_eq!(circle.len(), 1);
    assert_eq!(
        rect.len(),
        1,
        "rectangle overlaps the same visible stroke as the smaller circle"
    );
}

#[test]
fn anchor_query_finds_an_arc_center_outside_its_bounds() {
    use avenger_geometry::marks::MarkGeometryUtils;
    use avenger_scenegraph::marks::{arc::SceneArcMark, group::SceneGroup};
    let arc = SceneArcMark {
        x: 10.0.into(),
        y: 20.0.into(),
        inner_radius: 20.0.into(),
        outer_radius: 30.0.into(),
        start_angle: 0.0.into(),
        end_angle: 0.5.into(),
        ..Default::default()
    };
    assert_eq!(
        arc.geometry_iter(vec![0], [5.0, 7.0])
            .next()
            .unwrap()
            .anchor,
        Some([15.0, 27.0])
    );
    let scene = SceneGraph {
        width: 100.0,
        height: 100.0,
        origin: [5.0, 7.0],
        marks: vec![SceneGroup {
            marks: vec![arc.into()],
            ..Default::default()
        }
        .into()],
    };
    let tree = SceneGraphRTree::from_scene_graph(&scene);
    let shape = Shape::Circle {
        cx: 15.0,
        cy: 27.0,
        radius: 1.0,
    };
    assert_eq!(tree.query_shape(&shape, Policy::AnchorInside).len(), 1);
    assert!(tree
        .query_shape(&shape, Policy::GeometryIntersects)
        .is_empty());
}

#[test]
fn all_query_shapes_include_stroke_in_containment() {
    let tree = tree(Geometry::Point(geo_types::Point::new(5.0, 5.0)), 2.0);
    for half_extent in [1.0, 2.0, 3.0] {
        let lo = 5.0 - half_extent;
        let hi = 5.0 + half_extent;
        for shape in [
            Shape::Rect {
                x0: lo,
                y0: lo,
                x1: hi,
                y1: hi,
            },
            Shape::Circle {
                cx: 5.0,
                cy: 5.0,
                radius: half_extent,
            },
            Shape::Polygon {
                points: vec![[lo, lo], [hi, lo], [hi, hi], [lo, hi]],
            },
        ] {
            assert_eq!(
                !tree
                    .query_shape(&shape, Policy::GeometryContained)
                    .is_empty(),
                half_extent >= 2.0,
                "{shape:?}"
            );
        }
    }
}

#[test]
fn fill_rules_control_path_and_transformed_symbol_queries() {
    use avenger_common::{
        lyon::parse_svg_path,
        types::{FillRule, SymbolShape},
    };
    use avenger_scenegraph::marks::{path::ScenePathMark, symbol::SceneSymbolMark};
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        for symbol in [false, true] {
            let path = parse_svg_path("M0,0 H40 V40 H0 Z M10,10 H30 V30 H10 Z").unwrap();
            let mark = if symbol {
                SceneSymbolMark {
                    shapes: vec![SymbolShape::Path(path)],
                    fill_rule: rule,
                    size: 4.0.into(),
                    x: 100.0.into(),
                    y: 20.0.into(),
                    angle: 90.0.into(),
                    stroke_width: None,
                    ..Default::default()
                }
                .into()
            } else {
                ScenePathMark {
                    path: path.into(),
                    fill_rule: rule,
                    stroke_width: None,
                    ..Default::default()
                }
                .into()
            };
            let scene = SceneGraph {
                width: 120.0,
                height: 120.0,
                origin: [0.0; 2],
                marks: vec![mark],
            };
            let tree = SceneGraphRTree::from_scene_graph(&scene);
            let p = if symbol { [60.0, 60.0] } else { [20.0, 20.0] };
            let filled = rule == FillRule::NonZero;
            assert_eq!(tree.locate_at_point(&p).is_some(), filled);
            assert_eq!(
                !tree
                    .query_shape(
                        &Shape::Circle {
                            cx: p[0],
                            cy: p[1],
                            radius: 1.0
                        },
                        Policy::GeometryIntersects
                    )
                    .is_empty(),
                filled
            );
            let outer = if symbol { [90.0, 30.0] } else { [5.0, 5.0] };
            assert!(tree.locate_at_point(&outer).is_some());
        }
    }
}

#[test]
fn dashed_line_singletons_remain_queryable_at_breaks_and_at_the_end() {
    use avenger_common::{types::StrokeCap, value::ScalarOrArray as S};
    use avenger_scenegraph::marks::line::SceneLineMark;
    let tree = SceneGraphRTree::from_scene_graph(&SceneGraph {
        width: 100.,
        height: 100.,
        origin: [0.; 2],
        marks: vec![SceneLineMark {
            len: 3,
            x: S::new_array(vec![20., 50., 80.]),
            y: 50.0.into(),
            defined: S::new_array(vec![true, false, true]),
            stroke_width: 10.,
            stroke_cap: StrokeCap::Round,
            stroke_dash: Some(vec![2., 3.]),
            ..Default::default()
        }
        .into()],
    });
    for x in [20., 80.] {
        assert_eq!(
            tree.query_shape(
                &Shape::Circle {
                    cx: x,
                    cy: 50.,
                    radius: 1.
                },
                Policy::GeometryIntersects
            )
            .len(),
            1
        );
    }
}
