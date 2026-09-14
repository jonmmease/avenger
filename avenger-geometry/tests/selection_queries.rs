use avenger_common::types::{FillRule, SymbolShape};
use avenger_geometry::{
    rtree::SceneGraphRTree, GeometryInstance, GeometryQueryHitPolicy as Policy,
    GeometryQueryShape as Shape,
};
use avenger_scenegraph::{
    marks::{
        group::{Clip, SceneGroup},
        mark::{MarkInstance, SceneMark},
        rect::SceneRectMark,
        symbol::SceneSymbolMark,
    },
    scene_graph::SceneGraph,
};
use avenger_typst_label::bundled_label_engine;
use geo_types::{Geometry, LineString, Polygon};

fn tree(geometry: Geometry<f32>, reach: f32) -> SceneGraphRTree {
    let mut tree = SceneGraphRTree::from_scene_graph(&scene(vec![]), &bundled_label_engine());
    tree.insert(GeometryInstance {
        mark_instance: MarkInstance {
            name: "probe".into(),
            mark_path: vec![0],
            instance_index: Some(0),
        },
        interactive: true,
        geometry,
        reach,
    });
    tree
}

fn scene(marks: Vec<SceneMark>) -> SceneGraph {
    SceneGraph {
        width: 100.0,
        height: 100.0,
        origin: [0.0, 0.0],
        marks,
    }
}

fn scene_tree(marks: Vec<SceneMark>) -> SceneGraphRTree {
    SceneGraphRTree::from_scene_graph(&scene(marks), &bundled_label_engine())
}

/// The instance indices that match.
fn matches(tree: &SceneGraphRTree, shape: &Shape, policy: Policy) -> Vec<Option<usize>> {
    tree.query_shape(shape, policy)
        .iter()
        .map(|instance| instance.mark_instance.instance_index)
        .collect()
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
            fill_rule: FillRule::NonZero,
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

/// A pie slice matches where it is drawn: its centroid lies in the slice, and the pie's
/// center, outside the slice, matches nothing.
#[test]
fn slice_is_selected_by_its_centroid() {
    use avenger_scenegraph::marks::arc::SceneArcMark;
    let slice = SceneArcMark {
        x: 50.0.into(),
        y: 50.0.into(),
        inner_radius: 20.0.into(),
        outer_radius: 30.0.into(),
        start_angle: 0.0.into(),
        end_angle: 0.5.into(),
        ..Default::default()
    };
    let tree = scene_tree(vec![slice.into()]);
    let middle = 25.0;
    let angle = 0.25f32;
    // Arcs start at 12 o'clock and run clockwise.
    let inside = Shape::Circle {
        cx: 50.0 + middle * angle.sin(),
        cy: 50.0 - middle * angle.cos(),
        radius: 2.0,
    };
    let center = Shape::Circle {
        cx: 50.0,
        cy: 50.0,
        radius: 2.0,
    };
    assert_eq!(matches(&tree, &inside, Policy::CentroidInside).len(), 1);
    assert!(matches(&tree, &center, Policy::CentroidInside).is_empty());
    assert!(matches(&tree, &center, Policy::GeometryIntersects).is_empty());
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
                fill_rule: FillRule::NonZero,
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

/// A circle symbol is its center and radius, so picking and queries meet its drawn edge at any
/// size.
#[test]
fn circle_symbols_are_exact_at_any_size() {
    for size in [16.0f32, 2500.0] {
        let radius = size.sqrt() / 2.0;
        let tree = scene_tree(vec![SceneSymbolMark {
            x: 50.0.into(),
            y: 50.0.into(),
            size: size.into(),
            ..Default::default()
        }
        .into()]);
        for degrees in [0.0f32, 22.5, 45.0, 100.0] {
            let (sin, cos) = degrees.to_radians().sin_cos();
            let at = |distance: f32| [50.0 + distance * cos, 50.0 + distance * sin];
            assert!(tree.pick_top_mark_at_point(&at(radius - 0.05)).is_some());
            assert!(tree.pick_top_mark_at_point(&at(radius + 0.05)).is_none());
            let touching = Shape::Circle {
                cx: at(radius + 1.0)[0],
                cy: at(radius + 1.0)[1],
                radius: 1.05,
            };
            assert_eq!(
                matches(&tree, &touching, Policy::GeometryIntersects).len(),
                1
            );
        }
        let around = |extra: f32| Shape::Circle {
            cx: 50.0,
            cy: 50.0,
            radius: radius + extra,
        };
        assert_eq!(
            matches(&tree, &around(0.05), Policy::GeometryContained).len(),
            1
        );
        assert!(matches(&tree, &around(-0.05), Policy::GeometryContained).is_empty());
    }
}

/// A straight-edged symbol takes its rotation, in degrees, as renderers draw it.
#[test]
fn rotated_symbols_hit_where_they_draw() {
    let square: SymbolShape = "square".try_into().unwrap();
    let tree = scene_tree(vec![SceneSymbolMark {
        x: 50.0.into(),
        y: 50.0.into(),
        size: 100.0.into(),
        angle: 45.0.into(),
        shapes: vec![square],
        ..Default::default()
    }
    .into()]);
    // Turned 45°, the 10 × 10 square reaches 7.07 along each axis but not into its old corners.
    assert!(tree.pick_top_mark_at_point(&[56.8, 50.0]).is_some());
    assert!(tree.pick_top_mark_at_point(&[54.8, 54.8]).is_none());
}

/// A five-pointed star drawn in one stroke crosses itself, and encloses its middle twice: the
/// nonzero rule selects the middle, and the even-odd rule leaves it out.
#[test]
fn fill_rule_decides_what_a_self_crossing_lasso_encloses() {
    let points = |indices: &[usize]| -> Vec<SceneMark> {
        vec![SceneSymbolMark {
            len: indices.len() as u32,
            x: indices
                .iter()
                .map(|&i| [50.0, 50.0, 68.0][i])
                .collect::<Vec<_>>()
                .into(),
            y: indices
                .iter()
                .map(|&i| [50.0, 52.0, 44.0][i])
                .collect::<Vec<_>>()
                .into(),
            size: 4.0.into(),
            ..Default::default()
        }
        .into()]
    };
    let tree = scene_tree(points(&[0, 1, 2]));
    let star: Vec<[f32; 2]> = (0..5)
        .map(|k| {
            let angle = (-90.0 + 144.0 * k as f32).to_radians();
            [50.0 + 30.0 * angle.cos(), 50.0 + 30.0 * angle.sin()]
        })
        .collect();
    let lasso = |fill_rule| Shape::Polygon {
        points: star.clone(),
        fill_rule,
    };
    // Points 0 and 1 are in the middle; point 2 is in a tip.
    assert_eq!(
        matches(&tree, &lasso(FillRule::NonZero), Policy::CentroidInside),
        [Some(0), Some(1), Some(2)]
    );
    assert_eq!(
        matches(&tree, &lasso(FillRule::EvenOdd), Policy::CentroidInside),
        [Some(2)]
    );
}

/// Marks in a clipped group match only where the clip leaves them visible.
#[test]
fn clipped_marks_match_only_where_visible() {
    let points = SceneSymbolMark {
        name: "points".into(),
        len: 2,
        x: vec![40.0, 60.0].into(),
        y: vec![40.0, 60.0].into(),
        size: 16.0.into(),
        ..Default::default()
    };
    let bar = SceneRectMark {
        name: "bar".into(),
        x: 46.0.into(),
        y: 10.0.into(),
        width: Some(10.0.into()),
        height: Some(10.0.into()),
        ..Default::default()
    };
    let tree = scene_tree(vec![SceneGroup {
        clip: Clip::Rect {
            x: 0.0,
            y: 0.0,
            width: 50.0,
            height: 50.0,
        },
        marks: vec![points.into(), bar.into()],
        ..Default::default()
    }
    .into()]);
    let names = |shape: &Shape, policy| {
        tree.query_shape(shape, policy)
            .iter()
            .map(|instance| {
                (
                    instance.mark_instance.name.clone(),
                    instance.mark_instance.instance_index,
                )
            })
            .collect::<Vec<_>>()
    };
    let everything = Shape::Rect {
        x0: 0.0,
        y0: 0.0,
        x1: 100.0,
        y1: 100.0,
    };
    let lasso = Shape::Polygon {
        points: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]],
        fill_rule: FillRule::NonZero,
    };
    let circle = Shape::Circle {
        cx: 50.0,
        cy: 50.0,
        radius: 80.0,
    };
    for shape in [&everything, &lasso, &circle] {
        assert_eq!(
            names(shape, Policy::CentroidInside),
            [("points".to_string(), Some(0))],
            "{shape:?}"
        );
        assert_eq!(
            names(shape, Policy::GeometryIntersects),
            [
                ("points".to_string(), Some(0)),
                ("bar".to_string(), Some(0))
            ],
            "{shape:?}"
        );
        assert_eq!(
            names(shape, Policy::GeometryContained),
            [("points".to_string(), Some(0))],
            "the clip cuts the bar: {shape:?}"
        );
    }
}
