//! The instances that brushes select, found in the trees of the scenes below them.

use std::sync::Arc;

use avenger_geometry::data::Datum;
use avenger_geometry::query::{Extent, Query, QueryShape, Relation};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_layout::Rect;
use avenger_scenegraph::marks::mark::MarkInstance;
use geo::Centroid;

type Id<I> = Arc<dyn Fn(&Datum) -> Option<I> + Send + Sync>;

/// The instances that a brush selects, from the app state when the brush starts a gesture.
pub(crate) type InstancesOf<State, I> = Box<dyn Fn(&State) -> Instances<I> + Send + Sync>;

/// Instances that don't depend on the app state.
pub(crate) fn fixed<State, I: Send + Sync + 'static>(
    instances: Instances<I>,
) -> InstancesOf<State, I> {
    Box::new(move |_| instances.clone())
}

/// A target tree that is stale, as after a pan or a zoom that hasn't settled, so its hits don't
/// show which instances the scene draws.
pub(crate) struct Stale;

/// The instances that a brush selects: the marks that it searches in the scenes below the
/// brushes, and the id of each instance's row, such as an ID column's value.
///
/// Identify the instances by a column, not by row index. A row index counts rows in the batch
/// that the app attached to the mark for one build, and an app that attaches filtered batches
/// gives different marks' rows the same indices.
pub struct Instances<I> {
    marks: Vec<String>,
    id: Id<I>,
}

impl<I> Clone for Instances<I> {
    fn clone(&self) -> Self {
        Self {
            marks: self.marks.clone(),
            id: self.id.clone(),
        }
    }
}

impl<I> Instances<I> {
    /// The instances of the marks named `marks`, with rows attached to the scene. Set their
    /// ids with [`id`](Self::id).
    pub fn marks(marks: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            marks: marks.into_iter().map(Into::into).collect(),
            id: Arc::new(|_| None),
        }
    }

    /// Read each instance's id from its row. An instance without a id can't be selected.
    pub fn id(mut self, id: impl Fn(&Datum) -> Option<I> + Send + Sync + 'static) -> Self {
        self.id = Arc::new(id);
        self
    }

    /// Whether `tree` has rows for one of the marks.
    fn targets(&self, tree: &SceneGraphRTree) -> bool {
        (self.marks.iter()).any(|mark| tree.data().rows(mark).is_some())
    }

    /// Whether `mark` is an instance of one of the marks with a row in `tree`.
    fn keeps(&self, tree: &SceneGraphRTree, mark: &MarkInstance) -> bool {
        mark.instance_index.is_some()
            && self.marks.contains(&mark.name)
            && tree.data().rows(&mark.name).is_some()
    }

    /// The id of the topmost instance at `at` in `trees`, ordered from bottom to top, unless a
    /// target tree above the instance is stale.
    pub(crate) fn at(&self, trees: &[SceneGraphRTree], at: [f32; 2]) -> Result<Option<I>, Stale> {
        for tree in trees.iter().rev() {
            if !self.targets(tree) {
                continue;
            }
            if !tree.is_current() {
                return Err(Stale);
            }
            let topmost = Query::new(QueryShape::Point(at), Relation::Intersects)
                .filter(|mark| self.keeps(tree, mark))
                .limit(1);
            if let Some(instance) = tree.query(&topmost).first() {
                let datum = tree.datum(&instance.mark_instance);
                return Ok(datum.and_then(|datum| (self.id)(&datum)));
            }
        }
        Ok(None)
    }

    /// The ids and centers of the instances whose centers lie in `area`, in `trees` ordered from
    /// bottom to top, unless a target tree is stale. The instances that a clip hides are left
    /// out.
    pub(crate) fn centers(
        &self,
        trees: &[SceneGraphRTree],
        area: Rect,
    ) -> Result<Vec<(I, [f32; 2])>, Stale> {
        let mut centers = Vec::new();
        for tree in trees.iter().rev() {
            if !self.targets(tree) {
                continue;
            }
            if !tree.is_current() {
                return Err(Stale);
            }
            let shape = QueryShape::Rect {
                min: [area.x, area.y],
                max: [area.right(), area.bottom()],
            };
            let inside = Query::new(shape, Relation::Intersects)
                .extent(Extent::Centroid)
                .filter(|mark| self.keeps(tree, mark));
            for instance in tree.query(&inside) {
                let Some(center) = instance.geometry.centroid() else {
                    continue;
                };
                let datum = tree.datum(&instance.mark_instance);
                if let Some(id) = datum.and_then(|datum| (self.id)(&datum)) {
                    centers.push((id, [center.x(), center.y()]));
                }
            }
        }
        Ok(centers)
    }
}
