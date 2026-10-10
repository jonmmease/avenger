use std::collections::HashMap;

use avenger_common::types::FillRule;
use avenger_scenegraph::{
    marks::{group::Clip, mark::MarkInstance},
    render_order::{SceneDisplayList, SceneDisplayMark},
    scene_graph::SceneGraph,
};
use geo::{BoundingRect, Distance, Euclidean};
use geo_svg::{Color, CombineToSVG};
use geo_types::Geometry;
use rstar::{Envelope, PointDistance, RTree, RTreeObject, AABB};

use crate::{marks::TextGeometryUtils, region::Region};

/// A selection region in scene coordinates.
#[derive(Debug, Clone)]
pub enum GeometryQueryShape {
    /// A rectangle between two corners, in either order.
    Rect {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
    },
    Circle {
        cx: f32,
        cy: f32,
        radius: f32,
    },
    /// A closed polygon, such as a lasso, which may cross itself: `fill_rule` decides which
    /// areas it encloses.
    Polygon {
        points: Vec<[f32; 2]>,
        fill_rule: FillRule,
    },
}

/// How an instance must lie in a query region to match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometryQueryHitPolicy {
    /// The geometry's centroid, such as a circle symbol's center, lies in the region.
    CentroidInside,
    /// The region comes within the instance's reach of its geometry.
    GeometryIntersects,
    /// The geometry lies in the region, at least its reach from the region's edge.
    GeometryContained,
}

/// One mark instance's hit geometry: every point within `reach` of `geometry`.
#[derive(Debug, Clone)]
pub struct GeometryInstance {
    pub mark_instance: MarkInstance,
    pub interactive: bool,
    /// The instance's shape in scene coordinates. A circle symbol's shape is its center point.
    pub geometry: Geometry<f32>,
    /// How far the drawn instance reaches beyond `geometry`: half its stroke width, plus a circle
    /// symbol's radius.
    pub reach: f32,
}

impl GeometryInstance {
    /// The box around the geometry, grown by the instance's reach.
    pub fn envelope(&self) -> AABB<[f32; 2]> {
        if let Some(bbox) = self.geometry.bounding_rect() {
            AABB::from_corners(
                [bbox.min().x - self.reach, bbox.min().y - self.reach],
                [bbox.max().x + self.reach, bbox.max().y + self.reach],
            )
        } else {
            println!("No bounding box for geometry: {:?}", self.geometry);
            AABB::from_corners([0.0, 0.0], [0.0, 0.0])
        }
    }
}

/// An instance in the tree, with its place in the tree's draw order.
#[derive(Debug, Clone)]
struct Node {
    /// Where the instance draws among the tree's instances, bottom first. No two nodes share one.
    draw_index: usize,
    instance: GeometryInstance,
}

impl Node {
    fn geometry_distance(&self, point: &[f32; 2]) -> f32 {
        let point = geo_types::Point::new(point[0], point[1]);
        Euclidean::distance(&self.instance.geometry, &point)
    }
}

impl RTreeObject for Node {
    type Envelope = AABB<[f32; 2]>;

    fn envelope(&self) -> Self::Envelope {
        self.instance.envelope()
    }
}

impl PointDistance for Node {
    /// The squared distance from `point` to the drawn instance, which is 0 within its reach.
    fn distance_2(&self, point: &[f32; 2]) -> f32 {
        let distance = (self.geometry_distance(point) - self.instance.reach).max(0.0);
        distance * distance
    }

    fn contains_point(&self, point: &[f32; 2]) -> bool {
        self.geometry_distance(point) <= self.instance.reach
    }
}

#[derive(Debug, Clone)]
pub struct SceneGraphRTree {
    /// The R-tree containing the geometries, relative to the scene graph origin
    rtree: RTree<Node>,
    /// The envelope of the scene graph, relative to the scene graph origin
    envelope: AABB<[f32; 2]>,
    /// Absolute origin of each group
    group_origins: HashMap<Vec<usize>, [f32; 2]>,
    /// Names of each named group
    group_names: HashMap<String, Vec<usize>>,
    /// Group names keyed by path, retaining repeated names in separate branches.
    group_names_by_path: HashMap<Vec<usize>, String>,
    /// The draw index that the next inserted instance gets.
    next_draw_index: usize,
    /// The clip, in scene coordinates, of each mark that a clip cuts, by mark path.
    clips: HashMap<Vec<usize>, Clip>,
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

        // The instances come in draw order, bottom first.
        let next_draw_index = geometries.len();
        let rtree = RTree::bulk_load(
            geometries
                .into_iter()
                .enumerate()
                .map(|(draw_index, instance)| Node {
                    draw_index,
                    instance,
                })
                .collect(),
        );

        Self {
            rtree,
            envelope,
            group_origins,
            group_names,
            group_names_by_path,
            next_draw_index,
            clips: HashMap::new(),
        }
    }

    pub fn from_scene_graph(
        scene_graph: &SceneGraph,
        text_engine: &avenger_typst_label::LabelEngine,
    ) -> SceneGraphRTree {
        let mut geometry_instances: Vec<GeometryInstance> = vec![];

        // Build geometry in the same global bottom-to-top order used by the
        // renderer. Z-index is global across the chart; a hierarchy path does
        // not imply that one mark is visually above another.
        let display_list = SceneDisplayList::from_scene_graph(scene_graph);
        for item in display_list.ordered_items() {
            let SceneDisplayMark::Borrowed(mark) = &item.mark else {
                // Group fill/stroke paths were not part of the historical
                // interaction geometry contract. Keep that behavior while
                // ordering ordinary primitive marks consistently with render.
                continue;
            };
            geometry_instances.extend(
                mark.geometry_iter(item.mark_path.clone(), item.origin, text_engine)
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

        let mut tree = SceneGraphRTree::new(
            geometry_instances,
            scene_graph.group_origins(),
            scene_graph.group_names(),
            group_names_by_path,
        );
        tree.clips = display_list
            .items
            .iter()
            .filter(|item| !matches!(item.clip, Clip::None))
            .map(|item| (item.mark_path.clone(), item.clip.clone()))
            .collect();
        tree
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
    pub fn iter(&self) -> impl Iterator<Item = &GeometryInstance> + '_ {
        self.rtree.iter().map(|node| &node.instance)
    }

    /// Returns a single top-most mark instance at a given point.
    ///
    /// If multiple marks or mark instances contain the given point, the top-most one is returned.
    /// A mark in a clipped group is hit only where the clip leaves it visible.
    pub fn pick_top_mark_at_point(&self, point: &[f32; 2]) -> Option<&MarkInstance> {
        self.rtree
            .locate_all_at_point(*point)
            .filter(
                |node| match self.clips.get(&node.instance.mark_instance.mark_path) {
                    None | Some(Clip::None) => true,
                    Some(Clip::Rect {
                        x,
                        y,
                        width,
                        height,
                    }) => {
                        *width > 0.0
                            && *height > 0.0
                            && point[0] >= *x
                            && point[0] <= x + width
                            && point[1] >= *y
                            && point[1] <= y + height
                    }
                    Some(Clip::Path { path, fill_rule }) => {
                        lyon_algorithms::hit_test::hit_test_path(
                            &lyon_path::math::point(point[0], point[1]),
                            path.iter(),
                            (*fill_rule).into(),
                            0.01,
                        )
                    }
                },
            )
            .max_by_key(|node| node.draw_index)
            .map(|node| &node.instance.mark_instance)
    }

    /// Returns a single object that covers a given point.
    ///
    /// If multiple elements contain the given point, any of them is returned.
    pub fn locate_at_point(&self, point: &[f32; 2]) -> Option<&GeometryInstance> {
        self.rtree
            .locate_at_point(*point)
            .map(|node| &node.instance)
    }

    /// Returns every object that covers a given point.
    pub fn locate_all_at_point(
        &self,
        point: &[f32; 2],
    ) -> impl Iterator<Item = &GeometryInstance> + '_ {
        self.rtree
            .locate_all_at_point(*point)
            .map(|node| &node.instance)
    }

    /// Returns all elements contained in an envelope
    pub fn locate_in_envelope(
        &self,
        envelope: &AABB<[f32; 2]>,
    ) -> impl Iterator<Item = &GeometryInstance> + '_ {
        self.rtree
            .locate_in_envelope(*envelope)
            .map(|node| &node.instance)
    }

    /// Returns all elements whose envelope intersects a given envelope
    pub fn locate_in_envelope_intersecting(
        &self,
        envelope: &AABB<[f32; 2]>,
    ) -> impl Iterator<Item = &GeometryInstance> + '_ {
        self.rtree
            .locate_in_envelope_intersecting(*envelope)
            .map(|node| &node.instance)
    }

    /// The interactive instances that match `policy` in `shape`, ordered by mark path and
    /// instance.
    ///
    /// An instance in a clipped mark matches only in the part of `shape` that the clip leaves
    /// visible, so a lasso drawn past a plot's edge doesn't select points clipped out of view.
    pub fn query_shape(
        &self,
        shape: &GeometryQueryShape,
        policy: GeometryQueryHitPolicy,
    ) -> Vec<&GeometryInstance> {
        let Some(region) = Region::new(shape) else {
            return Vec::new();
        };
        let mut clipped: HashMap<&[usize], Option<Region>> = HashMap::new();
        let mut matches = Vec::new();
        for instance in self.locate_in_envelope_intersecting(&region.envelope()) {
            let path = instance.mark_instance.mark_path.as_slice();
            let visible = match self.clips.get(path) {
                None => Some(&region),
                Some(clip) => clipped
                    .entry(path)
                    .or_insert_with(|| region.clipped(clip))
                    .as_ref(),
            };
            if visible.is_some_and(|visible| visible.matches(instance, policy)) {
                matches.push(instance);
            }
        }
        matches.sort_by(|a, b| {
            a.mark_instance
                .mark_path
                .cmp(&b.mark_instance.mark_path)
                .then_with(|| {
                    a.mark_instance
                        .instance_index
                        .cmp(&b.mark_instance.instance_index)
                })
        });
        matches
    }

    /// Returns the nearest neighbor for a given point
    pub fn nearest_neighbor(&self, query_point: &[f32; 2]) -> Option<&GeometryInstance> {
        self.rtree
            .nearest_neighbor(*query_point)
            .map(|node| &node.instance)
    }

    /// Returns all elements of the tree sorted by their distance to a given point
    pub fn nearest_neighbor_iter(
        &self,
        query_point: &[f32; 2],
    ) -> impl Iterator<Item = &GeometryInstance> + '_ {
        self.rtree
            .nearest_neighbor_iter(*query_point)
            .map(|node| &node.instance)
    }

    /// Returns all elements of the tree within a certain distance
    pub fn locate_within_distance(
        &self,
        query_point: [f32; 2],
        max_squared_radius: f32,
    ) -> impl Iterator<Item = &GeometryInstance> + '_ {
        self.rtree
            .locate_within_distance(query_point, max_squared_radius)
            .map(|node| &node.instance)
    }

    /// Returns all elements of the tree sorted by their distance, along with their squared
    /// distances
    pub fn nearest_neighbor_iter_with_distance_2(
        &self,
        query_point: &[f32; 2],
    ) -> impl Iterator<Item = (&GeometryInstance, f32)> + '_ {
        self.rtree
            .nearest_neighbor_iter_with_distance_2(*query_point)
            .map(|(node, distance_2)| (&node.instance, distance_2))
    }

    /// Returns all nearest neighbors that have exactly the same distance
    pub fn nearest_neighbors(&self, query_point: &[f32; 2]) -> Vec<&GeometryInstance> {
        self.rtree
            .nearest_neighbors(query_point)
            .into_iter()
            .map(|node| &node.instance)
            .collect()
    }

    /// Returns all possible intersecting objects between this and another tree
    pub fn intersection_candidates_with_other_tree<'a>(
        &'a self,
        other: &'a SceneGraphRTree,
    ) -> impl Iterator<Item = (&'a GeometryInstance, &'a GeometryInstance)> + 'a {
        self.rtree
            .intersection_candidates_with_other_tree(&other.rtree)
            .map(|(a, b)| (&a.instance, &b.instance))
    }

    /// Insert a new geometry instance into the tree, above the instances it holds
    pub fn insert(&mut self, geometry: GeometryInstance) {
        // Update the envelope to include the new geometry
        self.envelope = self.envelope.merged(&geometry.envelope());

        self.rtree.insert(Node {
            draw_index: self.next_draw_index,
            instance: geometry,
        });
        self.next_draw_index += 1;
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
        marks::{group::SceneGroup, mark::SceneMark, rect::SceneRectMark},
        scene_graph::SceneGraph,
    };
    use avenger_typst_label::bundled_label_engine;
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
            geometry: Geometry::Point(Point::new(x, y)),
            reach: 0.0,
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
            geometry: Geometry::Rect(Rect::new(
                geo_types::Coord { x: x0, y: y0 },
                geo_types::Coord { x: x1, y: y1 },
            )),
            reach: 0.0,
        }
    }

    fn test_tree(geometries: Vec<GeometryInstance>) -> SceneGraphRTree {
        SceneGraphRTree::new(geometries, HashMap::new(), HashMap::new(), HashMap::new())
    }

    fn hit_rect(name: &str, zindex: Option<i32>) -> SceneMark {
        SceneRectMark {
            name: name.to_string(),
            x: 0.0.into(),
            y: 0.0.into(),
            width: Some(20.0.into()),
            height: Some(20.0.into()),
            zindex,
            ..Default::default()
        }
        .into()
    }

    fn hit_scene(marks: Vec<SceneMark>) -> SceneGraph {
        SceneGraph {
            marks,
            width: 20.0,
            height: 20.0,
            origin: [0.0, 0.0],
        }
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
        let bounds = scene_mark.bounding_box(&bundled_label_engine());
        assert_eq!(bounds.lower(), [10.0, 20.0]);
        assert_eq!(bounds.upper(), [40.0, 60.0]);

        let scene = SceneGraph {
            marks: vec![scene_mark],
            width: 100.0,
            height: 100.0,
            origin: [0.0, 0.0],
        };
        let rtree = SceneGraphRTree::from_scene_graph(&scene, &bundled_label_engine());
        assert_eq!(rtree.size(), 0);
        assert!(rtree.pick_top_mark_at_point(&[20.0, 30.0]).is_none());
    }

    #[test]
    fn top_pick_obeys_translated_rect_and_path_clips() {
        let mut top = hit_rect("clipped", None);
        if let SceneMark::Rect(rect) = &mut top {
            rect.clip = true;
        }
        let scene = hit_scene(vec![
            hit_rect("under", None),
            SceneGroup {
                origin: [3.0, 4.0],
                clip: Clip::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 5.0,
                    height: 5.0,
                },
                marks: vec![top.clone()],
                ..Default::default()
            }
            .into(),
        ]);
        let tree =
            SceneGraphRTree::from_scene_graph(&scene, &avenger_typst_label::bundled_label_engine());
        assert_eq!(
            tree.pick_top_mark_at_point(&[4.0, 5.0]).unwrap().name,
            "clipped"
        );
        assert_eq!(
            tree.pick_top_mark_at_point(&[10.0, 10.0]).unwrap().name,
            "under"
        );
        use avenger_common::types::FillRule;
        let mut path = lyon_path::Path::builder();
        for [x, y, side] in [[0.0, 0.0, 10.0], [1.0, 1.0, 3.0]] {
            path.begin(lyon_path::math::point(x, y));
            path.line_to(lyon_path::math::point(x + side, y));
            path.line_to(lyon_path::math::point(x, y + side));
            path.close();
        }
        let path = path.build();
        for (fill_rule, expected) in [(FillRule::NonZero, "clipped"), (FillRule::EvenOdd, "under")]
        {
            let scene = hit_scene(vec![
                hit_rect("under", None),
                SceneGroup {
                    clip: Clip::Path {
                        path: path.clone(),
                        fill_rule,
                    },
                    marks: vec![top.clone()],
                    ..Default::default()
                }
                .into(),
            ]);
            let tree = SceneGraphRTree::from_scene_graph(
                &scene,
                &avenger_typst_label::bundled_label_engine(),
            );
            assert_eq!(
                tree.pick_top_mark_at_point(&[2.0, 2.0]).unwrap().name,
                expected
            );
            assert_eq!(
                tree.pick_top_mark_at_point(&[1.0, 6.0]).unwrap().name,
                "clipped"
            );
            assert_eq!(
                tree.pick_top_mark_at_point(&[8.0, 8.0]).unwrap().name,
                "under"
            );
        }
    }

    #[test]
    fn top_pick_uses_global_z_index_before_scene_path() {
        let scene = hit_scene(vec![
            hit_rect("visually_top", Some(10)),
            hit_rect("later_but_under", Some(-10)),
        ]);
        let rtree = SceneGraphRTree::from_scene_graph(&scene, &bundled_label_engine());

        assert_eq!(
            rtree
                .pick_top_mark_at_point(&[10.0, 10.0])
                .expect("overlapping rect")
                .name,
            "visually_top"
        );
    }

    #[test]
    fn top_pick_uses_document_order_to_break_equal_z_index_ties() {
        let scene = hit_scene(vec![
            hit_rect("first", Some(2)),
            hit_rect("second", Some(2)),
        ]);
        let rtree = SceneGraphRTree::from_scene_graph(&scene, &bundled_label_engine());

        assert_eq!(
            rtree
                .pick_top_mark_at_point(&[10.0, 10.0])
                .expect("overlapping equal-z rect")
                .name,
            "second"
        );
    }

    #[test]
    fn inherited_group_z_index_is_global_across_the_chart() {
        let high_group = SceneGroup {
            name: "high_group".into(),
            zindex: Some(20),
            marks: vec![hit_rect("nested_top", None)],
            ..Default::default()
        };
        let scene = hit_scene(vec![
            high_group.into(),
            hit_rect("later_root_but_under", Some(5)),
        ]);
        let rtree = SceneGraphRTree::from_scene_graph(&scene, &bundled_label_engine());

        assert_eq!(
            rtree
                .pick_top_mark_at_point(&[10.0, 10.0])
                .expect("nested and root overlap")
                .name,
            "nested_top"
        );
    }

    #[test]
    fn later_instance_within_one_mark_is_topmost() {
        let mark = SceneRectMark {
            name: "rects".into(),
            len: 2,
            x: vec![0.0, 0.0].into(),
            y: vec![0.0, 0.0].into(),
            width: Some(vec![20.0, 20.0].into()),
            height: Some(vec![20.0, 20.0].into()),
            ..Default::default()
        };
        let rtree = SceneGraphRTree::from_scene_graph(
            &hit_scene(vec![mark.into()]),
            &bundled_label_engine(),
        );

        assert_eq!(
            rtree
                .pick_top_mark_at_point(&[10.0, 10.0])
                .expect("overlapping mark instances")
                .instance_index,
            Some(1)
        );
    }

    /// An instance inserted into a built tree draws above its scene's marks.
    #[test]
    fn inserted_instance_is_topmost() {
        let mut tree = SceneGraphRTree::from_scene_graph(
            &hit_scene(vec![hit_rect("under", None)]),
            &bundled_label_engine(),
        );
        let mut probe = rect_instance(vec![1], 0, 0.0, 0.0, 20.0, 20.0);
        probe.mark_instance.name = "probe".to_string();
        tree.insert(probe);
        assert_eq!(
            tree.pick_top_mark_at_point(&[5.0, 5.0]).unwrap().name,
            "probe"
        );
    }

    /// The nearest instance is the closest one, however far the query point is: the tree ranks
    /// instances and subtrees on the same squared distances.
    #[test]
    fn nearest_neighbor_matches_the_closest_instance() {
        let mut seed = 7u64;
        let mut next = move || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 40) as f32 / (1u64 << 24) as f32 * 100.0
        };
        let points: Vec<_> = (0..300).map(|_| [next(), next()]).collect();
        let tree = test_tree(
            points
                .iter()
                .enumerate()
                .map(|(i, p)| point_instance(vec![0], i, p[0], p[1]))
                .collect(),
        );
        let distance = |p: &[f32; 2], q: &[f32; 2]| (p[0] - q[0]).hypot(p[1] - q[1]);
        for _ in 0..200 {
            let query = [next() * 3.0 - 100.0, next() * 3.0 - 100.0];
            let closest = points
                .iter()
                .map(|p| distance(p, &query))
                .fold(f32::INFINITY, f32::min);
            let Geometry::Point(nearest) = &tree.nearest_neighbor(&query).unwrap().geometry else {
                panic!("point instances");
            };
            assert_eq!(distance(&[nearest.x(), nearest.y()], &query), closest);
        }
    }

    #[test]
    fn rect_query_sorts_matches_by_mark_and_instance() {
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
            GeometryQueryHitPolicy::CentroidInside,
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
    fn circle_query_matches_centroids() {
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
            GeometryQueryHitPolicy::CentroidInside,
        );

        let ids = matches
            .iter()
            .map(|instance| instance.mark_instance.instance_index)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![Some(0), Some(1)]);
    }

    #[test]
    fn polygon_query_matches_centroids() {
        let tree = test_tree(vec![
            point_instance(vec![0], 0, 5.0, 5.0),
            point_instance(vec![0], 1, 12.0, 8.0),
            point_instance(vec![0], 2, 15.0, 18.0),
        ]);

        let matches = tree.query_shape(
            &GeometryQueryShape::Polygon {
                points: vec![[0.0, 0.0], [20.0, 0.0], [10.0, 20.0]],
                fill_rule: FillRule::NonZero,
            },
            GeometryQueryHitPolicy::CentroidInside,
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
