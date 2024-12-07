use avenger_geometry::rtree::MarkRTree;
use avenger_scenegraph::marks::symbol::{SceneSymbolMark, SymbolShape};
use float_cmp::assert_approx_eq;
use geo::BoundingRect;
use rstar::{PointDistance, AABB};

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

    let rtree = MarkRTree::new(mark.geometry_iter(0).collect());

    // Test point inside the circle
    let nearest = rtree.locate_at_point(&[1.0, 1.0]).unwrap();
    assert_eq!(nearest.instance_idx, Some(0));

    // Test point outside but close. Locate should return None
    assert!(rtree.locate_at_point(&[2.5, 1.0]).is_none());

    // Nearest should return the circle, with distance 0.5
    let nearest: &avenger_geometry::GeometryInstance = rtree.nearest_neighbor(&[2.5, 1.0]).unwrap();
    assert_eq!(nearest.instance_idx, Some(0));
    assert_approx_eq!(f32, nearest.distance_2(&[2.5, 1.0]), 0.5);

    // Check bounding box
    let bbox = nearest.geometry.bounding_rect().unwrap();
    assert!((bbox.width() - 2.0).abs() < 0.1); // Size 2.0, area 4.0
}

#[test]
fn test_symbol_rtree_multiple() {
    // Create a symbol mark with two symbols at different locations
    let mut mark = SceneSymbolMark::default();
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

    let rtree = MarkRTree::new(mark.geometry_iter(0).collect());

    // Test nearest to first symbol
    let nearest = rtree.nearest_neighbor(&[0.2, 0.2]).unwrap();
    assert_eq!(nearest.instance_idx, Some(0));
    assert_approx_eq!(f32, nearest.distance_2(&[0.2, 0.2]), 0.0);

    // Test nearest to second symbol
    let nearest = rtree.nearest_neighbor(&[3.0, 0.0]).unwrap();
    assert_eq!(nearest.instance_idx, Some(1));
    assert_approx_eq!(f32, nearest.distance_2(&[3.0, 0.0]), 0.0);

    // Test that we get both symbols in order of distance
    let nearest_two: Vec<_> = rtree.nearest_neighbor_iter(&[1.5, 0.0]).take(2).collect();
    assert_eq!(nearest_two.len(), 2);
    assert_eq!(nearest_two[0].instance_idx, Some(0));
    assert_eq!(nearest_two[1].instance_idx, Some(1));
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

    let rtree = MarkRTree::new(mark.geometry_iter(0).collect());
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

    let rtree = MarkRTree::new(mark.geometry_iter(0).collect());

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

    let rtree = MarkRTree::new(mark.geometry_iter(0).collect());

    // Query a box that should contain the middle circle
    let query_box = AABB::from_corners([1.0, -1.0], [3.0, 1.0]);
    let results: Vec<_> = rtree.locate_in_envelope(&query_box).collect();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].instance_idx, Some(1)); // Middle circle
}
