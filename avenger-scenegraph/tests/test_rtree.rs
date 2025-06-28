use avenger_common::{
    canvas::CanvasDimensions,
    types::{AreaOrientation, SymbolShape},
};
use avenger_geometry::marks::MarkGeometryUtils;
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::marks::{
    area::SceneAreaMark, symbol::SceneSymbolMark, text::SceneTextMark,
};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_text::rasterization::cosmic::CosmicTextRasterizer;
use float_cmp::assert_approx_eq;
use geo::BoundingRect;
use geo_svg::ToSvg;
use rstar::{PointDistance, AABB};

#[test]
fn test_symbol_rtree_single() {
    // Create a symbol mark with a single circle at (1,1) with size 2
    let mark = SceneSymbolMark {
        shapes: vec![SymbolShape::Circle],
        x: vec![1.0].into(),
        y: vec![1.0].into(),
        size: vec![4.0].into(),
        angle: vec![0.0].into(),
        shape_index: vec![0].into(),
        ..Default::default()
    };

    let scene_graph = SceneGraph {
        marks: vec![mark.into()],
        width: 5.0,
        height: 5.0,
        origin: [0.0, 0.0],
    };
    let rtree = SceneGraphRTree::from_scene_graph(&scene_graph);

    // Test point inside the circle
    let nearest = rtree.locate_at_point(&[1.0, 1.0]).unwrap();
    assert_eq!(nearest.mark_instance.instance_index, Some(0));

    // Test point outside but close. Locate should return None
    assert!(rtree.locate_at_point(&[2.5, 1.0]).is_none());

    // Nearest should return the circle, with distance 0.5
    let nearest: &avenger_geometry::GeometryInstance = rtree.nearest_neighbor(&[2.5, 1.0]).unwrap();
    assert_eq!(nearest.mark_instance.instance_index, Some(0));
    assert_approx_eq!(f32, nearest.distance_2(&[2.5, 1.0]), 0.5);

    // Check bounding box
    let bbox = nearest.geometry.bounding_rect().unwrap();
    assert!((bbox.width() - 2.0).abs() < 0.1); // Size 2.0, area 4.0
}

#[test]
fn test_symbol_rtree_multiple() {
    // Create a symbol mark with two symbols at different locations
    let mark = SceneSymbolMark {
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
    };

    let scene_graph = SceneGraph {
        marks: vec![mark.into()],
        width: 5.0,
        height: 5.0,
        origin: [0.0, 0.0],
    };
    let rtree = SceneGraphRTree::from_scene_graph(&scene_graph);

    // Test nearest to first symbol
    let nearest = rtree.nearest_neighbor(&[0.2, 0.2]).unwrap();
    assert_eq!(nearest.mark_instance.instance_index, Some(0));
    assert_approx_eq!(f32, nearest.distance_2(&[0.2, 0.2]), 0.0);

    // Test nearest to second symbol
    let nearest = rtree.nearest_neighbor(&[3.0, 0.0]).unwrap();
    assert_eq!(nearest.mark_instance.instance_index, Some(1));
    assert_approx_eq!(f32, nearest.distance_2(&[3.0, 0.0]), 0.0);

    // Test that we get both symbols in order of distance
    let nearest_two: Vec<_> = rtree.nearest_neighbor_iter(&[1.5, 0.0]).take(2).collect();
    assert_eq!(nearest_two.len(), 2);
    assert_eq!(nearest_two[0].mark_instance.instance_index, Some(0));
    assert_eq!(nearest_two[1].mark_instance.instance_index, Some(1));
}

#[test]
fn test_symbol_rtree_rotation() {
    // Create a symbol mark with a rotated rectangle
    let mark = SceneSymbolMark {
        shapes: vec![SymbolShape::from_vega_str("square").unwrap()],
        x: vec![1.0].into(),
        y: vec![1.0].into(),
        size: vec![4.0].into(),
        angle: vec![45.0].into(), // 45 degree rotation
        shape_index: vec![0].into(),
        ..Default::default()
    };

    let scene_graph = SceneGraph {
        marks: vec![mark.into()],
        width: 5.0,
        height: 5.0,
        origin: [0.0, 0.0],
    };
    let rtree = SceneGraphRTree::from_scene_graph(&scene_graph);

    let geometry = rtree.nearest_neighbor(&[1.0, 1.0]).unwrap();

    // The bounding box should be larger due to rotation
    let bbox = geometry.geometry.bounding_rect().unwrap();
    assert!(bbox.width() > 2.0); // Diagonal is longer than side
    assert!((bbox.width() - bbox.height()).abs() < 0.1); // Should be square
}

#[test]
fn test_symbol_rtree_empty() {
    // Create an empty symbol mark
    let mark = SceneSymbolMark {
        len: 0,
        ..Default::default()
    };

    let scene_graph = SceneGraph {
        marks: vec![mark.into()],
        width: 5.0,
        height: 5.0,
        origin: [0.0, 0.0],
    };
    let rtree = SceneGraphRTree::from_scene_graph(&scene_graph);

    // Should return None for nearest neighbor
    assert!(rtree.nearest_neighbor(&[0.0, 0.0]).is_none());
}

#[test]
fn test_symbol_rtree_spatial_query() {
    let mark = SceneSymbolMark {
        len: 3,
        shapes: vec![SymbolShape::Circle],
        x: vec![0.0, 2.0, 4.0].into(),
        y: vec![0.0, 0.0, 0.0].into(),
        size: vec![1.0, 1.0, 1.0].into(),
        angle: vec![0.0, 0.0, 0.0].into(),
        shape_index: vec![0, 0, 0].into(),
        ..Default::default()
    };

    let scene_graph = SceneGraph {
        marks: vec![mark.into()],
        width: 5.0,
        height: 5.0,
        origin: [0.0, 0.0],
    };
    let rtree = SceneGraphRTree::from_scene_graph(&scene_graph);

    // Query a box that should contain the middle circle
    let query_box = AABB::from_corners([1.0, -1.0], [3.0, 1.0]);
    let results: Vec<_> = rtree.locate_in_envelope(&query_box).collect();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].mark_instance.instance_index, Some(1));
    assert_eq!(results[0].mark_instance.mark_path, vec![0]);
}

#[test]
fn test_stacked_area_rtree() {
    // Create two area marks, stacked vertically
    let upper_mark = SceneAreaMark {
        len: 3,
        stroke_width: 0.0,
        x: vec![0.0, 2.0, 4.0].into(),
        y: vec![0.0, 0.0, 0.0].into(),
        y2: vec![1.0, 2.0, 1.0].into(),
        orientation: AreaOrientation::Vertical,
        ..Default::default()
    };

    let lower_mark = SceneAreaMark {
        len: 3,
        stroke_width: 0.0,
        x: vec![0.0, 2.0, 4.0].into(),
        y: vec![1.0, 2.0, 1.0].into(),
        y2: vec![2.0, 3.0, 2.0].into(),
        orientation: AreaOrientation::Vertical,
        ..Default::default()
    };

    // Create the rtree
    let scene_graph = SceneGraph {
        marks: vec![upper_mark.into(), lower_mark.into()],
        width: 5.0,
        height: 5.0,
        origin: [0.0, 0.0],
    };
    let rtree = SceneGraphRTree::from_scene_graph(&scene_graph);

    // Test spatial query
    let instance = rtree
        // .pick_mark_at_point(&[2.0, 1.9])
        .locate_at_point(&[2.0, 1.9])
        .expect("Expected an instance at point");

    println!("{:?}", instance);
    assert_eq!(instance.mark_instance.instance_index, None);
    assert_eq!(instance.mark_instance.mark_path, vec![0]);
}

#[test]
fn test_text_rtree() {
    let mark = SceneTextMark {
        len: 1,
        x: vec![0.0].into(),
        y: vec![0.0].into(),
        text: vec!["0".to_string()].into(),
        ..Default::default()
    };

    let _rasterizer = CosmicTextRasterizer::<()>::new();
    let _dimensions = CanvasDimensions {
        size: [100.0, 100.0],
        scale: 1.0,
    };

    let geometries: Vec<_> = mark.geometry_iter(vec![0], [0.0, 0.0]).collect();
    // let rtree = MarkRTree::new(geometries);

    println!("{}", geometries[0].geometry.to_svg().svg_str())
    // let instance = rtree.locate_at_point(&[0.0, 0.0]).unwrap();
    // assert_eq!(instance.instance_index, Some(0));
}
