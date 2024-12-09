use crate::marks::GeometryIter;
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_scenegraph::marks::mark::SceneMark;
use geo::{BoundingRect, Distance, Euclidean};
use geo_types::Geometry;
use rstar::{
    iterators::{
        IntersectionIterator, LocateAllAtPoint, LocateInEnvelope, LocateInEnvelopeIntersecting,
        LocateWithinDistanceIterator, NearestNeighborDistance2Iterator, NearestNeighborIterator,
        RTreeIterator,
    },
    Envelope, PointDistance, RTree, RTreeObject, AABB,
};

/// A geometry with an associated instance ID for storage in the R-tree
#[derive(Debug, Clone)]
pub struct GeometryInstance {
    pub mark_index: usize,
    pub instance_index: Option<usize>,
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
pub struct MarkRTree {
    rtree: RTree<GeometryInstance>,
    envelope: AABB<[f32; 2]>,
}

impl MarkRTree {
    pub fn new(geometries: Vec<GeometryInstance>) -> Self {
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

        Self { rtree, envelope }
    }

    pub fn from_scene_group(group: &SceneGroup) -> MarkRTree {
        let mut geometry_instances: Vec<GeometryInstance> = vec![];
        for (mark_index, mark) in group.marks.iter().enumerate() {
            match mark {
                SceneMark::Arc(mark) => {
                    geometry_instances.extend(mark.geometry_iter(mark_index));
                }
                SceneMark::Area(mark) => {
                    geometry_instances.extend(mark.geometry_iter(mark_index));
                }
                SceneMark::Path(mark) => {
                    geometry_instances.extend(mark.geometry_iter(mark_index));
                }
                SceneMark::Symbol(mark) => {
                    geometry_instances.extend(mark.geometry_iter(mark_index));
                }
                SceneMark::Line(mark) => {
                    geometry_instances.extend(mark.geometry_iter(mark_index));
                }
                SceneMark::Trail(mark) => {
                    geometry_instances.extend(mark.geometry_iter(mark_index));
                }
                SceneMark::Rect(mark) => {
                    geometry_instances.extend(mark.geometry_iter(mark_index));
                }
                SceneMark::Rule(mark) => {
                    geometry_instances.extend(mark.geometry_iter(mark_index));
                }
                SceneMark::Text(mark) => {
                    geometry_instances.extend(mark.geometry_iter(mark_index));
                }
                SceneMark::Image(mark) => {
                    geometry_instances.extend(mark.geometry_iter(mark_index));
                }
                SceneMark::Group(_scene_group) => {
                    // Consider whether to recurse into group marks
                }
            }
        }
        MarkRTree::new(geometry_instances)
    }

    /// Returns the envelope of the entire tree
    pub fn envelope(&self) -> &AABB<[f32; 2]> {
        &self.envelope
    }

    /// Returns the number of objects in the r-tree
    pub fn size(&self) -> usize {
        self.rtree.size()
    }

    /// Returns an iterator over all elements contained in the tree
    pub fn iter(&self) -> RTreeIterator<GeometryInstance> {
        self.rtree.iter()
    }

    /// Returns a single top-most mark instance at a given point.
    ///
    /// If multiple marks or mark instances contain the given point, the top-most one is returned.
    pub fn pick_top_mark_at_point(&self, point: &[f32; 2]) -> Option<&GeometryInstance> {
        let mut candidate_instance: Option<&GeometryInstance> = None;
        for next_instance in self.rtree.locate_all_at_point(point) {
            if let Some(inner_candidate_instance) = candidate_instance {
                if next_instance.mark_index == inner_candidate_instance.mark_index {
                    if next_instance.z_index > inner_candidate_instance.z_index {
                        // Same mark as current candidate, but higher z-index, so keep it.
                        candidate_instance = Some(next_instance);
                    }
                } else if next_instance.mark_index > inner_candidate_instance.mark_index {
                    // Mark is above the current candidate's mark, so keep it.
                    candidate_instance = Some(next_instance);
                }
            } else {
                candidate_instance = Some(next_instance);
            }
        }
        candidate_instance
    }

    /// Returns all elements contained in an envelope

    /// Returns a single object that covers a given point.
    ///
    /// If multiple elements contain the given point, any of them is returned.
    pub fn locate_at_point(&self, point: &[f32; 2]) -> Option<&GeometryInstance> {
        self.rtree.locate_at_point(point)
    }

    /// Returns a mutable reference to the object that covers a given point.
    ///
    /// If multiple elements contain the given point, any of them is returned.
    pub fn locate_all_at_point(&self, point: &[f32; 2]) -> LocateAllAtPoint<GeometryInstance> {
        self.rtree.locate_all_at_point(point)
    }

    /// Returns all elements contained in an envelope
    pub fn locate_in_envelope(
        &self,
        envelope: &AABB<[f32; 2]>,
    ) -> LocateInEnvelope<GeometryInstance> {
        self.rtree.locate_in_envelope(envelope)
    }

    /// Returns all elements whose envelope intersects a given envelope
    pub fn locate_in_envelope_intersecting(
        &self,
        envelope: &AABB<[f32; 2]>,
    ) -> LocateInEnvelopeIntersecting<GeometryInstance> {
        self.rtree.locate_in_envelope_intersecting(envelope)
    }

    /// Returns the nearest neighbor for a given point
    pub fn nearest_neighbor(&self, query_point: &[f32; 2]) -> Option<&GeometryInstance> {
        self.rtree.nearest_neighbor(query_point)
    }

    /// Returns all elements of the tree sorted by their distance to a given point
    pub fn nearest_neighbor_iter(
        &self,
        query_point: &[f32; 2],
    ) -> NearestNeighborIterator<GeometryInstance> {
        self.rtree.nearest_neighbor_iter(query_point)
    }

    /// Returns all elements of the tree within a certain distance
    pub fn locate_within_distance(
        &self,
        query_point: [f32; 2],
        max_squared_radius: f32,
    ) -> LocateWithinDistanceIterator<GeometryInstance> {
        self.rtree
            .locate_within_distance(query_point, max_squared_radius)
    }

    /// Returns all elements of the tree sorted by their distance, along with their distances
    pub fn nearest_neighbor_iter_with_distance_2(
        &self,
        query_point: &[f32; 2],
    ) -> NearestNeighborDistance2Iterator<GeometryInstance> {
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
        other: &'a MarkRTree,
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
}
