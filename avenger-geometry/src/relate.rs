//! The tests behind every relation: a candidate's shape against a query's. Each is `geo`'s DE-9IM
//! predicate on the two shapes, with `Intersects` and `Disjoint` measured as a distance, so a pick
//! needn't build either shape.

use std::cell::OnceCell;

use geo::coordinate_position::CoordPos;
use geo::dimensions::Dimensions;
use geo::relate::IntersectionMatrix;
use geo::{Contains, Distance, Euclidean, PreparedGeometry, Relate};
use geo_types::Geometry;

use crate::query::Relation;
use crate::shape::{box_geometry, Shape};

type Prepared = PreparedGeometry<'static, Geometry<f32>, f32>;

/// A query's shape, prepared for relates on first use and at most once per query.
pub(crate) struct Target<'q> {
    pub(crate) shape: Shape<'q>,
    prepared: OnceCell<Option<Prepared>>,
}

impl<'q> Target<'q> {
    pub(crate) fn new(shape: Shape<'q>) -> Self {
        Self {
            shape,
            prepared: OnceCell::new(),
        }
    }

    /// The shape as one geometry, as polygons when it grows, prepared for relates.
    fn prepared(&self) -> Option<&Prepared> {
        let prepare = || self.shape.clone().into_geometry().map(Prepared::from);
        self.prepared.get_or_init(prepare).as_ref()
    }
}

/// Whether candidate shape `a` stands in `relation` to the query shape.
pub(crate) fn holds(relation: &Relation, a: &Shape, q: &Target) -> bool {
    let matrix = || matrix(a, q);
    match relation {
        Relation::Intersects => near(a, q),
        Relation::Disjoint => !near(a, q),
        Relation::CoveredBy => fits(a, &q.shape) && covered_by(a, q),
        Relation::Within => fits(a, &q.shape) && matrix().is_within(),
        Relation::Covers => fits(&q.shape, a) && matrix().is_covers(),
        Relation::Contains => fits(&q.shape, a) && matrix().is_contains(),
        Relation::ContainsProperly => fits(&q.shape, a) && matrix().is_contains_properly(),
        Relation::Equals => fits(a, &q.shape) && fits(&q.shape, a) && matrix().is_equal_topo(),
        Relation::Touches => matrix().is_touches(),
        Relation::Crosses => matrix().is_crosses(),
        Relation::Overlaps => matrix().is_overlaps(),
        Relation::InteriorsIntersect => {
            matrix().get(CoordPos::Inside, CoordPos::Inside) != Dimensions::Empty
        }
        Relation::Pattern(pattern) => pattern.matches(&matrix()),
    }
}

/// Whether `a` and the query shape share a point.
fn near(a: &Shape, q: &Target) -> bool {
    let (Some((ga, ra)), Some((gq, rq))) = (a.parts(), q.shape.parts()) else {
        return false;
    };
    Euclidean.distance(ga, gq) <= ra + rq
}

/// Whether `a` lies in the query shape.
fn covered_by(a: &Shape, q: &Target) -> bool {
    let (Some(envelope), Some(q)) = (a.envelope(), q.prepared()) else {
        return false;
    };
    // A shape whose box the query shape holds lies in it, which spares building the shape, such
    // as a drawn circle's polygon, for most of the instances that a lasso selects.
    if q.geometry().contains(&box_geometry(&envelope)) {
        return true;
    }
    (a.clone().into_geometry()).is_some_and(|a| a.relate(q).is_coveredby())
}

/// Whether `inner`'s box lies in `outer`'s, as it must for `outer` to hold `inner`.
fn fits(inner: &Shape, outer: &Shape) -> bool {
    let (Some(inner), Some(outer)) = (inner.envelope(), outer.envelope()) else {
        return false;
    };
    (0..2).all(|axis| {
        inner.lower()[axis] >= outer.lower()[axis] && inner.upper()[axis] <= outer.upper()[axis]
    })
}

/// The DE-9IM matrix of `a` against the query shape, with an empty shape as an empty geometry.
fn matrix(a: &Shape, q: &Target) -> IntersectionMatrix {
    let empty = Geometry::GeometryCollection(Default::default());
    let a = a.clone().into_geometry().unwrap_or_else(|| empty.clone());
    match q.prepared() {
        Some(q) => a.relate(q),
        None => a.relate(&empty),
    }
}
