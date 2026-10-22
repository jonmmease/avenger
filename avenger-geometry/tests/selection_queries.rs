use avenger_common::types::{FillRule, SymbolShape};
use avenger_geometry::query::{Extent, Query, QueryShape as Shape, Relation};
use avenger_geometry::{rtree::SceneGraphRTree, GeometryInstance};
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
        geometry,
        radius: 0.0,
        margin: reach,
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

/// The ways a selection takes instances: by their centers, by any part, or by their whole drawn
/// shape.
const CENTER: (Relation, Extent) = (Relation::CoveredBy, Extent::Centroid);
const TOUCH: (Relation, Extent) = (Relation::Intersects, Extent::Drawn);
const ENCLOSE: (Relation, Extent) = (Relation::CoveredBy, Extent::Drawn);

/// The instances that `shape` selects in `way`, by mark path and instance.
fn select<'t>(
    tree: &'t SceneGraphRTree,
    shape: &Shape,
    (relation, extent): (Relation, Extent),
) -> Vec<&'t GeometryInstance> {
    let mut found = tree.query(&Query::new(shape.clone(), relation).extent(extent));
    found.sort_by_key(|instance| {
        let mark = &instance.mark_instance;
        (mark.mark_path.clone(), mark.instance_index)
    });
    found
}

/// The instance indices that match.
fn matches(tree: &SceneGraphRTree, shape: &Shape, way: (Relation, Extent)) -> Vec<Option<usize>> {
    select(tree, shape, way)
        .iter()
        .map(|instance| instance.mark_instance.instance_index)
        .collect()
}

/// The mark instance of the topmost instance that `point` hits.
fn pick(tree: &SceneGraphRTree, point: [f32; 2]) -> Option<MarkInstance> {
    let query = Query::new(Shape::Point(point), Relation::Intersects);
    (tree.query(&query).first()).map(|instance| instance.mark_instance.clone())
}

#[test]
fn centroid_query_uses_triangle_centroid() {
    let triangle = Polygon::new(
        LineString::from(vec![(0.0, 0.0), (9.0, 0.0), (0.0, 9.0), (0.0, 0.0)]),
        vec![],
    );
    let tree = tree(triangle.into(), 0.0);
    let hits = select(
        &tree,
        &Shape::Circle {
            center: [3.0, 3.0],
            radius: 0.1,
        },
        CENTER,
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
    let hits = select(
        &tree,
        &Shape::Circle {
            center: [0.0, 0.0],
            radius: 10.0,
        },
        ENCLOSE,
    );
    assert_eq!(hits.len(), 1, "every point of diamond is within radius 9");
}

#[test]
fn rectangle_query_intersects_visible_stroke() {
    let tree = tree(LineString::from(vec![(0.0, 0.0), (20.0, 0.0)]).into(), 5.0);
    let rect = select(
        &tree,
        &Shape::Rect {
            min: [8.0, 3.0],
            max: [12.0, 4.0],
        },
        TOUCH,
    );
    let circle = select(
        &tree,
        &Shape::Circle {
            center: [10.0, 3.5],
            radius: 0.1,
        },
        TOUCH,
    );
    let polygon = select(
        &tree,
        &Shape::Polygon {
            contours: vec![vec![[8.0, 3.0], [12.0, 3.0], [12.0, 4.0], [8.0, 4.0]]],
            fill_rule: FillRule::NonZero,
        },
        TOUCH,
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
        center: [50.0 + middle * angle.sin(), 50.0 - middle * angle.cos()],
        radius: 2.0,
    };
    let center = Shape::Circle {
        center: [50.0, 50.0],
        radius: 2.0,
    };
    assert_eq!(matches(&tree, &inside, CENTER).len(), 1);
    assert!(matches(&tree, &center, CENTER).is_empty());
    assert!(matches(&tree, &center, TOUCH).is_empty());
}

#[test]
fn all_query_shapes_include_stroke_in_containment() {
    let tree = tree(Geometry::Point(geo_types::Point::new(5.0, 5.0)), 2.0);
    for half_extent in [1.0, 2.0, 3.0] {
        let lo = 5.0 - half_extent;
        let hi = 5.0 + half_extent;
        for shape in [
            Shape::Rect {
                min: [lo, lo],
                max: [hi, hi],
            },
            Shape::Circle {
                center: [5.0, 5.0],
                radius: half_extent,
            },
            Shape::Polygon {
                contours: vec![vec![[lo, lo], [hi, lo], [hi, hi], [lo, hi]]],
                fill_rule: FillRule::NonZero,
            },
        ] {
            assert_eq!(
                !select(&tree, &shape, ENCLOSE).is_empty(),
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
            assert!(pick(&tree, at(radius - 0.05)).is_some());
            assert!(pick(&tree, at(radius + 0.05)).is_none());
            let touching = Shape::Circle {
                center: [at(radius + 1.0)[0], at(radius + 1.0)[1]],
                radius: 1.05,
            };
            assert_eq!(matches(&tree, &touching, TOUCH).len(), 1);
        }
        let around = |extra: f32| Shape::Circle {
            center: [50.0, 50.0],
            radius: radius + extra,
        };
        assert_eq!(select(&tree, &around(0.05), ENCLOSE).len(), 1);
        assert!(select(&tree, &around(-0.05), ENCLOSE).is_empty());
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
    assert!(pick(&tree, [56.8, 50.0]).is_some());
    assert!(pick(&tree, [54.8, 54.8]).is_none());
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
        contours: vec![star.clone()],
        fill_rule,
    };
    // Points 0 and 1 are in the middle; point 2 is in a tip.
    assert_eq!(
        matches(&tree, &lasso(FillRule::NonZero), CENTER),
        [Some(0), Some(1), Some(2)]
    );
    assert_eq!(matches(&tree, &lasso(FillRule::EvenOdd), CENTER), [Some(2)]);
}

/// Marks in a clipped group match by their whole shapes, unless the clip hides them. The second
/// point is hidden and matches nothing. The bar crosses the clip's edge, so it matches by its
/// whole shape, except by its center, which lies where the clip hides it.
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
        select(&tree, shape, policy)
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
        min: [0.0, 0.0],
        max: [100.0, 100.0],
    };
    let lasso = Shape::Polygon {
        contours: vec![vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]]],
        fill_rule: FillRule::NonZero,
    };
    let circle = Shape::Circle {
        center: [50.0, 50.0],
        radius: 80.0,
    };
    for shape in [&everything, &lasso, &circle] {
        assert_eq!(
            names(shape, CENTER),
            [("points".to_string(), Some(0))],
            "{shape:?}"
        );
        assert_eq!(
            names(shape, TOUCH),
            [
                ("points".to_string(), Some(0)),
                ("bar".to_string(), Some(0))
            ],
            "{shape:?}"
        );
        assert_eq!(
            names(shape, ENCLOSE),
            [
                ("points".to_string(), Some(0)),
                ("bar".to_string(), Some(0))
            ],
            "{shape:?}"
        );
    }
}

/// Each instance of a mesh image hits where its own mesh draws, as that instance.
#[test]
fn mesh_images_hit_only_their_mesh() {
    use avenger_scenegraph::marks::mesh_image::{MeshImage, SceneMeshImageMark};
    let instance = |positions: Vec<[f32; 2]>| MeshImage {
        image: Default::default(),
        positions,
        uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
        indices: vec![0, 1, 2, 0, 2, 3],
    };
    // A parallelogram, whose bounding box reaches beyond it, and a square to its right.
    let tree = scene_tree(vec![SceneMeshImageMark {
        interactive: Some(true),
        instances: vec![
            instance(vec![[10.0, 10.0], [50.0, 10.0], [70.0, 50.0], [30.0, 50.0]]),
            instance(vec![
                [80.0, 10.0],
                [100.0, 10.0],
                [100.0, 30.0],
                [80.0, 30.0],
            ]),
        ],
        ..Default::default()
    }
    .into()]);
    let hit = |point: [f32; 2]| pick(&tree, point).map(|instance| instance.instance_index);
    assert_eq!(hit([40.0, 30.0]), Some(Some(0)));
    assert_eq!(hit([90.0, 20.0]), Some(Some(1)));
    // Inside the first instance's bounding box, left of its mesh.
    assert_eq!(hit([12.0, 45.0]), None);
}

/// A resource image hits where its declared size places it, before its pixels arrive.
#[test]
fn resource_images_hit_before_their_pixels_arrive() {
    use avenger_scenegraph::marks::image::{SceneImageMark, SceneImageResource};
    let tree = scene_tree(vec![SceneImageMark {
        interactive: Some(true),
        image: avenger_scenegraph::marks::image::SceneImageSource::from(SceneImageResource {
            url: "https://example.com/wide.png".to_string(),
            width: 40,
            height: 10,
            placeholder: None,
        })
        .into(),
        x: 10.0.into(),
        y: 10.0.into(),
        width: 40.0.into(),
        height: 40.0.into(),
        ..Default::default()
    }
    .into()]);
    let hit = |point: [f32; 2]| pick(&tree, point).is_some();
    // The aspect places the image in the box's middle 10 units.
    assert!(hit([30.0, 30.0]));
    assert!(!hit([30.0, 15.0]));
}

/// Path marks and transformed symbols hit where they fill under the nonzero rule: an inner square
/// with the outer square's winding is filled, and one with the opposite winding is a hole.
#[test]
fn compound_path_and_transformed_symbol_queries_follow_the_nonzero_rule() {
    use avenger_common::lyon::parse_svg_path;
    use avenger_scenegraph::marks::path::ScenePathMark;
    for (inner, filled) in [
        ("M10,10 H30 V30 H10 Z", true),
        ("M10,10 V30 H30 V10 Z", false),
    ] {
        for symbol in [false, true] {
            let path = parse_svg_path(&format!("M0,0 H40 V40 H0 Z {inner}")).unwrap();
            let mark = if symbol {
                SceneSymbolMark {
                    shapes: vec![SymbolShape::Path(path)],
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
            let tree = SceneGraphRTree::from_scene_graph(
                &scene,
                &avenger_typst_label::bundled_label_engine(),
            );
            let p = if symbol { [60.0, 60.0] } else { [20.0, 20.0] };
            assert_eq!(pick(&tree, p).is_some(), filled);
            assert_eq!(
                !select(
                    &tree,
                    &Shape::Circle {
                        center: [p[0], p[1]],
                        radius: 1.0
                    },
                    TOUCH
                )
                .is_empty(),
                filled
            );
            let outer = if symbol { [90.0, 30.0] } else { [5.0, 5.0] };
            assert!(pick(&tree, outer).is_some());
        }
    }
}

#[test]
fn dashed_line_singletons_remain_queryable_at_breaks_and_at_the_end() {
    use avenger_common::{types::StrokeCap, value::ScalarOrArray as S};
    use avenger_scenegraph::marks::line::SceneLineMark;
    let tree = SceneGraphRTree::from_scene_graph(
        &SceneGraph {
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
        },
        &avenger_typst_label::bundled_label_engine(),
    );
    for x in [20., 80.] {
        assert_eq!(
            select(
                &tree,
                &Shape::Circle {
                    center: [x, 50.],
                    radius: 1.
                },
                TOUCH
            )
            .len(),
            1
        );
    }
}
