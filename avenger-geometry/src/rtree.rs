use geo::{BoundingRect, Distance, Euclidean};
use geo_types::Geometry;
use rstar::{
    iterators::{
        IntersectionIterator, LocateInEnvelope, LocateInEnvelopeIntersecting,
        LocateWithinDistanceIterator, NearestNeighborDistance2Iterator, NearestNeighborIterator,
        RTreeIterator,
    },
    Envelope, PointDistance, RTree, RTreeObject, AABB,
};

/// A geometry with an associated instance ID for storage in the R-tree
#[derive(Debug, Clone)]
pub struct GeometryInstance {
    pub id: usize,
    pub geometry: Geometry<f32>,
    pub half_stroke_width: f32,
}

impl RTreeObject for GeometryInstance {
    type Envelope = AABB<[f32; 2]>;

    /// Returns the envelope of the geometry, including the stroke width
    fn envelope(&self) -> Self::Envelope {
        let bbox = self.geometry.bounding_rect().unwrap();
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
        // Compute the envelope of the geometries
        let envelope = geometries
            .iter()
            .map(|g| g.envelope())
            .reduce(|a, b| a.merged(&b))
            .unwrap();

        // Bulk load the geometries into an R-tree
        let rtree = RTree::bulk_load(geometries);

        Self { rtree, envelope }
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

    /// Returns a single object that covers a given point.
    ///
    /// If multiple elements contain the given point, any of them is returned.
    pub fn locate_at_point(&self, point: &[f32; 2]) -> Option<&GeometryInstance> {
        self.rtree.locate_at_point(point)
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
