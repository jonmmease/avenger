use std::collections::HashMap;

use avenger_scenegraph::{
    marks::{group::Clip, mark::MarkInstance},
    render::order::{SceneDisplayList, SceneDisplayMark},
    scene_graph::SceneGraph,
};
use geo::BoundingRect;
use geo_svg::{Color, CombineToSVG};
use geo_types::Geometry;
use rstar::{Envelope, RTree, RTreeObject, AABB};

use crate::data::{instance_count, Datum, SceneData, SceneDataError};
use crate::{marks::TextGeometryUtils, shape::ClipArea};

/// One mark instance's hit geometry: every point within its reach of `geometry`.
#[derive(Debug, Clone)]
pub struct GeometryInstance {
    pub mark_instance: MarkInstance,
    /// The instance's shape in scene coordinates. A circle symbol's shape is its center point.
    pub geometry: Geometry<f32>,
    /// How far the fill extends beyond `geometry`: a circle symbol's radius, and 0 otherwise.
    pub radius: f32,
    /// How far the drawn instance extends beyond its fill: half its stroke width, or a text
    /// label's hit margin.
    pub margin: f32,
}

impl GeometryInstance {
    /// How far the drawn instance reaches beyond `geometry`: its radius and its margin, where
    /// neither is negative.
    pub fn reach(&self) -> f32 {
        self.radius.max(0.0) + self.margin.max(0.0)
    }

    /// The box around the geometry, grown by the instance's reach.
    pub fn envelope(&self) -> AABB<[f32; 2]> {
        let reach = self.reach();
        if let Some(bbox) = self.geometry.bounding_rect() {
            AABB::from_corners(
                [bbox.min().x - reach, bbox.min().y - reach],
                [bbox.max().x + reach, bbox.max().y + reach],
            )
        } else {
            // An empty geometry has no box, and its shape is empty, so no query finds it.
            AABB::from_corners([0.0, 0.0], [0.0, 0.0])
        }
    }
}

/// An instance's place in the tree: its index in the tree's draw order, which is its index among
/// the tree's instances, and its envelope, found once, since the tree asks for it throughout every
/// build and search.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Node {
    /// Where the instance draws among the tree's instances, bottom first. No two nodes share one.
    pub(crate) draw_index: usize,
    pub(crate) envelope: AABB<[f32; 2]>,
}

impl RTreeObject for Node {
    type Envelope = AABB<[f32; 2]>;

    fn envelope(&self) -> Self::Envelope {
        self.envelope
    }
}

/// Instances' draw indices, by mark path and then instance index.
type Identities = HashMap<Vec<usize>, HashMap<Option<usize>, usize>>;

#[derive(Debug, Clone)]
pub struct SceneGraphRTree {
    /// The R-tree of the instances' places, relative to the scene graph origin
    pub(crate) rtree: RTree<Node>,
    /// The instances, in draw order, bottom first
    pub(crate) instances: Vec<GeometryInstance>,
    /// The envelope of the scene graph, relative to the scene graph origin
    envelope: AABB<[f32; 2]>,
    /// Absolute origin of each group
    group_origins: HashMap<Vec<usize>, [f32; 2]>,
    /// Names of each named group
    group_names: HashMap<String, Vec<usize>>,
    /// The clip, in scene coordinates, of each clipped mark, by mark path.
    pub(crate) clips: HashMap<Vec<usize>, Clip>,
    /// Each clip with its path's area resolved, for queries, by mark path.
    clip_areas: HashMap<Vec<usize>, ClipArea>,
    /// Each instance's draw index by its identity.
    identities: Identities,
    /// The rows behind the marks' instances, from the build that made the tree.
    data: SceneData,
}

impl SceneGraphRTree {
    fn new(
        geometries: Vec<GeometryInstance>,
        group_origins: HashMap<Vec<usize>, [f32; 2]>,
        group_names: HashMap<String, Vec<usize>>,
    ) -> Self {
        // The instances come in draw order, bottom first.
        let nodes: Vec<Node> = (geometries.iter().enumerate())
            .map(|(draw_index, instance)| Node {
                draw_index,
                envelope: instance.envelope(),
            })
            .collect();
        let envelope = (nodes.iter())
            .map(|node| node.envelope)
            .reduce(|a, b| a.merged(&b))
            .unwrap_or_else(|| AABB::from_corners([0.0, 0.0], [0.0, 0.0]));
        let mut identities = HashMap::new();
        for (draw_index, instance) in geometries.iter().enumerate() {
            identify(&mut identities, instance, draw_index);
        }

        Self {
            rtree: RTree::bulk_load(nodes),
            instances: geometries,
            envelope,
            group_origins,
            group_names,
            clips: HashMap::new(),
            clip_areas: HashMap::new(),
            identities,
            data: SceneData::new(),
        }
    }

    pub fn from_scene_graph(
        scene_graph: &SceneGraph,
        text_engine: &avenger_typst_label::LabelEngine,
    ) -> SceneGraphRTree {
        let display_list = SceneDisplayList::from_scene_graph(scene_graph);
        Self::from_display_list(scene_graph, &display_list, text_engine)
    }

    /// The tree of `scene_graph`'s interactive instances, each paired with its row in `data`,
    /// the rows that the scene's build made. Rows for a name that no mark in the scene has pair
    /// with nothing, since a scene can leave a mark out. Fails when several marks share a name
    /// that has rows, or when a mark's rows don't number its instances.
    pub fn from_scene_graph_with_data(
        scene_graph: &SceneGraph,
        data: &SceneData,
        text_engine: &avenger_typst_label::LabelEngine,
    ) -> Result<SceneGraphRTree, SceneDataError> {
        let display_list = SceneDisplayList::from_scene_graph(scene_graph);
        for (name, rows) in data.iter() {
            let mut marks = (display_list.items.iter()).filter_map(|item| match &item.mark {
                SceneDisplayMark::Borrowed(mark) if mark.mark_name() == name => Some(mark),
                _ => None,
            });
            let Some(mark) = marks.next() else {
                continue;
            };
            if marks.next().is_some() {
                return Err(SceneDataError::SharedName(name.to_string()));
            }
            let instances = instance_count(mark);
            if rows.num_rows() != instances {
                return Err(SceneDataError::RowCount {
                    mark: name.to_string(),
                    instances,
                    rows: rows.num_rows(),
                });
            }
        }
        let mut tree = Self::from_display_list(scene_graph, &display_list, text_engine);
        tree.data = data.clone();
        Ok(tree)
    }

    /// The rows behind the tree's instances, by mark name.
    pub fn data(&self) -> &SceneData {
        &self.data
    }

    /// The row of `mark`'s instance, or `None` for an instance without an index, of a mark
    /// without rows, or past its mark's rows.
    pub fn datum(&self, mark: &MarkInstance) -> Option<Datum> {
        let rows = self.data.rows(&mark.name)?;
        let row = mark.instance_index?;
        (row < rows.num_rows()).then(|| Datum {
            rows: rows.clone(),
            row,
        })
    }

    fn from_display_list(
        scene_graph: &SceneGraph,
        display_list: &SceneDisplayList,
        text_engine: &avenger_typst_label::LabelEngine,
    ) -> SceneGraphRTree {
        let mut geometry_instances: Vec<GeometryInstance> = vec![];

        // Build geometry in the same global bottom-to-top order used by the
        // renderer. Z-index is global across the chart; a hierarchy path does
        // not imply that one mark is visually above another.
        for item in display_list.ordered_items() {
            let SceneDisplayMark::Borrowed(mark) = &item.mark else {
                // Group fill/stroke paths were not part of the historical
                // interaction geometry contract. Keep that behavior while
                // ordering ordinary primitive marks consistently with render.
                continue;
            };
            if item.interactive {
                geometry_instances.extend(mark.geometry_iter(
                    item.mark_path.clone(),
                    item.origin,
                    text_engine,
                ));
            }
        }

        let mut tree = SceneGraphRTree::new(
            geometry_instances,
            scene_graph.group_origins(),
            scene_graph.group_names(),
        );
        tree.clips = display_list
            .items
            .iter()
            .filter(|item| !matches!(item.clip, Clip::None))
            .map(|item| (item.mark_path.clone(), item.clip.clone()))
            .collect();
        tree.clip_areas = (tree.clips.iter())
            .map(|(path, clip)| (path.clone(), ClipArea::new(clip)))
            .collect();
        tree
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
        self.instances.len()
    }

    /// The tree's instances, in draw order, bottom first.
    pub fn iter(&self) -> impl Iterator<Item = &GeometryInstance> + '_ {
        self.instances.iter()
    }

    /// The clip of the mark at `mark_path`, with its path's area resolved.
    pub(crate) fn clip_area(&self, mark_path: &[usize]) -> &ClipArea {
        static NONE: ClipArea = ClipArea::None;
        self.clip_areas.get(mark_path).unwrap_or(&NONE)
    }

    /// The tree's instance with `mark`'s mark path and instance index, or `None` when the tree
    /// holds no such instance.
    pub fn instance(&self, mark: &MarkInstance) -> Option<&GeometryInstance> {
        let draw_index = self
            .identities
            .get(mark.mark_path.as_slice())?
            .get(&mark.instance_index)?;
        self.instances.get(*draw_index)
    }

    /// Insert a new geometry instance into the tree, above the instances it holds
    pub fn insert(&mut self, geometry: GeometryInstance) {
        let node = Node {
            draw_index: self.instances.len(),
            envelope: geometry.envelope(),
        };
        // Update the envelope to include the new geometry
        self.envelope = self.envelope.merged(&node.envelope);
        identify(&mut self.identities, &geometry, node.draw_index);
        self.instances.push(geometry);
        self.rtree.insert(node);
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

/// Record `instance`'s draw index in `identities`, by its mark path and instance index. A later
/// instance with the same identity replaces it.
fn identify(identities: &mut Identities, instance: &GeometryInstance, draw_index: usize) {
    let mark = &instance.mark_instance;
    (identities.entry(mark.mark_path.clone()).or_default()).insert(mark.instance_index, draw_index);
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
    use crate::query::{Extent, Nearest, Query, QueryShape, Relation};
    use avenger_common::types::FillRule;
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
            geometry: Geometry::Point(Point::new(x, y)),
            radius: 0.0,
            margin: 0.0,
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
            geometry: Geometry::Rect(Rect::new(
                geo_types::Coord { x: x0, y: y0 },
                geo_types::Coord { x: x1, y: y1 },
            )),
            radius: 0.0,
            margin: 0.0,
        }
    }

    fn test_tree(geometries: Vec<GeometryInstance>) -> SceneGraphRTree {
        SceneGraphRTree::new(geometries, HashMap::new(), HashMap::new())
    }

    /// The mark instance of the topmost instance that `point` hits.
    fn pick(tree: &SceneGraphRTree, point: [f32; 2]) -> Option<MarkInstance> {
        let query = Query::new(QueryShape::Point(point), Relation::Intersects);
        (tree.query(&query).first()).map(|instance| instance.mark_instance.clone())
    }

    /// The instance indices of the instances that `query` finds, in its order.
    fn indices(tree: &SceneGraphRTree, query: &Query) -> Vec<Option<usize>> {
        (tree.query(query).iter())
            .map(|instance| instance.mark_instance.instance_index)
            .collect()
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
    fn noninteractive_marks_are_excluded_from_scene_graph_rtree_but_not_bounds() {
        let rect = SceneRectMark {
            interactive: Some(false),
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
        assert!(pick(&rtree, [20.0, 30.0]).is_none());
    }

    #[test]
    fn marks_take_their_groups_interactive_setting_unless_they_set_their_own() {
        let scene = hit_scene(vec![SceneMark::Group(SceneGroup {
            interactive: Some(false),
            marks: vec![
                hit_rect("inherited", None),
                SceneMark::Group(SceneGroup {
                    origin: [40.0, 0.0],
                    marks: vec![hit_rect("explicit", None).with_interactive(true)],
                    ..Default::default()
                }),
            ],
            ..Default::default()
        })]);
        let rtree = SceneGraphRTree::from_scene_graph(&scene, &bundled_label_engine());

        assert!(pick(&rtree, [10.0, 10.0]).is_none());
        let hit = pick(&rtree, [50.0, 10.0]).expect("the explicitly interactive rect takes hits");
        assert_eq!(hit.mark_path, vec![0, 1, 0]);
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
        assert_eq!(pick(&tree, [4.0, 5.0]).unwrap().name, "clipped");
        assert_eq!(pick(&tree, [10.0, 10.0]).unwrap().name, "under");
        let mut path = lyon_path::Path::builder();
        path.begin(lyon_path::math::point(0.0, 0.0));
        path.line_to(lyon_path::math::point(10.0, 0.0));
        path.line_to(lyon_path::math::point(0.0, 10.0));
        path.close();
        let scene = hit_scene(vec![
            hit_rect("under", None),
            SceneGroup {
                clip: Clip::Path(path.build()),
                marks: vec![top],
                ..Default::default()
            }
            .into(),
        ]);
        let tree =
            SceneGraphRTree::from_scene_graph(&scene, &avenger_typst_label::bundled_label_engine());
        assert_eq!(pick(&tree, [2.0, 2.0]).unwrap().name, "clipped");
        assert_eq!(pick(&tree, [8.0, 8.0]).unwrap().name, "under");
    }

    #[test]
    fn top_pick_uses_global_z_index_before_scene_path() {
        let scene = hit_scene(vec![
            hit_rect("visually_top", Some(10)),
            hit_rect("later_but_under", Some(-10)),
        ]);
        let rtree = SceneGraphRTree::from_scene_graph(&scene, &bundled_label_engine());

        assert_eq!(
            pick(&rtree, [10.0, 10.0]).expect("overlapping rect").name,
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
            pick(&rtree, [10.0, 10.0])
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
            pick(&rtree, [10.0, 10.0])
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
            pick(&rtree, [10.0, 10.0])
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
        assert_eq!(pick(&tree, [5.0, 5.0]).unwrap().name, "probe");
    }

    /// An instance is found by its mark path and instance index, whether the tree was built with
    /// it or it was inserted later, among instances with the same index or the same box.
    #[test]
    fn instances_are_found_by_identity() {
        let mut tree = test_tree(vec![
            point_instance(vec![0], 0, 10.0, 10.0),
            point_instance(vec![0], 1, 10.0, 10.0),
            point_instance(vec![1], 0, 10.0, 10.0),
        ]);
        tree.insert(point_instance(vec![1], 1, 30.0, 30.0));
        let find = |path: Vec<usize>, index| {
            let mark = MarkInstance {
                name: "points".to_string(),
                mark_path: path,
                instance_index: Some(index),
            };
            tree.instance(&mark)
                .map(|found| found.mark_instance.clone())
        };
        for (path, index) in [(vec![0], 0), (vec![0], 1), (vec![1], 0), (vec![1], 1)] {
            let found = find(path.clone(), index).unwrap();
            assert_eq!((found.mark_path, found.instance_index), (path, Some(index)));
        }
        assert!(find(vec![0], 2).is_none());
        assert!(find(vec![2], 0).is_none());
    }

    /// The nearest instance is the closest one, however far the query point is: the search ranks
    /// subtrees by their boxes, which bound the distances inside them.
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
            let nearest = Nearest::new(query, f32::INFINITY);
            let found = tree.nearest(nearest).next().unwrap();
            let Geometry::Point(point) = &found.instance.geometry else {
                panic!("point instances");
            };
            assert_eq!(distance(&[point.x(), point.y()], &query), closest);
            assert_eq!(found.distance, closest);
        }
    }

    /// A query returns its matches topmost first, whatever their marks.
    #[test]
    fn queries_return_matches_topmost_first() {
        let tree = test_tree(vec![
            point_instance(vec![1], 2, 12.0, 12.0),
            point_instance(vec![0], 1, 10.0, 10.0),
            point_instance(vec![0], 0, 5.0, 5.0),
            point_instance(vec![0], 3, 30.0, 30.0),
        ]);
        let query = Query::new(
            QueryShape::Rect {
                min: [15.0, 15.0],
                max: [0.0, 0.0],
            },
            Relation::CoveredBy,
        );
        let ids = (tree.query(&query).iter())
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

    /// Circles and polygons select the instances whose centroids they cover.
    #[test]
    fn circles_and_polygons_cover_centroids() {
        let tree = test_tree(vec![
            point_instance(vec![0], 0, 8.0, 8.0),
            point_instance(vec![0], 1, 12.0, 10.0),
            point_instance(vec![0], 2, 18.0, 10.0),
        ]);
        let circle = QueryShape::Circle {
            center: [10.0, 10.0],
            radius: 3.0,
        };
        let query = Query::new(circle, Relation::CoveredBy).extent(Extent::Centroid);
        assert_eq!(indices(&tree, &query), [Some(1), Some(0)]);

        let tree = test_tree(vec![
            point_instance(vec![0], 0, 5.0, 5.0),
            point_instance(vec![0], 1, 12.0, 8.0),
            point_instance(vec![0], 2, 15.0, 18.0),
        ]);
        let triangle = QueryShape::Polygon {
            contours: vec![vec![[0.0, 0.0], [20.0, 0.0], [10.0, 20.0]]],
            fill_rule: FillRule::NonZero,
        };
        let query = Query::new(triangle, Relation::CoveredBy).extent(Extent::Centroid);
        assert_eq!(indices(&tree, &query), [Some(1), Some(0)]);
    }

    #[test]
    fn intersects_selects_marks_that_reach_into_the_shape() {
        let tree = test_tree(vec![
            rect_instance(vec![0], 0, 0.0, 0.0, 5.0, 5.0),
            rect_instance(vec![0], 1, 20.0, 20.0, 30.0, 30.0),
        ]);
        let query = Query::new(
            QueryShape::Rect {
                min: [4.0, 4.0],
                max: [10.0, 10.0],
            },
            Relation::Intersects,
        );
        assert_eq!(indices(&tree, &query), [Some(0)]);
    }
}
