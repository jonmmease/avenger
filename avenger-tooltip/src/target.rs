//! What a tooltip describes: the instance that a target finds at the pointer.

use avenger_geometry::query::{Metric, Nearest, Query, QueryShape, Relation};
use avenger_geometry::rtree::{GeometryInstance, SceneGraphRTree};
use avenger_scenegraph::marks::mark::MarkInstance;

use crate::content::TooltipField;

/// Where a tooltip stands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TooltipAnchor {
    /// Beside the instance's drawn edge, with a callout to it, so the tooltip stays still while
    /// the pointer moves over the instance.
    #[default]
    Data,
    /// Below and to the right of the pointer, so the pointer stays in view, for an instance
    /// without one point to point at, such as a region of a map.
    Pointer,
}

/// How a target finds its instance at the pointer.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Strategy {
    Exact,
    Nearest { max_radius: f32, metric: Metric },
}

/// An instance that a tooltip describes, with its row: how the target finds the instance at the
/// pointer and among which marks, where its tooltip stands, and which columns the tooltip shows.
/// Only an instance whose mark has rows is found, since a tooltip describes data.
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceTarget {
    strategy: Strategy,
    marks: Option<Vec<String>>,
    pub(crate) anchor: TooltipAnchor,
    pub(crate) header: Option<TooltipField>,
    pub(crate) fields: Option<Vec<TooltipField>>,
    pub(crate) swatch: bool,
}

impl InstanceTarget {
    /// The topmost instance under the pointer.
    pub fn exact() -> Self {
        Self::of(Strategy::Exact)
    }

    /// The instance nearest the pointer, within `max_radius` pixels of it in the plane.
    pub fn nearest(max_radius: f32) -> Self {
        Self::of(Strategy::Nearest {
            max_radius,
            metric: Metric::Euclidean,
        })
    }

    fn of(strategy: Strategy) -> Self {
        Self {
            strategy,
            marks: None,
            anchor: TooltipAnchor::default(),
            header: None,
            fields: None,
            swatch: true,
        }
    }

    /// The target, measuring a nearest instance's distance by `metric`, such as along x alone.
    /// An exact target, which measures no distance, ignores it.
    pub fn with_metric(mut self, metric: Metric) -> Self {
        if let Strategy::Nearest { metric: own, .. } = &mut self.strategy {
            *own = metric;
        }
        self
    }

    /// The target, among the instances of the marks named `marks` alone.
    pub fn with_marks(mut self, marks: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.marks = Some(marks.into_iter().map(Into::into).collect());
        self
    }

    pub fn with_anchor(mut self, anchor: TooltipAnchor) -> Self {
        self.anchor = anchor;
        self
    }

    /// The target, with `field`'s value as its tooltip's header: a bold first line, beside a
    /// swatch of the instance's color unless the target is
    /// [`without_swatch`](Self::without_swatch).
    pub fn with_header(mut self, field: impl Into<TooltipField>) -> Self {
        self.header = Some(field.into());
        self
    }

    /// The target, without the swatch of the instance's color beside its tooltip's header, for
    /// marks whose color says nothing about the instance.
    pub fn without_swatch(mut self) -> Self {
        self.swatch = false;
        self
    }

    /// The target, with a row for each of `fields`, in order. Without fields, a tooltip shows a
    /// row for each column of the instance's row.
    pub fn with_fields(
        mut self,
        fields: impl IntoIterator<Item = impl Into<TooltipField>>,
    ) -> Self {
        self.fields = Some(fields.into_iter().map(Into::into).collect());
        self
    }

    /// The instance that the target finds at `pointer` in `tree`.
    pub(crate) fn find<'t>(
        &self,
        tree: &'t SceneGraphRTree,
        pointer: [f32; 2],
    ) -> Option<&'t GeometryInstance> {
        let keep = |mark: &MarkInstance| {
            mark.instance_index.is_some()
                && tree.data().rows(&mark.name).is_some()
                && (self.marks.as_ref()).is_none_or(|marks| marks.contains(&mark.name))
        };
        match self.strategy {
            Strategy::Exact => {
                let query = Query::new(QueryShape::Point(pointer), Relation::Intersects)
                    .filter(keep)
                    .limit(1);
                tree.query(&query).first().copied()
            }
            Strategy::Nearest { max_radius, metric } => {
                let nearest = Nearest::new(pointer, max_radius)
                    .metric(metric)
                    .filter(keep);
                tree.nearest(nearest)
                    .next()
                    .map(|neighbor| neighbor.instance)
            }
        }
    }
}
