use std::sync::Arc;

use avenger_common::{
    types::AreaOrientation,
    canvas::CanvasDimensions
};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::marks::{
    area::{SceneAreaMark},
    symbol::{SceneSymbolMark, SymbolShape},
    text::SceneTextMark,
};
use avenger_text::rasterization::{cosmic::CosmicTextRasterizer, TextRasterizer};
use float_cmp::assert_approx_eq;
use geo::BoundingRect;
use geo_svg::ToSvg;
use rstar::{PointDistance, AABB};
use avenger_geometry::marks::MarkGeometryUtils;
use avenger_scenegraph::scene_graph::SceneGraph;

#[test]
fn test_symbol_rtree_single() {
    // Create a symbol mark with a single circle at (1,1) with size 2
    let mut mark = SceneSymbolMark::default();
    mark.shapes = vec![SymbolShape::Circle].into();
    mark.x = vec![1.0].into();
    mark.y = vec![1.0].into();
    mark.size = vec![4.0].into();
    mark.angle = vec![0.0].into();
    mark.shape_index = vec![0].into();

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
    let mut mark = SceneSymbolMark::default();
    mark.len = 2;
    mark.shapes = vec![
        SymbolShape::Circle,
        SymbolShape::from_vega_str("square").unwrap(),
    ]
    .into();
    mark.x = vec![0.0, 3.0].into();
    mark.y = vec![0.0, 0.0].into();
    mark.size = vec![1.0, 1.0].into();
    mark.angle = vec![0.0, 0.0].into();
    mark.shape_index = vec![0, 1].into();

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
    let mut mark = SceneSymbolMark::default();
    mark.shapes = vec![SymbolShape::from_vega_str("square").unwrap()].into();
    mark.x = vec![1.0].into();
    mark.y = vec![1.0].into();
    mark.size = vec![4.0].into();
    mark.angle = vec![45.0].into(); // 45 degree rotation
    mark.shape_index = vec![0].into();

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
    let mut mark = SceneSymbolMark::default();
    mark.len = 0;

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
    let mut mark = SceneSymbolMark::default();
    mark.len = 3;
    mark.shapes = vec![SymbolShape::Circle].into();
    mark.x = vec![0.0, 2.0, 4.0].into();
    mark.y = vec![0.0, 0.0, 0.0].into();
    mark.size = vec![1.0, 1.0, 1.0].into();
    mark.angle = vec![0.0, 0.0, 0.0].into();
    mark.shape_index = vec![0, 0, 0].into();

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
    let mut upper_mark = SceneAreaMark::default();
    upper_mark.len = 3;
    upper_mark.stroke_width = 0.0;
    upper_mark.x = vec![0.0, 2.0, 4.0].into();
    upper_mark.y = vec![0.0, 0.0, 0.0].into();
    upper_mark.y2 = vec![1.0, 2.0, 1.0].into();
    upper_mark.orientation = AreaOrientation::Vertical;

    let mut lower_mark = upper_mark.clone();
    lower_mark.len = 3;
    lower_mark.stroke_width = 0.0;
    lower_mark.x = vec![0.0, 2.0, 4.0].into();
    lower_mark.y = vec![1.0, 2.0, 1.0].into();
    lower_mark.y2 = vec![2.0, 3.0, 2.0].into();
    lower_mark.orientation = AreaOrientation::Vertical;

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
    let mut mark = SceneTextMark::default();
    mark.len = 1;
    mark.x = vec![0.0].into();
    mark.y = vec![0.0].into();
    mark.text = vec!["0".to_string()].into();

    let rasterizer = CosmicTextRasterizer::<()>::new();
    let dimensions = CanvasDimensions {
        size: [100.0, 100.0],
        scale: 1.0,
    };

    let geometries: Vec<_> = mark
        .geometry_iter(vec![0], [0.0, 0.0])
        .collect();
    // let rtree = MarkRTree::new(geometries);

    println!("{}", geometries[0].geometry.to_svg().svg_str())
    // let instance = rtree.locate_at_point(&[0.0, 0.0]).unwrap();
    // assert_eq!(instance.instance_index, Some(0));
}
