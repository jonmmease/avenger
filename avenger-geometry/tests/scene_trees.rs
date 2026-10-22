//! Trees built from scenes hold each symbol and area where it draws, for queries and `nearest`.

use avenger_common::types::{AreaOrientation, SymbolShape};
use avenger_geometry::query::{Nearest, Query, QueryShape, Relation};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::marks::{area::SceneAreaMark, symbol::SceneSymbolMark};
use avenger_scenegraph::scene_graph::SceneGraph;
use geo::BoundingRect;

fn tree(marks: Vec<avenger_scenegraph::marks::mark::SceneMark>) -> SceneGraphRTree {
    let scene_graph = SceneGraph {
        marks,
        width: 5.0,
        height: 5.0,
        origin: [0.0, 0.0],
    };
    SceneGraphRTree::from_scene_graph(&scene_graph, &avenger_typst_label::bundled_label_engine())
}

fn indices<'t>(
    instances: impl Iterator<Item = &'t avenger_geometry::GeometryInstance>,
) -> Vec<Option<usize>> {
    instances
        .map(|instance| instance.mark_instance.instance_index)
        .collect()
}

fn neighbors(tree: &SceneGraphRTree, query: Nearest) -> Vec<Option<usize>> {
    indices(tree.nearest(query).map(|near| near.instance))
}

#[test]
fn test_symbol_rtree_single() {
    // A circle at (1, 1) with size 4, so radius 1
    let tree = tree(vec![SceneSymbolMark {
        shapes: vec![SymbolShape::Circle],
        x: vec![1.0].into(),
        y: vec![1.0].into(),
        size: vec![4.0].into(),
        angle: vec![0.0].into(),
        shape_index: vec![0].into(),
        ..Default::default()
    }
    .into()]);
    let hits = |point| {
        indices(
            tree.query(&Query::new(QueryShape::Point(point), Relation::Intersects))
                .into_iter(),
        )
    };

    // A point inside the circle hits it, and one half a unit outside doesn't
    assert_eq!(hits([1.0, 1.0]), [Some(0)]);
    assert!(hits([2.5, 1.0]).is_empty());

    // The circle lies half a unit from that point
    assert_eq!(neighbors(&tree, Nearest::new([2.5, 1.0], 0.5)), [Some(0)]);
    assert!(tree
        .nearest(Nearest::new([2.5, 1.0], 0.49))
        .next()
        .is_none());

    // Its box is its center grown by its radius
    let nearest = tree.nearest(Nearest::new([2.5, 1.0], 1.0)).next().unwrap();
    assert_eq!(nearest.distance, 0.5);
    let envelope = nearest.instance.envelope();
    assert!((envelope.upper()[0] - envelope.lower()[0] - 2.0).abs() < 0.1);
}

#[test]
fn test_symbol_rtree_multiple() {
    // A circle and a square, each one unit across
    let tree = tree(vec![SceneSymbolMark {
        len: 2,
        shapes: vec![
            SymbolShape::Circle,
            SymbolShape::from_vega_str("square").unwrap(),
        ],
        x: vec![0.0, 3.0].into(),
        y: vec![0.0, 0.0].into(),
        size: vec![1.0, 1.0].into(),
        angle: vec![0.0, 0.0].into(),
        shape_index: vec![0, 1].into(),
        ..Default::default()
    }
    .into()]);

    // A point inside each symbol lies at no distance from it
    assert_eq!(neighbors(&tree, Nearest::new([0.2, 0.2], 0.0)), [Some(0)]);
    assert_eq!(neighbors(&tree, Nearest::new([3.0, 0.0], 0.0)), [Some(1)]);

    // Halfway between, both lie one unit away, and the topmost comes first
    let both = neighbors(&tree, Nearest::new([1.5, 0.0], f32::INFINITY));
    assert_eq!(both, [Some(1), Some(0)]);
}

#[test]
fn test_symbol_rtree_rotation() {
    // A square turned 45°
    let tree = tree(vec![SceneSymbolMark {
        shapes: vec![SymbolShape::from_vega_str("square").unwrap()],
        x: vec![1.0].into(),
        y: vec![1.0].into(),
        size: vec![4.0].into(),
        angle: vec![45.0].into(),
        shape_index: vec![0].into(),
        ..Default::default()
    }
    .into()]);
    let square = tree
        .nearest(Nearest::new([1.0, 1.0], 0.0))
        .next()
        .unwrap()
        .instance;

    // Its box is wider than its side, since its diagonal lies along each axis
    let bbox = square.geometry.bounding_rect().unwrap();
    assert!(bbox.width() > 2.0);
    assert!((bbox.width() - bbox.height()).abs() < 0.1);
}

#[test]
fn test_symbol_rtree_empty() {
    let tree = tree(vec![SceneSymbolMark {
        len: 0,
        ..Default::default()
    }
    .into()]);
    assert!(tree
        .nearest(Nearest::new([0.0, 0.0], f32::INFINITY))
        .next()
        .is_none());
}

#[test]
fn test_symbol_rtree_spatial_query() {
    // Three circles in a row, and a box that holds only the middle one
    let tree = tree(vec![SceneSymbolMark {
        len: 3,
        shapes: vec![SymbolShape::Circle],
        x: vec![0.0, 2.0, 4.0].into(),
        y: vec![0.0, 0.0, 0.0].into(),
        size: vec![1.0, 1.0, 1.0].into(),
        angle: vec![0.0, 0.0, 0.0].into(),
        shape_index: vec![0, 0, 0].into(),
        ..Default::default()
    }
    .into()]);
    let held = tree.query(&Query::new(
        QueryShape::Rect {
            min: [1.0, -1.0],
            max: [3.0, 1.0],
        },
        Relation::CoveredBy,
    ));
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].mark_instance.instance_index, Some(1));
    assert_eq!(held[0].mark_instance.mark_path, vec![0]);
}

#[test]
fn test_stacked_area_rtree() {
    // Two areas, the second stacked on the first
    let lower = SceneAreaMark {
        len: 3,
        stroke_width: 0.0,
        x: vec![0.0, 2.0, 4.0].into(),
        y: vec![0.0, 0.0, 0.0].into(),
        y2: vec![1.0, 2.0, 1.0].into(),
        orientation: AreaOrientation::Vertical,
        ..Default::default()
    };
    let upper = SceneAreaMark {
        len: 3,
        stroke_width: 0.0,
        x: vec![0.0, 2.0, 4.0].into(),
        y: vec![1.0, 2.0, 1.0].into(),
        y2: vec![2.0, 3.0, 2.0].into(),
        orientation: AreaOrientation::Vertical,
        ..Default::default()
    };
    let tree = tree(vec![lower.into(), upper.into()]);

    // The point lies inside the first area, a tenth of a unit from the second
    let hits = tree.query(&Query::new(
        QueryShape::Point([2.0, 1.9]),
        Relation::Intersects,
    ));
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].mark_instance.instance_index, None);
    assert_eq!(hits[0].mark_instance.mark_path, vec![0]);
}
