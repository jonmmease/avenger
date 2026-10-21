//! Focus: the data under the pointer that a tooltip describes, which a focus strategy finds in
//! the hit-testing tree.

use std::collections::HashMap;

use avenger_scenegraph::marks::group::Clip;
use avenger_scenegraph::marks::mark::MarkInstance;
use geo::line_intersection::{line_intersection, LineIntersection};
use geo::BoundingRect;
use geo_types::{coord, Geometry, Line, LineString, Polygon};
use rstar::{PointDistance, AABB};

use avenger_scenegraph::render::order::{SceneDisplayList, SceneDisplayMark};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::LabelEngine;

use crate::data::{Datum, SceneData};
use crate::marks::TextGeometryUtils;
use crate::rtree::{clip_shows, Node, SceneGraphRTree};
use crate::GeometryInstance;

/// How a focus finds the data at the pointer, among the tree's instances with rows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FocusStrategy {
    /// The topmost instance under the pointer.
    Exact,
    /// The instance whose drawn edge lies nearest the pointer, within `max_radius` pixels.
    Nearest { max_radius: f32 },
    /// The instance nearest the pointer along x, within `max_radius` pixels along x.
    NearestX { max_radius: f32 },
    /// The instance nearest the pointer along y, within `max_radius` pixels along y.
    NearestY { max_radius: f32 },
    /// For each mark with instances within `max_radius` pixels of the pointer along x, its
    /// instance nearest the pointer along x, such as each series' vertex in a time series. With
    /// a `limit` of k, the k of them nearest the pointer along y. Members come in the order of
    /// their positions down the scene.
    GroupX {
        max_radius: f32,
        limit: Option<usize>,
    },
}

impl Default for FocusStrategy {
    /// The nearest instance within 40 pixels.
    fn default() -> Self {
        Self::Nearest { max_radius: 40.0 }
    }
}

/// A search of the hit-testing tree for the focus at a pointer: the instances that a strategy
/// finds there, among some marks or every mark with rows, while a tooltip shows a focus or
/// none.
#[derive(Debug, Clone, Copy)]
pub struct FocusQuery<'a> {
    pointer: [f32; 2],
    strategy: FocusStrategy,
    marks: Option<&'a [String]>,
    shown: Option<&'a Focus>,
}

impl<'a> FocusQuery<'a> {
    /// The focus that `strategy` finds at `pointer`, among every mark with rows.
    pub fn at(pointer: [f32; 2], strategy: FocusStrategy) -> Self {
        Self {
            pointer,
            strategy,
            marks: None,
            shown: None,
        }
    }

    /// The query, among the instances of the marks named `marks` alone.
    pub fn among(self, marks: &'a [String]) -> Self {
        Self {
            marks: Some(marks),
            ..self
        }
    }

    /// The query, while a tooltip shows `shown`. A strategy with a limit keeps a shown member's
    /// mark in its place until another mark is nearer by more than a pixel, so rows don't
    /// flicker between marks that are about as near.
    pub fn showing(self, shown: &'a Focus) -> Self {
        Self {
            shown: Some(shown),
            ..self
        }
    }
}

/// The data that a tooltip describes, with the pointer that found it.
#[derive(Debug, Clone)]
pub struct Focus {
    pub pointer: [f32; 2],
    pub members: Vec<Member>,
}

impl Focus {
    /// The focus with each member's row in `data`, such as a newer build's rows, so the
    /// methods that find members again by instance index, [`Focus::in_scene`] and
    /// [`SceneGraphRTree::find_again`], find the same data. A member keeps its row when its
    /// mark's rows are the same rows, and otherwise takes the row whose `key` column holds the
    /// member's key, as [`Datum::row_in`] finds it. `None` when a member's row is gone: its mark
    /// has no rows in `data`, or other rows and there's no key, or no row holds its key. Positions
    /// stay the focus's until the focus is found again.
    pub fn in_data(&self, data: &SceneData, key: Option<&str>) -> Option<Focus> {
        let members = self
            .members
            .iter()
            .map(|member| {
                let rows = data.rows(&member.mark().name)?;
                let row = member.datum.row_in(rows, key)?;
                let mut member = member.clone();
                member.entry.mark_instance.instance_index = Some(row);
                member.datum = Datum {
                    rows: rows.clone(),
                    row,
                };
                Some(member)
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Focus {
            pointer: self.pointer,
            members,
        })
    }

    /// The focus as `scene` draws it: each member's instance in the scene, by its mark's name
    /// and instance index, with its row in the scene's `data` at that index, which
    /// [`Focus::in_data`] keeps on the member's data when the rows change. A scene that the app
    /// built without its hit-testing tree, as during a drag, can draw its data elsewhere, or
    /// other data, than the tree that found the focus, and an overlay draws the focus as the
    /// scene draws it. Members that the scene doesn't draw, or draws where their clip hides them,
    /// or whose marks have no rows in `data`, are left out, and `None` when none is left. The
    /// pointer stays the focus's.
    pub fn in_scene(
        &self,
        scene: &SceneGraph,
        data: &SceneData,
        text_engine: &LabelEngine,
    ) -> Option<Focus> {
        let display_list = SceneDisplayList::from_scene_graph(scene);
        let members: Vec<Member> = self
            .members
            .iter()
            .filter_map(|member| {
                let wanted = member.mark();
                let (item, mark) = display_list
                    .items
                    .iter()
                    .find_map(|item| match &item.mark {
                        SceneDisplayMark::Borrowed(mark) if mark.mark_name() == wanted.name => {
                            Some((item, mark))
                        }
                        _ => None,
                    })?;
                let entry = mark
                    .geometry_iter(item.mark_path.clone(), item.origin, text_engine)
                    .find(|entry| entry.mark_instance.instance_index == wanted.instance_index)?;
                let rows = data.rows(&wanted.name)?;
                let row = wanted.instance_index.filter(|row| *row < rows.num_rows())?;
                let position = position(&entry);
                let clip = (!matches!(item.clip, Clip::None)).then(|| item.clip.clone());
                if clip
                    .as_ref()
                    .is_some_and(|clip| !clip_shows(clip, &position))
                {
                    return None;
                }
                Some(Member {
                    entry,
                    position,
                    datum: Datum {
                        rows: rows.clone(),
                        row,
                    },
                    clip,
                })
            })
            .collect();
        (!members.is_empty()).then_some(Focus {
            pointer: self.pointer,
            members,
        })
    }

    /// Whether two focuses hold the same instances at the same positions, so a tooltip for one
    /// shows the other.
    pub fn same_members(&self, other: &Focus) -> bool {
        self.members.len() == other.members.len()
            && self
                .members
                .iter()
                .zip(&other.members)
                .all(|(a, b)| a.mark() == b.mark() && a.position == b.position)
    }
}

/// One instance in a focus.
#[derive(Debug, Clone)]
pub struct Member {
    /// The instance's entry in the tree: its mark, geometry, and reach.
    pub entry: GeometryInstance,
    /// The instance's position in scene coordinates: a point's position, or the center of its
    /// geometry's bounding box.
    pub position: [f32; 2],
    /// The instance's row, from the scene data of the build that drew it.
    pub datum: Datum,
    /// The clip, in scene coordinates, that cuts the instance's mark, if any.
    pub clip: Option<Clip>,
}

/// A side of an anchor, on which a tooltip lies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Right,
    Left,
    Above,
    Below,
}

impl Side {
    /// The side's direction in scene coordinates, where y grows down.
    pub fn direction(self) -> [f32; 2] {
        match self {
            Side::Right => [1.0, 0.0],
            Side::Left => [-1.0, 0.0],
            Side::Above => [0.0, -1.0],
            Side::Below => [0.0, 1.0],
        }
    }
}

impl Member {
    pub fn mark(&self) -> &MarkInstance {
        &self.entry.mark_instance
    }

    /// The point where the instance's drawn edge meets `side`. A point's edge lies its reach
    /// from its position, such as a circle symbol's radius and half its stroke. Other geometry's
    /// edge lies where a ray from its bounding box's center toward the side leaves it, plus its
    /// reach.
    pub fn anchor(&self, side: Side) -> [f32; 2] {
        let [dx, dy] = side.direction();
        let reach = self.entry.reach;
        let [x, y] = match &self.entry.geometry {
            Geometry::Point(_) => self.position,
            geometry => exit(geometry, self.position, side).unwrap_or(self.position),
        };
        [x + dx * reach, y + dy * reach]
    }
}

/// Where a ray from `center` toward `side` last crosses `geometry`'s lines, or `None` when it
/// crosses none.
fn exit(geometry: &Geometry<f32>, center: [f32; 2], side: Side) -> Option<[f32; 2]> {
    let bounds = geometry.bounding_rect()?;
    let [dx, dy] = side.direction();
    let length = bounds.width() + bounds.height() + 1.0;
    let ray = Line::new(
        coord! { x: center[0], y: center[1] },
        coord! { x: center[0] + dx * length, y: center[1] + dy * length },
    );
    let mut lines = Vec::new();
    segments(geometry, &mut lines);
    lines
        .into_iter()
        .filter_map(|line| match line_intersection(ray, line)? {
            LineIntersection::SinglePoint { intersection, .. } => Some(intersection),
            LineIntersection::Collinear { intersection } => Some(intersection.end),
        })
        .map(|point| [point.x, point.y])
        .max_by(|a, b| {
            let along = |p: &[f32; 2]| (p[0] - center[0]) * dx + (p[1] - center[1]) * dy;
            along(a).total_cmp(&along(b))
        })
}

/// Push `geometry`'s line segments onto `lines`: a line string's segments, and the segments of
/// a polygon's rings.
fn segments(geometry: &Geometry<f32>, lines: &mut Vec<Line<f32>>) {
    let rings = |polygon: &Polygon<f32>, lines: &mut Vec<Line<f32>>| {
        for ring in std::iter::once(polygon.exterior()).chain(polygon.interiors()) {
            lines.extend(ring.lines());
        }
    };
    let strings =
        |string: &LineString<f32>, lines: &mut Vec<Line<f32>>| lines.extend(string.lines());
    match geometry {
        Geometry::Point(_) | Geometry::MultiPoint(_) => {}
        Geometry::Line(line) => lines.push(*line),
        Geometry::LineString(string) => strings(string, lines),
        Geometry::MultiLineString(multi) => multi.iter().for_each(|string| strings(string, lines)),
        Geometry::Polygon(polygon) => rings(polygon, lines),
        Geometry::MultiPolygon(multi) => multi.iter().for_each(|polygon| rings(polygon, lines)),
        Geometry::Rect(rect) => rings(&rect.to_polygon(), lines),
        Geometry::Triangle(triangle) => rings(&triangle.to_polygon(), lines),
        Geometry::GeometryCollection(collection) => collection
            .iter()
            .for_each(|geometry| segments(geometry, lines)),
    }
}

impl SceneGraphRTree {
    /// The focus that `query` finds, or `None` when it finds no instance. A focus holds only
    /// instances that have rows in the tree's scene data, since it describes data. An instance
    /// in a clipped mark is found only where its clip leaves it visible.
    pub fn focus(&self, query: FocusQuery) -> Option<Focus> {
        let FocusQuery {
            pointer,
            strategy,
            marks,
            shown,
        } = query;
        let candidate = |node: &&Node| {
            let instance = &node.instance.mark_instance;
            instance.instance_index.is_some()
                && marks.is_none_or(|marks| marks.contains(&instance.name))
                && self.data.rows(&instance.name).is_some()
                && self.visible_at(node, &pointer)
        };
        let topmost = |a: &&Node, b: &&Node| b.draw_index.cmp(&a.draw_index);
        let nodes = match strategy {
            FocusStrategy::Exact => self
                .rtree
                .locate_all_at_point(pointer)
                .filter(candidate)
                .max_by_key(|node| node.draw_index)
                .into_iter()
                .collect(),
            FocusStrategy::Nearest { max_radius } => self
                .rtree
                .locate_within_distance(pointer, max_radius * max_radius)
                .filter(candidate)
                .min_by(|a, b| {
                    a.distance_2(&pointer)
                        .total_cmp(&b.distance_2(&pointer))
                        .then_with(|| topmost(a, b))
                })
                .into_iter()
                .collect(),
            FocusStrategy::NearestX { max_radius } => self
                .band(pointer, 0, max_radius)
                .filter(candidate)
                .min_by(|a, b| nearest_along(a, b, pointer, 0).then_with(|| topmost(a, b)))
                .into_iter()
                .collect(),
            FocusStrategy::NearestY { max_radius } => self
                .band(pointer, 1, max_radius)
                .filter(candidate)
                .min_by(|a, b| nearest_along(a, b, pointer, 1).then_with(|| topmost(a, b)))
                .into_iter()
                .collect(),
            FocusStrategy::GroupX { max_radius, limit } => {
                self.group_x(pointer, max_radius, limit, shown, candidate)
            }
        };
        let members: Vec<Member> = nodes
            .into_iter()
            .filter_map(|node: &Node| self.member(&node.instance))
            .collect();
        (!members.is_empty()).then_some(Focus { pointer, members })
    }

    /// The instances of `focus` in this tree, which can be another tree than the one that found
    /// them, by their marks' names and instance indices, at `focus`'s pointer, with their rows in
    /// the tree's data, which [`Focus::in_data`] keeps on the members' data when the rows change.
    /// `None` when one of them isn't in the tree, or lies where its clip hides it.
    pub fn find_again(&self, focus: &Focus) -> Option<Focus> {
        let members = focus
            .members
            .iter()
            .map(|member| {
                let wanted = member.mark();
                let node = self.rtree.iter().find(|node| {
                    let instance = &node.instance.mark_instance;
                    instance.name == wanted.name && instance.instance_index == wanted.instance_index
                })?;
                if !self.visible_at(node, &position(&node.instance)) {
                    return None;
                }
                self.member(&node.instance)
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Focus {
            pointer: focus.pointer,
            members,
        })
    }

    /// The nodes in the band within `max_radius` of `pointer` along `axis`.
    fn band(
        &self,
        pointer: [f32; 2],
        axis: usize,
        max_radius: f32,
    ) -> impl Iterator<Item = &Node> + '_ {
        let mut lower = [f32::MIN, f32::MIN];
        let mut upper = [f32::MAX, f32::MAX];
        lower[axis] = pointer[axis] - max_radius;
        upper[axis] = pointer[axis] + max_radius;
        self.rtree
            .locate_in_envelope_intersecting(AABB::from_corners(lower, upper))
    }

    /// For each mark in the band within `max_radius` of the pointer's x, its node nearest the
    /// pointer's x, or of two as near, the first. With a limit, the marks of the nodes nearest
    /// the pointer's y, where a shown mark gains a pixel. The nodes come in the order of their
    /// positions' y.
    fn group_x<'a>(
        &'a self,
        pointer: [f32; 2],
        max_radius: f32,
        limit: Option<usize>,
        shown: Option<&Focus>,
        candidate: impl Fn(&&'a Node) -> bool,
    ) -> Vec<&'a Node> {
        let mut nearest: HashMap<&[usize], &Node> = HashMap::new();
        for node in self.band(pointer, 0, max_radius).filter(candidate) {
            let mark = node.instance.mark_instance.mark_path.as_slice();
            let index = |node: &Node| node.instance.mark_instance.instance_index;
            let nearer = nearest.get(mark).is_none_or(|other| {
                nearest_along(node, other, pointer, 0)
                    .then_with(|| index(node).cmp(&index(other)))
                    .is_lt()
            });
            if nearer {
                nearest.insert(mark, node);
            }
        }
        let mut nodes: Vec<&Node> = nearest.into_values().collect();
        if let Some(limit) = limit {
            let shown_marks: Vec<&[usize]> = shown
                .map(|focus| {
                    focus
                        .members
                        .iter()
                        .map(|member| member.mark().mark_path.as_slice())
                        .collect()
                })
                .unwrap_or_default();
            let distance = |node: &Node| {
                let mark = node.instance.mark_instance.mark_path.as_slice();
                let margin = if shown_marks.contains(&mark) {
                    1.0
                } else {
                    0.0
                };
                gap(node, pointer, 1) - margin
            };
            nodes.sort_by(|a, b| distance(a).total_cmp(&distance(b)));
            nodes.truncate(limit);
        }
        nodes.sort_by(|a, b| position(&a.instance)[1].total_cmp(&position(&b.instance)[1]));
        nodes
    }

    /// The focus member of a tree entry, with its row, or `None` when its mark has no rows.
    fn member(&self, entry: &GeometryInstance) -> Option<Member> {
        Some(Member {
            entry: entry.clone(),
            position: position(entry),
            datum: self.datum(&entry.mark_instance)?,
            clip: self.clips.get(&entry.mark_instance.mark_path).cloned(),
        })
    }
}

/// How `a` and `b` order by their distance from `pointer` along `axis`, and then across it.
fn nearest_along(a: &Node, b: &Node, pointer: [f32; 2], axis: usize) -> std::cmp::Ordering {
    gap(a, pointer, axis)
        .total_cmp(&gap(b, pointer, axis))
        .then_with(|| gap(a, pointer, 1 - axis).total_cmp(&gap(b, pointer, 1 - axis)))
}

/// The distance along `axis` from `pointer` to `node`'s envelope, or 0 inside it.
fn gap(node: &Node, pointer: [f32; 2], axis: usize) -> f32 {
    let envelope = node.instance.envelope();
    (envelope.lower()[axis] - pointer[axis])
        .max(pointer[axis] - envelope.upper()[axis])
        .max(0.0)
}

/// An entry's position: a point's position, or the center of its geometry's bounding box.
fn position(entry: &GeometryInstance) -> [f32; 2] {
    match &entry.geometry {
        Geometry::Point(point) => [point.x(), point.y()],
        geometry => geometry
            .bounding_rect()
            .map(|bounds| [bounds.center().x, bounds.center().y])
            .unwrap_or_default(),
    }
}
