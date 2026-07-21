use std::collections::HashMap;

use avenger_scenegraph::{marks::mark::MarkInstance, scene_graph::SceneGraph};
use geo::{BoundingRect, Contains, Distance, Euclidean, Intersects};
use geo_svg::{Color, CombineToSVG};
use geo_types::Geometry;
use rstar::{
    iterators::{
        IntersectionIterator, LocateAllAtPoint, LocateInEnvelope, LocateInEnvelopeIntersecting,
        LocateWithinDistanceIterator, NearestNeighborDistance2Iterator, NearestNeighborIterator,
        RTreeIterator,
    },
    Envelope, PointDistance, RTree, RTreeObject, AABB,
};

use crate::marks::MarkGeometryUtils;

#[derive(Debug, Clone)]
pub enum GeometryQueryShape {
    Rect { x0: f32, y0: f32, x1: f32, y1: f32 },
    Circle { cx: f32, cy: f32, radius: f32 },
    Polygon { points: Vec<[f32; 2]> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometryQueryHitPolicy {
    EnvelopeIntersects,
    GeometryIntersects,
    GeometryContained,
    AnchorInside,
    CentroidInside,
}

/// A geometry with an associated instance ID for storage in the R-tree
#[derive(Debug, Clone)]
pub struct GeometryInstance {
    pub mark_instance: MarkInstance,
    pub interactive: bool,
    pub z_index: usize,
    pub geometry: Geometry<f32>,
    pub half_stroke_width: f32,
}

impl RTreeObject for GeometryInstance {
    type Envelope = AABB<[f32; 2]>;

    /// Returns the envelope of the geometry, including the stroke width
    fn envelope(&self) -> Self::Envelope {
        if let Some(bbox) = self.geometry.bounding_rect() {
            AABB::from_corners(
                [
                    bbox.min().x - self.half_stroke_width,
                    bbox.min().y - self.half_stroke_width,
                ],
                [
                    bbox.max().x + self.half_stroke_width,
                    bbox.max().y + self.half_stroke_width,
                ],
            )
        } else {
            println!("No bounding box for geometry: {:?}", self.geometry);
            AABB::from_corners([0.0, 0.0], [0.0, 0.0])
        }
    }
}

impl PointDistance for GeometryInstance {
    fn distance_2(&self, point: &[f32; 2]) -> f32 {
        // Compute the distance from the point to the geometry, then subtract the stroke half-width
        let point = geo_types::Point::new(point[0], point[1]);
        (Euclidean::distance(&self.geometry, &point) - self.half_stroke_width).max(0.0)
    }

    fn contains_point(&self, point: &[f32; 2]) -> bool {
        let point = geo_types::Point::new(point[0], point[1]);
        Euclidean::distance(&self.geometry, &point) <= self.half_stroke_width
    }
}

#[derive(Debug, Clone)]
pub struct SceneGraphRTree {
    /// The R-tree containing the geometries, relative to the scene graph origin
    rtree: RTree<GeometryInstance>,
    /// The envelope of the scene graph, relative to the scene graph origin
    envelope: AABB<[f32; 2]>,
    /// Absolute origin of each group
    group_origins: HashMap<Vec<usize>, [f32; 2]>,
    /// Names of each named group
    group_names: HashMap<String, Vec<usize>>,
    /// Group names keyed by path, retaining repeated names in separate branches.
    group_names_by_path: HashMap<Vec<usize>, String>,
}

impl SceneGraphRTree {
    fn new(
        geometries: Vec<GeometryInstance>,
        group_origins: HashMap<Vec<usize>, [f32; 2]>,
        group_names: HashMap<String, Vec<usize>>,
        group_names_by_path: HashMap<Vec<usize>, String>,
    ) -> Self {
        let envelope = if geometries.is_empty() {
            AABB::from_corners([0.0, 0.0], [0.0, 0.0])
        } else {
            geometries
                .iter()
                .map(|g| g.envelope())
                .reduce(|a, b| a.merged(&b))
                .unwrap()
        };

        // Bulk load the geometries into an R-tree
        let rtree = RTree::bulk_load(geometries);

        Self {
            rtree,
            envelope,
            group_origins,
            group_names,
            group_names_by_path,
        }
    }

    pub fn from_scene_graph(scene_graph: &SceneGraph) -> SceneGraphRTree {
        let mut geometry_instances: Vec<GeometryInstance> = vec![];

        for (group_index, group) in scene_graph.marks.iter().enumerate() {
            let mark_path = vec![group_index];
            let origin = [scene_graph.origin[0], scene_graph.origin[1]];
            geometry_instances.extend(
                group
                    .geometry_iter(mark_path, origin)
                    .filter(|instance| instance.interactive),
            );
        }

        let group_names_by_path = scene_graph
            .group_paths()
            .into_iter()
            .filter_map(|path| {
                let avenger_scenegraph::marks::mark::SceneMark::Group(group) =
                    scene_graph.get_mark(&path)?
                else {
                    return None;
                };
                Some((path, group.name.clone()))
            })
            .collect();

        SceneGraphRTree::new(
            geometry_instances,
            scene_graph.group_origins(),
            scene_graph.group_names(),
            group_names_by_path,
        )
    }

    /// Whether a picked scene mark belongs to a stable public target.
    ///
    /// Ordinary chart marks carry their public target as `name`. Widget marks
    /// retain the public scene topology of part-named children inside
    /// widget-named groups, so their owner segments are recovered from the
    /// ancestor group path.
    pub fn mark_target_matches(&self, instance: &MarkInstance, target: &str) -> bool {
        if instance.name == target
            || instance
                .name
                .strip_prefix(target)
                .is_some_and(|suffix| suffix.starts_with('.'))
        {
            return true;
        }

        let segments = target.split('.').collect::<Vec<_>>();
        let mut ancestors = self
            .group_names_by_path
            .iter()
            .filter(|(path, _)| {
                path.len() < instance.mark_path.len() && instance.mark_path.starts_with(path)
            })
            .collect::<Vec<_>>();
        ancestors.sort_by_key(|(path, _)| path.len());
        let ancestor_names = ancestors
            .into_iter()
            .map(|(_, name)| name.as_str())
            .collect::<Vec<_>>();
        if !segments.is_empty() && ancestor_names.ends_with(&segments) {
            return true;
        }

        let Some((part, owners)) = segments.split_last() else {
            return false;
        };
        if instance.name != *part || owners.is_empty() {
            return false;
        }
        ancestor_names.ends_with(owners)
    }

    /// Returns the envelope of the entire tree
    pub fn envelope(&self) -> &AABB<[f32; 2]> {
        &self.envelope
    }

    /// Returns the absolute origin of a group
    pub fn group_origin(&self, path: &[usize]) -> Option<[f32; 2]> {
        self.group_origins.get(path).cloned()
    }

    /// Returns the absolute origin of a named group
    pub fn named_group_origin(&self, name: &str) -> Option<[f32; 2]> {
        self.group_names
            .get(name)
            .and_then(|path| self.group_origins.get(path))
            .cloned()
    }

    /// Returns the number of objects in the r-tree
    pub fn size(&self) -> usize {
        self.rtree.size()
    }

    /// Returns an iterator over all elements contained in the tree
    pub fn iter(&self) -> RTreeIterator<'_, GeometryInstance> {
        self.rtree.iter()
    }

    /// Returns a single top-most mark instance at a given point.
    ///
    /// If multiple marks or mark instances contain the given point, the top-most one is returned.
    pub fn pick_top_mark_at_point(&self, point: &[f32; 2]) -> Option<&MarkInstance> {
        let mut candidate_instance: Option<&GeometryInstance> = None;
        for next_instance in self.rtree.locate_all_at_point(point) {
            if let Some(inner_candidate_instance) = candidate_instance {
                if next_instance.mark_instance.mark_path
                    == inner_candidate_instance.mark_instance.mark_path
                {
                    if next_instance.z_index > inner_candidate_instance.z_index {
                        // Same mark as current candidate, but higher z-index, so keep it.
                        candidate_instance = Some(next_instance);
                    }
                } else if next_instance.mark_instance.mark_path
                    > inner_candidate_instance.mark_instance.mark_path
                {
                    // Mark is above the current candidate's mark, so keep it.
                    candidate_instance = Some(next_instance);
                }
            } else {
                candidate_instance = Some(next_instance);
            }
        }
        candidate_instance.map(|g| &g.mark_instance)
    }

    /// Returns a single object that covers a given point.
    ///
    /// If multiple elements contain the given point, any of them is returned.
    pub fn locate_at_point(&self, point: &[f32; 2]) -> Option<&GeometryInstance> {
        self.rtree.locate_at_point(point)
    }

    /// Returns a mutable reference to the object that covers a given point.
    ///
    /// If multiple elements contain the given point, any of them is returned.
    pub fn locate_all_at_point(&self, point: &[f32; 2]) -> LocateAllAtPoint<'_, GeometryInstance> {
        self.rtree.locate_all_at_point(point)
    }

    /// Returns all elements contained in an envelope
    pub fn locate_in_envelope(
        &self,
        envelope: &AABB<[f32; 2]>,
    ) -> LocateInEnvelope<'_, GeometryInstance> {
        self.rtree.locate_in_envelope(envelope)
    }

    /// Returns all elements whose envelope intersects a given envelope
    pub fn locate_in_envelope_intersecting(
        &self,
        envelope: &AABB<[f32; 2]>,
    ) -> LocateInEnvelopeIntersecting<'_, GeometryInstance> {
        self.rtree.locate_in_envelope_intersecting(envelope)
    }

    /// Returns rendered geometry instances matching a scene-space query shape.
    ///
    /// The R-tree is used for coarse envelope filtering and the selected hit
    /// policy is applied to the candidate geometries. Results are sorted by
    /// rendered mark path and instance index for deterministic event handling.
    pub fn query_shape(
        &self,
        shape: &GeometryQueryShape,
        hit_policy: GeometryQueryHitPolicy,
    ) -> Vec<&GeometryInstance> {
        let Some(envelope) = query_shape_envelope(shape) else {
            return Vec::new();
        };
        let mut out: Vec<&GeometryInstance> = self
            .rtree
            .locate_in_envelope_intersecting(&envelope)
            .filter(|instance| query_shape_hits(shape, hit_policy, instance))
            .collect();
        out.sort_by(|a, b| {
            a.mark_instance
                .mark_path
                .cmp(&b.mark_instance.mark_path)
                .then_with(|| {
                    a.mark_instance
                        .instance_index
                        .cmp(&b.mark_instance.instance_index)
                })
        });
        out
    }

    /// Returns the nearest neighbor for a given point
    pub fn nearest_neighbor(&self, query_point: &[f32; 2]) -> Option<&GeometryInstance> {
        self.rtree.nearest_neighbor(query_point)
    }

    /// Returns all elements of the tree sorted by their distance to a given point
    pub fn nearest_neighbor_iter(
        &self,
        query_point: &[f32; 2],
    ) -> NearestNeighborIterator<'_, GeometryInstance> {
        self.rtree.nearest_neighbor_iter(query_point)
    }

    /// Returns all elements of the tree within a certain distance
    pub fn locate_within_distance(
        &self,
        query_point: [f32; 2],
        max_squared_radius: f32,
    ) -> LocateWithinDistanceIterator<'_, GeometryInstance> {
        self.rtree
            .locate_within_distance(query_point, max_squared_radius)
    }

    /// Returns all elements of the tree sorted by their distance, along with their distances
    pub fn nearest_neighbor_iter_with_distance_2(
        &self,
        query_point: &[f32; 2],
    ) -> NearestNeighborDistance2Iterator<'_, GeometryInstance> {
        self.rtree
            .nearest_neighbor_iter_with_distance_2(query_point)
    }

    /// Returns all nearest neighbors that have exactly the same distance
    pub fn nearest_neighbors(&self, query_point: &[f32; 2]) -> Vec<&GeometryInstance> {
        self.rtree.nearest_neighbors(query_point)
    }

    /// Returns all possible intersecting objects between this and another tree
    pub fn intersection_candidates_with_other_tree<'a>(
        &'a self,
        other: &'a SceneGraphRTree,
    ) -> IntersectionIterator<'a, GeometryInstance, GeometryInstance> {
        self.rtree
            .intersection_candidates_with_other_tree(&other.rtree)
    }

    /// Insert a new geometry instance into the tree
    pub fn insert(&mut self, geometry: GeometryInstance) {
        // Update the envelope to include the new geometry
        let geom_envelope = geometry.envelope();
        self.envelope = self.envelope.merged(&geom_envelope);

        // Insert into rtree
        self.rtree.insert(geometry);
    }

    /// Insert multiple geometry instances into an existing tree
    pub fn insert_all(&mut self, geometries: Vec<GeometryInstance>) {
        for geometry in geometries {
            self.insert(geometry);
        }
    }

    pub fn to_svg(&self) -> String {
        self.iter()
            .map(|g| g.geometry.clone())
            .collect::<Vec<_>>()
            .combine_to_svg()
            .unwrap()
            .with_stroke_color(Color::Named("crimson"))
            .with_stroke_opacity(0.5)
            .with_stroke_width(0.5)
            .with_fill_color(Color::Named("blue"))
            .with_fill_opacity(0.2)
            .to_string()
    }
}

fn query_shape_envelope(shape: &GeometryQueryShape) -> Option<AABB<[f32; 2]>> {
    match shape {
        GeometryQueryShape::Rect { x0, y0, x1, y1 } => Some(AABB::from_corners(
            [x0.min(*x1), y0.min(*y1)],
            [x0.max(*x1), y0.max(*y1)],
        )),
        GeometryQueryShape::Circle { cx, cy, radius } => {
            let radius = radius.abs();
            Some(AABB::from_corners(
                [cx - radius, cy - radius],
                [cx + radius, cy + radius],
            ))
        }
        GeometryQueryShape::Polygon { points } => {
            let mut iter = points.iter();
            let first = iter.next()?;
            let mut min_x = first[0];
            let mut max_x = first[0];
            let mut min_y = first[1];
            let mut max_y = first[1];
            for point in iter {
                min_x = min_x.min(point[0]);
                max_x = max_x.max(point[0]);
                min_y = min_y.min(point[1]);
                max_y = max_y.max(point[1]);
            }
            Some(AABB::from_corners([min_x, min_y], [max_x, max_y]))
        }
    }
}

fn query_shape_hits(
    shape: &GeometryQueryShape,
    hit_policy: GeometryQueryHitPolicy,
    instance: &GeometryInstance,
) -> bool {
    match hit_policy {
        GeometryQueryHitPolicy::EnvelopeIntersects => {
            let Some(shape_envelope) = query_shape_envelope(shape) else {
                return false;
            };
            shape_envelope.intersects(&instance.envelope())
        }
        GeometryQueryHitPolicy::AnchorInside => {
            query_shape_contains_point(shape, geometry_instance_anchor(instance))
        }
        GeometryQueryHitPolicy::CentroidInside => {
            query_shape_contains_point(shape, geometry_instance_anchor(instance))
        }
        GeometryQueryHitPolicy::GeometryIntersects => {
            query_shape_intersects_geometry(shape, instance)
        }
        GeometryQueryHitPolicy::GeometryContained => query_shape_contains_geometry(shape, instance),
    }
}

fn geometry_instance_anchor(instance: &GeometryInstance) -> [f32; 2] {
    let envelope = instance.envelope();
    let lower = envelope.lower();
    let upper = envelope.upper();
    [(lower[0] + upper[0]) * 0.5, (lower[1] + upper[1]) * 0.5]
}

fn query_shape_contains_point(shape: &GeometryQueryShape, point: [f32; 2]) -> bool {
    match shape {
        GeometryQueryShape::Rect { x0, y0, x1, y1 } => {
            point[0] >= x0.min(*x1)
                && point[0] <= x0.max(*x1)
                && point[1] >= y0.min(*y1)
                && point[1] <= y0.max(*y1)
        }
        GeometryQueryShape::Circle { cx, cy, radius } => {
            let dx = point[0] - cx;
            let dy = point[1] - cy;
            dx * dx + dy * dy <= radius.abs() * radius.abs()
        }
        GeometryQueryShape::Polygon { points } => {
            let Some(polygon) = polygon_from_points(points) else {
                return false;
            };
            polygon.contains(&geo_types::Point::new(point[0], point[1]))
        }
    }
}

fn query_shape_intersects_geometry(
    shape: &GeometryQueryShape,
    instance: &GeometryInstance,
) -> bool {
    match shape {
        GeometryQueryShape::Rect { .. } | GeometryQueryShape::Polygon { .. } => {
            let Some(query_geometry) = query_shape_geometry(shape) else {
                return false;
            };
            query_geometry.intersects(&instance.geometry)
        }
        GeometryQueryShape::Circle { cx, cy, radius } => {
            let point = geo_types::Point::new(*cx, *cy);
            Euclidean::distance(&instance.geometry, &point)
                <= radius.abs() + instance.half_stroke_width
        }
    }
}

fn query_shape_contains_geometry(shape: &GeometryQueryShape, instance: &GeometryInstance) -> bool {
    match shape {
        GeometryQueryShape::Rect { .. } | GeometryQueryShape::Polygon { .. } => {
            let Some(query_geometry) = query_shape_geometry(shape) else {
                return false;
            };
            query_geometry.contains(&instance.geometry)
        }
        GeometryQueryShape::Circle { cx, cy, radius } => {
            let Some(bbox) = instance.geometry.bounding_rect() else {
                return false;
            };
            let radius = radius.abs();
            [
                [bbox.min().x, bbox.min().y],
                [bbox.min().x, bbox.max().y],
                [bbox.max().x, bbox.min().y],
                [bbox.max().x, bbox.max().y],
            ]
            .into_iter()
            .all(|point| {
                let dx = point[0] - cx;
                let dy = point[1] - cy;
                dx * dx + dy * dy <= radius * radius
            })
        }
    }
}

fn query_shape_geometry(shape: &GeometryQueryShape) -> Option<Geometry<f32>> {
    match shape {
        GeometryQueryShape::Rect { x0, y0, x1, y1 } => {
            let rect = geo_types::Rect::new(
                geo_types::Coord {
                    x: x0.min(*x1),
                    y: y0.min(*y1),
                },
                geo_types::Coord {
                    x: x0.max(*x1),
                    y: y0.max(*y1),
                },
            );
            Some(Geometry::Rect(rect))
        }
        GeometryQueryShape::Circle { .. } => None,
        GeometryQueryShape::Polygon { points } => {
            polygon_from_points(points).map(Geometry::Polygon)
        }
    }
}

fn polygon_from_points(points: &[[f32; 2]]) -> Option<geo_types::Polygon<f32>> {
    if points.len() < 3 {
        return None;
    }
    let mut coords = points
        .iter()
        .map(|point| geo_types::Coord {
            x: point[0],
            y: point[1],
        })
        .collect::<Vec<_>>();
    if coords.first() != coords.last() {
        coords.push(*coords.first()?);
    }
    Some(geo_types::Polygon::new(coords.into(), Vec::new()))
}

pub trait EnvelopeUtils {
    fn height(&self) -> f32;
    fn width(&self) -> f32;
}

impl EnvelopeUtils for AABB<[f32; 2]> {
    fn height(&self) -> f32 {
        self.upper()[1] - self.lower()[1]
    }

    fn width(&self) -> f32 {
        self.upper()[0] - self.lower()[0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_scenegraph::{
        marks::{mark::SceneMark, rect::SceneRectMark},
        scene_graph::SceneGraph,
    };
    use geo_types::{Point, Rect};

    fn point_instance(
        mark_path: Vec<usize>,
        instance_index: usize,
        x: f32,
        y: f32,
    ) -> GeometryInstance {
        GeometryInstance {
            mark_instance: MarkInstance {
                name: "points".to_string(),
                mark_path,
                instance_index: Some(instance_index),
            },
            interactive: true,
            z_index: 0,
            geometry: Geometry::Point(Point::new(x, y)),
            half_stroke_width: 0.0,
        }
    }

    fn rect_instance(
        mark_path: Vec<usize>,
        instance_index: usize,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
    ) -> GeometryInstance {
        GeometryInstance {
            mark_instance: MarkInstance {
                name: "rects".to_string(),
                mark_path,
                instance_index: Some(instance_index),
            },
            interactive: true,
            z_index: 0,
            geometry: Geometry::Rect(Rect::new(
                geo_types::Coord { x: x0, y: y0 },
                geo_types::Coord { x: x1, y: y1 },
            )),
            half_stroke_width: 0.0,
        }
    }

    fn test_tree(geometries: Vec<GeometryInstance>) -> SceneGraphRTree {
        SceneGraphRTree::new(geometries, HashMap::new(), HashMap::new(), HashMap::new())
    }

    #[test]
    fn public_widget_targets_include_ancestor_group_identity() {
        let tree = SceneGraphRTree::new(
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([
                (vec![0], "filters".to_string()),
                (vec![0, 2], "regions".to_string()),
                (vec![1], "unrelated".to_string()),
            ]),
        );
        let row = MarkInstance {
            name: "row".to_string(),
            mark_path: vec![0, 2, 4],
            instance_index: Some(0),
        };
        assert!(tree.mark_target_matches(&row, "filters.regions.row"));
        assert!(tree.mark_target_matches(&row, "filters.regions"));
        assert!(tree.mark_target_matches(&row, "regions"));
        assert!(!tree.mark_target_matches(&row, "unrelated.regions.row"));
        assert!(!tree.mark_target_matches(&row, "unrelated.regions"));
    }

    #[test]
    fn noninteractive_marks_are_excluded_from_scene_graph_rtree_but_not_bounds() {
        let rect = SceneRectMark {
            interactive: false,
            x: 10.0.into(),
            y: 20.0.into(),
            width: Some(30.0.into()),
            height: Some(40.0.into()),
            ..Default::default()
        };
        let scene_mark = SceneMark::Rect(rect);
        let bounds = scene_mark.bounding_box();
        assert_eq!(bounds.lower(), [10.0, 20.0]);
        assert_eq!(bounds.upper(), [40.0, 60.0]);

        let scene = SceneGraph {
            marks: vec![scene_mark],
            width: 100.0,
            height: 100.0,
            origin: [0.0, 0.0],
        };
        let rtree = SceneGraphRTree::from_scene_graph(&scene);
        assert_eq!(rtree.size(), 0);
        assert!(rtree.pick_top_mark_at_point(&[20.0, 30.0]).is_none());
    }

    #[test]
    fn rect_query_uses_anchor_policy_and_stable_sort() {
        let tree = test_tree(vec![
            point_instance(vec![1], 2, 12.0, 12.0),
            point_instance(vec![0], 1, 10.0, 10.0),
            point_instance(vec![0], 0, 5.0, 5.0),
            point_instance(vec![0], 3, 30.0, 30.0),
        ]);

        let matches = tree.query_shape(
            &GeometryQueryShape::Rect {
                x0: 15.0,
                y0: 15.0,
                x1: 0.0,
                y1: 0.0,
            },
            GeometryQueryHitPolicy::AnchorInside,
        );

        let ids = matches
            .iter()
            .map(|instance| {
                (
                    instance.mark_instance.mark_path.clone(),
                    instance.mark_instance.instance_index,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![(vec![0], Some(0)), (vec![0], Some(1)), (vec![1], Some(2))]
        );
    }

    #[test]
    fn circle_query_matches_anchor_points() {
        let tree = test_tree(vec![
            point_instance(vec![0], 0, 8.0, 8.0),
            point_instance(vec![0], 1, 12.0, 10.0),
            point_instance(vec![0], 2, 18.0, 10.0),
        ]);

        let matches = tree.query_shape(
            &GeometryQueryShape::Circle {
                cx: 10.0,
                cy: 10.0,
                radius: 3.0,
            },
            GeometryQueryHitPolicy::AnchorInside,
        );

        let ids = matches
            .iter()
            .map(|instance| instance.mark_instance.instance_index)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![Some(0), Some(1)]);
    }

    #[test]
    fn polygon_query_matches_anchor_points() {
        let tree = test_tree(vec![
            point_instance(vec![0], 0, 5.0, 5.0),
            point_instance(vec![0], 1, 12.0, 8.0),
            point_instance(vec![0], 2, 15.0, 18.0),
        ]);

        let matches = tree.query_shape(
            &GeometryQueryShape::Polygon {
                points: vec![[0.0, 0.0], [20.0, 0.0], [10.0, 20.0]],
            },
            GeometryQueryHitPolicy::AnchorInside,
        );

        let ids = matches
            .iter()
            .map(|instance| instance.mark_instance.instance_index)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![Some(0), Some(1)]);
    }

    #[test]
    fn geometry_intersects_can_select_non_point_marks() {
        let tree = test_tree(vec![
            rect_instance(vec![0], 0, 0.0, 0.0, 5.0, 5.0),
            rect_instance(vec![0], 1, 20.0, 20.0, 30.0, 30.0),
        ]);

        let matches = tree.query_shape(
            &GeometryQueryShape::Rect {
                x0: 4.0,
                y0: 4.0,
                x1: 10.0,
                y1: 10.0,
            },
            GeometryQueryHitPolicy::GeometryIntersects,
        );

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].mark_instance.instance_index, Some(0));
    }
}
