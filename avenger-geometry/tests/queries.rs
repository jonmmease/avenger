//! Queries relate each instance's visible shape to a query shape by a DE-9IM relation, and find
//! the instances nearest a point.

use avenger_geometry::query::{Extent, Metric, Nearest, Pattern, Query, QueryShape, Relation};
use avenger_geometry::rtree::{GeometryInstance, SceneGraphRTree};
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
use geo_types::{coord, Geometry, LineString, Point, Rect};

fn instance(index: usize, geometry: Geometry<f32>, radius: f32, margin: f32) -> GeometryInstance {
    GeometryInstance {
        mark_instance: MarkInstance {
            name: "probe".into(),
            mark_path: vec![0],
            instance_index: Some(index),
        },
        geometry,
        radius,
        margin,
    }
}

fn empty_scene() -> SceneGraph {
    SceneGraph {
        width: 100.0,
        height: 100.0,
        origin: [0.0, 0.0],
        marks: Vec::new(),
    }
}

/// A tree of `instances`, drawn in order, so the last is topmost.
fn tree(instances: Vec<GeometryInstance>) -> SceneGraphRTree {
    let mut tree = SceneGraphRTree::from_scene_graph(&empty_scene(), &bundled_label_engine());
    for instance in instances {
        tree.insert(instance);
    }
    tree
}

fn found(tree: &SceneGraphRTree, query: &Query) -> Vec<usize> {
    (tree.query(query).iter())
        .filter_map(|instance| instance.mark_instance.instance_index)
        .collect()
}

fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Geometry<f32> {
    Rect::new(coord! { x: x0, y: y0 }, coord! { x: x1, y: y1 }).into()
}

fn line(points: &[(f32, f32)]) -> Geometry<f32> {
    LineString::from(points.to_vec()).into()
}

fn point(x: f32, y: f32) -> Geometry<f32> {
    Point::new(x, y).into()
}

/// Whether a candidate with `geometry`, grown by `radius` and `margin`, stands in `relation` to
/// `shape`, at the drawn extent.
fn relates(candidate: GeometryInstance, shape: QueryShape, relation: Relation) -> bool {
    !tree(vec![candidate])
        .query(&Query::new(shape, relation))
        .is_empty()
}

fn square() -> GeometryInstance {
    instance(0, rect(0.0, 0.0, 10.0, 10.0), 0.0, 0.0)
}

fn within_rect(x0: f32, y0: f32, x1: f32, y1: f32) -> QueryShape<'static> {
    QueryShape::Rect {
        min: [x0, y0],
        max: [x1, y1],
    }
}

/// Each relation is its DE-9IM predicate, with the candidate first: a square against rects that equal
/// it, miss it, touch it, overlap it, hold it, lie inside it, and lie inside it on its edge.
#[test]
fn relations_are_their_predicates_with_the_candidate_first() {
    use Relation::*;
    let all = [
        Equals,
        Disjoint,
        Intersects,
        Touches,
        Within,
        Contains,
        Overlaps,
        Covers,
        CoveredBy,
        ContainsProperly,
        InteriorsIntersect,
    ];
    let cases: [(&str, QueryShape, &[Relation]); 7] = [
        (
            "equal",
            within_rect(0.0, 0.0, 10.0, 10.0),
            &[
                Equals,
                Intersects,
                Within,
                Contains,
                Covers,
                CoveredBy,
                InteriorsIntersect,
            ],
        ),
        ("apart", within_rect(20.0, 0.0, 30.0, 10.0), &[Disjoint]),
        (
            "touching",
            within_rect(10.0, 0.0, 20.0, 10.0),
            &[Intersects, Touches],
        ),
        (
            "overlapping",
            within_rect(5.0, 0.0, 15.0, 10.0),
            &[Intersects, Overlaps, InteriorsIntersect],
        ),
        (
            "around it",
            within_rect(-5.0, -5.0, 15.0, 15.0),
            &[Intersects, Within, CoveredBy, InteriorsIntersect],
        ),
        (
            "inside it",
            within_rect(2.0, 2.0, 8.0, 8.0),
            &[
                Intersects,
                Contains,
                Covers,
                ContainsProperly,
                InteriorsIntersect,
            ],
        ),
        (
            "inside it on its edge",
            within_rect(0.0, 0.0, 5.0, 5.0),
            &[Intersects, Contains, Covers, InteriorsIntersect],
        ),
    ];
    for (name, shape, expected) in cases {
        for relation in &all {
            assert_eq!(
                relates(square(), shape.clone(), *relation),
                expected.contains(relation),
                "{name}: {relation:?}"
            );
        }
    }
}

/// A pattern matches the DE-9IM matrix cell by cell, and only nine cells of `T`, `F`, `*`, `0`,
/// `1`, and `2` make a pattern.
#[test]
fn patterns_match_the_matrix() {
    let within = Relation::Pattern(Pattern::new("T*F**F***").unwrap());
    assert!(relates(
        square(),
        within_rect(-5.0, -5.0, 15.0, 15.0),
        within
    ));
    assert!(!relates(
        square(),
        within_rect(5.0, 0.0, 15.0, 10.0),
        within
    ));
    let areas_overlap = Relation::Pattern(Pattern::new("2********").unwrap());
    assert!(relates(
        square(),
        within_rect(5.0, 0.0, 15.0, 10.0),
        areas_overlap
    ));
    for bad in ["T*F**F**", "T*F**F***F", "T*X**F***"] {
        assert!(Pattern::new(bad).is_err(), "{bad}");
    }

    // A pattern that holds for shapes apart finds instances far from the query shape, and one
    // with no fill, whose empty shape meets nothing.
    let apart = Relation::Pattern(Pattern::new("FF*FF****").unwrap());
    let far = within_rect(80.0, 80.0, 90.0, 90.0);
    assert!(relates(square(), far.clone(), apart));
    assert!(!relates(square(), within_rect(5.0, 0.0, 15.0, 10.0), apart));
    let tree = tree(vec![instance(
        0,
        line(&[(0.0, 0.0), (10.0, 0.0)]),
        0.0,
        0.0,
    )]);
    let fill = |pattern| {
        let query = Query::new(
            far.clone(),
            Relation::Pattern(Pattern::new(pattern).unwrap()),
        );
        found(&tree, &query.extent(Extent::Fill))
    };
    assert_eq!(fill("FF*FF***2"), [0]);
    assert!(fill("T********").is_empty());
    assert!(fill("FF*FF*F**").is_empty());
}

/// An instance's own shape is a query shape: a stroked circle stands in every reflexive relation
/// to itself, and in none that needs two shapes.
#[test]
fn an_instance_relates_to_itself() {
    use Relation::*;
    {
        let shape = instance(0, point(50.0, 50.0), 5.0, 1.0);
        let tree = tree(vec![shape.clone()]);
        let geometry = &shape.geometry;
        for relation in [
            Equals,
            Intersects,
            Within,
            Contains,
            Covers,
            CoveredBy,
            InteriorsIntersect,
        ] {
            let query = Query::new(QueryShape::Instance(&shape), relation);
            assert_eq!(found(&tree, &query), [0], "{geometry:?}: {relation:?}");
        }
        for relation in [Disjoint, Touches, Overlaps, Crosses, ContainsProperly] {
            let query = Query::new(QueryShape::Instance(&shape), relation);
            assert!(
                found(&tree, &query).is_empty(),
                "{geometry:?}: {relation:?}"
            );
        }
    }
}

/// A filter skips instances before any geometry, and a limit returns the topmost matches.
#[test]
fn filters_and_limits_choose_among_matches() {
    let pile: Vec<_> = (0..6)
        .map(|index| instance(index, point(50.0, 50.0), 4.0, 0.0))
        .collect();
    let tree = tree(pile.clone());
    let top = &pile[5];
    let stack = |limit: usize| {
        found(
            &tree,
            &Query::new(QueryShape::Instance(top), Relation::InteriorsIntersect)
                .extent(Extent::Fill)
                .limit(limit),
        )
    };
    assert_eq!(stack(3), [5, 4, 3]);
    assert_eq!(stack(10), [5, 4, 3, 2, 1, 0]);
    let even = Query::new(QueryShape::Point([50.0, 50.0]), Relation::Intersects)
        .filter(|mark| mark.instance_index.is_some_and(|index| index % 2 == 0));
    assert_eq!(found(&tree, &even), [4, 2, 0]);
}

/// Neighbors with outlines share interior where their strokes draw over each other, at the drawn
/// extent, and only touch at the fill extent.
#[test]
fn strokes_overlap_where_fills_only_touch() {
    let left = instance(0, rect(0.0, 0.0, 10.0, 10.0), 0.0, 0.5);
    let right = instance(1, rect(10.0, 0.0, 20.0, 10.0), 0.0, 0.5);
    let tree = tree(vec![left.clone(), right]);
    let neighbors = |extent| {
        found(
            &tree,
            &Query::new(QueryShape::Instance(&left), Relation::InteriorsIntersect).extent(extent),
        )
    };
    assert_eq!(neighbors(Extent::Drawn), [1, 0]);
    assert_eq!(neighbors(Extent::Fill), [0]);
}

/// `nearest` yields instances nearest first, topmost first among equal distances, within its
/// largest distance, which can be infinite.
#[test]
fn nearest_finds_instances_nearest_first() {
    let tree = tree(vec![
        instance(0, point(12.0, 10.0), 0.0, 0.0),
        instance(1, point(10.0, 11.0), 0.0, 0.0),
        instance(2, point(8.0, 10.0), 0.0, 0.0),
        instance(3, point(50.0, 50.0), 0.0, 0.0),
    ]);
    let order = |query: Nearest| -> Vec<usize> {
        (tree.nearest(query))
            .filter_map(|near| near.instance.mark_instance.instance_index)
            .collect()
    };
    assert_eq!(order(Nearest::new([10.0, 10.0], 5.0)), [1, 2, 0]);
    assert_eq!(
        order(Nearest::new([10.0, 10.0], f32::INFINITY)),
        [1, 2, 0, 3]
    );
    let odd = Nearest::new([10.0, 10.0], 5.0)
        .filter(|mark| mark.instance_index.is_some_and(|index| index % 2 == 1));
    assert_eq!(order(odd), [1]);
    // An instance at the largest distance is within it.
    let edge = tree_of_one(point(13.0, 14.0));
    let found: Vec<_> = edge.nearest(Nearest::new([10.0, 10.0], 5.0)).collect();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].distance, 5.0);
}

fn tree_of_one(geometry: Geometry<f32>) -> SceneGraphRTree {
    tree(vec![instance(0, geometry, 0.0, 0.0)])
}

/// Taking the first neighbor of each mark finds every series' nearest instance, nearest series
/// first, as a tooltip that lists each series needs.
#[test]
fn the_first_neighbor_of_each_mark_is_its_nearest() {
    let in_mark = |mark: usize, index: usize, x: f32, y: f32| {
        let mut instance = instance(index, point(x, y), 0.0, 0.0);
        instance.mark_instance.mark_path = vec![mark];
        instance
    };
    let tree = tree(vec![
        in_mark(0, 0, 0.0, 0.0),
        in_mark(0, 1, 8.0, 0.0),
        in_mark(1, 0, 13.0, 0.0),
        in_mark(1, 1, 30.0, 0.0),
        in_mark(2, 0, 50.0, 0.0),
    ]);
    let mut seen = Vec::new();
    for near in tree.nearest(Nearest::new([10.0, 0.0], 25.0).metric(Metric::AlongX)) {
        let mark = &near.instance.mark_instance;
        if !seen.iter().any(|(path, _)| path == &mark.mark_path) {
            seen.push((mark.mark_path.clone(), mark.instance_index.unwrap()));
        }
    }
    assert_eq!(seen, [(vec![0], 1), (vec![1], 0)]);
}

/// Each instance that `nearest` finds from `point` within `max` by `metric`, with its distance.
fn neighbors(
    tree: &SceneGraphRTree,
    point: [f32; 2],
    max: f32,
    metric: Metric,
) -> Vec<(usize, f32)> {
    (tree.nearest(Nearest::new(point, max).metric(metric)))
        .map(|near| {
            (
                near.instance.mark_instance.instance_index.unwrap(),
                near.distance,
            )
        })
        .collect()
}

/// Along an axis, distance counts only that axis, so far instances in the other one are near, and
/// equal distances go nearest in the plane first: of two stacked bars, the one under the pointer.
#[test]
fn nearest_along_an_axis_ignores_the_other_one() {
    let points = tree(vec![
        instance(0, point(10.0, 90.0), 0.0, 0.0),
        instance(1, point(10.0, 20.0), 0.0, 0.0),
        instance(2, point(13.0, 10.0), 0.0, 0.0),
        instance(3, point(30.0, 10.0), 0.0, 0.0),
    ]);
    assert_eq!(
        neighbors(&points, [10.0, 10.0], 5.0, Metric::AlongX),
        [(1, 0.0), (0, 0.0), (2, 3.0)]
    );
    assert_eq!(
        neighbors(&points, [10.0, 10.0], 10.0, Metric::AlongY),
        [(2, 0.0), (3, 0.0), (1, 10.0)]
    );
    let stacked = tree(vec![
        instance(0, rect(0.0, 0.0, 10.0, 100.0), 0.0, 0.0),
        instance(1, rect(0.0, 100.0, 10.0, 200.0), 0.0, 0.0),
    ]);
    for (y, under) in [(50.0, 0), (130.0, 1)] {
        let found = neighbors(&stacked, [5.0, y], 1.0, Metric::AlongX);
        assert_eq!(found[0], (under, 0.0), "{y}");
    }
}

/// The distance is to each instance's shape at the query's extent: a stroked circle's drawn
/// edge lies nearer than its fill's.
#[test]
fn nearest_measures_to_the_extent() {
    let tree = tree(vec![instance(0, point(0.0, 0.0), 5.0, 1.0)]);
    let at = |extent| {
        tree.nearest(Nearest::new([5.5, 0.0], 0.25).extent(extent))
            .count()
    };
    assert_eq!(at(Extent::Drawn), 1);
    assert_eq!(at(Extent::Fill), 0);
}

/// Points in a clipped group, the first hidden by the clip and the second half shown by it.
fn clipped_points() -> SceneGraphRTree {
    let points = SceneSymbolMark {
        name: "points".into(),
        len: 2,
        x: vec![70.0, 50.0].into(),
        y: vec![20.0, 20.0].into(),
        size: 100.0.into(),
        ..Default::default()
    };
    let scene = SceneGraph {
        marks: vec![SceneMark::Group(SceneGroup {
            clip: Clip::Rect {
                x: 0.0,
                y: 0.0,
                width: 50.0,
                height: 50.0,
            },
            marks: vec![points.into()],
            ..Default::default()
        })],
        ..empty_scene()
    };
    SceneGraphRTree::from_scene_graph(&scene, &bundled_label_engine())
}

fn indices<'t>(instances: impl IntoIterator<Item = &'t GeometryInstance>) -> Vec<usize> {
    (instances.into_iter())
        .filter_map(|instance| instance.mark_instance.instance_index)
        .collect()
}

/// An instance that its clip hides is never found, and one that its clip shows in part is found
/// by its whole shape: of the points at x = 70 and x = 50 under a clip that ends at x = 50, the
/// first lies nearer (62, 20), but only the second is found, at the distance to its whole disc.
#[test]
fn clips_hide_the_instances_that_they_show_none_of() {
    let tree = clipped_points();
    let found: Vec<_> = tree
        .nearest(Nearest::new([62.0, 20.0], f32::INFINITY))
        .collect();
    assert_eq!(indices(found.iter().map(|near| near.instance)), [1]);
    assert_eq!(found[0].distance, 7.0);
    let around_both = QueryShape::Rect {
        min: [40.0, 10.0],
        max: [80.0, 30.0],
    };
    let lasso = Query::new(around_both, Relation::CoveredBy);
    assert_eq!(indices(tree.query(&lasso)), [1]);
}

/// A query at a point that a clip hides sees nothing of the clip's marks, such as the pointer
/// over the part of a bar that its clip hides.
#[test]
fn queries_where_a_clip_hides_its_marks_see_none_of_them() {
    let bar = SceneRectMark {
        x: 0.0.into(),
        y: 0.0.into(),
        width: Some(20.0.into()),
        height: Some(10.0.into()),
        ..Default::default()
    };
    let scene = SceneGraph {
        marks: vec![SceneMark::Group(SceneGroup {
            clip: Clip::Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 20.0,
            },
            marks: vec![bar.into()],
            ..Default::default()
        })],
        ..empty_scene()
    };
    let tree = SceneGraphRTree::from_scene_graph(&scene, &bundled_label_engine());
    let hits = |point| {
        !tree
            .query(&Query::new(QueryShape::Point(point), Relation::Intersects))
            .is_empty()
    };
    assert!(hits([9.0, 5.0]));
    assert!(!hits([11.0, 5.0]));
}

/// A clip path shows or hides each instance whole: of discs at (10, 10), (45, 50), and (60, 60)
/// under a triangle whose edge runs along x + y = 100, it hides the third, and the second, which
/// crosses its edge, counts by its whole disc.
#[test]
fn clip_paths_show_or_hide_whole_instances() {
    let mut triangle = lyon_path::Path::builder();
    triangle.begin(lyon_path::math::point(0.0, 0.0));
    triangle.line_to(lyon_path::math::point(100.0, 0.0));
    triangle.line_to(lyon_path::math::point(0.0, 100.0));
    triangle.end(true);
    let discs = SceneSymbolMark {
        len: 3,
        x: vec![10.0, 45.0, 60.0].into(),
        y: vec![10.0, 50.0, 60.0].into(),
        size: 100.0.into(),
        ..Default::default()
    };
    let scene = SceneGraph {
        marks: vec![SceneMark::Group(SceneGroup {
            clip: Clip::Path(triangle.build()),
            marks: vec![discs.into()],
            ..Default::default()
        })],
        ..empty_scene()
    };
    let tree = SceneGraphRTree::from_scene_graph(&scene, &bundled_label_engine());
    let everything = within_rect(0.0, 0.0, 100.0, 100.0);
    let query = |relation| found(&tree, &Query::new(everything.clone(), relation));
    assert_eq!(query(Relation::CoveredBy), [1, 0]);
    assert_eq!(query(Relation::Intersects), [1, 0]);
    let near: Vec<_> = tree
        .nearest(Nearest::new([70.0, 70.0], f32::INFINITY))
        .collect();
    assert_eq!(indices(near.iter().map(|near| near.instance)), [1, 0]);
    assert!(
        (near[0].distance - (1025.0f32.sqrt() - 5.0)).abs() < 1e-3,
        "{}",
        near[0].distance
    );
}

/// A rect with no height acts as the segment between its corners, and one with no size as its
/// point, so a drag that hasn't moved yet relates like the pointer.
#[test]
fn flat_rects_act_as_their_segments_and_points() {
    use Relation::*;
    for flat in [
        within_rect(2.0, 5.0, 8.0, 5.0),
        within_rect(5.0, 5.0, 5.0, 5.0),
    ] {
        for relation in [Contains, ContainsProperly, InteriorsIntersect, Covers] {
            assert!(
                relates(square(), flat.clone(), relation),
                "{flat:?} {relation:?}"
            );
        }
    }
    let edge = within_rect(0.0, 2.0, 0.0, 8.0);
    assert!(relates(square(), edge.clone(), Touches));
    assert!(!relates(square(), edge, InteriorsIntersect));
}
