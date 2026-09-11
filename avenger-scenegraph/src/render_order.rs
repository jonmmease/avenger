use crate::{
    marks::{
        arc::SceneArcMark,
        area::SceneAreaMark,
        group::{Clip, SceneGroup},
        line::SceneLineMark,
        mark::SceneMark,
        path::ScenePathMark,
        pattern::PatternReferenceFrame,
        rect::SceneRectMark,
        rule::SceneRuleMark,
        symbol::SceneSymbolMark,
        trail::SceneTrailMark,
    },
    scene_graph::SceneGraph,
};

/// A renderer-independent display list built from a scene graph.
#[derive(Debug, Clone)]
pub struct SceneDisplayList<'a> {
    pub width: f32,
    pub height: f32,
    pub origin: [f32; 2],
    pub items: Vec<SceneDisplayItem<'a>>,
}

/// One drawable display-list item in document order.
#[derive(Debug, Clone)]
pub struct SceneDisplayItem<'a> {
    pub mark: SceneDisplayMark<'a>,
    pub zindex: i32,
    pub origin: [f32; 2],
    pub clip: Clip,
    pub pattern_reference_frame: Option<PatternReferenceFrame>,
    pub mark_path: Vec<usize>,
}

/// Display-list item mark data.
#[derive(Debug, Clone)]
#[allow(
    clippy::large_enum_variant,
    reason = "Keep temporary display paths inline instead of allocating once per group."
)]
pub enum SceneDisplayMark<'a> {
    Borrowed(&'a SceneMark),
    OwnedGroupPath(ScenePathMark),
}

impl<'a> SceneDisplayList<'a> {
    pub fn from_scene_graph(scene_graph: &'a SceneGraph) -> Self {
        let mut list = Self {
            width: scene_graph.width,
            height: scene_graph.height,
            origin: scene_graph.origin,
            items: Vec::new(),
        };

        for (index, mark) in scene_graph.children().iter().enumerate() {
            list.push_mark(mark, scene_graph.origin, &Clip::None, None, 0, vec![index]);
        }

        list
    }

    /// Return display items in final z-index layer order.
    ///
    /// Items are stored in document order so renderers can batch collection in the
    /// same sequence as the source scenegraph. This helper applies the shared
    /// z-index layer algorithm for final drawing.
    pub fn ordered_items(&self) -> Vec<&SceneDisplayItem<'a>> {
        let zindices = self
            .items
            .iter()
            .map(|item| item.zindex)
            .collect::<Vec<_>>();
        let layers = compute_zindex_layers(zindices);
        let mut ordered = Vec::with_capacity(self.items.len());

        for (min_z, max_z) in layers {
            for item in &self.items {
                if item.zindex >= min_z && item.zindex <= max_z {
                    ordered.push(item);
                }
            }
        }

        ordered
    }

    fn push_mark(
        &mut self,
        mark: &'a SceneMark,
        parent_origin: [f32; 2],
        parent_clip: &Clip,
        parent_pattern_reference_frame: Option<&PatternReferenceFrame>,
        parent_zindex: i32,
        mark_path: Vec<usize>,
    ) {
        match mark {
            SceneMark::Group(group) => {
                self.push_group(
                    group,
                    parent_origin,
                    parent_clip,
                    parent_pattern_reference_frame,
                    parent_zindex,
                    mark_path,
                );
            }
            _ => {
                let zindex = mark.zindex().unwrap_or(parent_zindex);
                self.items.push(SceneDisplayItem {
                    mark: SceneDisplayMark::Borrowed(mark),
                    zindex,
                    origin: parent_origin,
                    clip: parent_clip.maybe_clip(mark_clip_enabled(mark)),
                    pattern_reference_frame: parent_pattern_reference_frame.cloned(),
                    mark_path,
                });
            }
        }
    }

    fn push_group(
        &mut self,
        group: &'a SceneGroup,
        parent_origin: [f32; 2],
        parent_clip: &Clip,
        parent_pattern_reference_frame: Option<&PatternReferenceFrame>,
        parent_zindex: i32,
        mark_path: Vec<usize>,
    ) {
        let group_zindex = group.zindex.unwrap_or(parent_zindex);
        let origin = [
            parent_origin[0] + group.origin[0],
            parent_origin[1] + group.origin[1],
        ];
        let pattern_reference_frame = group
            .pattern_reference_frame
            .as_ref()
            .map(|frame| frame.translated(origin[0], origin[1]))
            .or_else(|| parent_pattern_reference_frame.cloned());

        if let Some(path_mark) = group.make_path_mark() {
            self.items.push(SceneDisplayItem {
                mark: SceneDisplayMark::OwnedGroupPath(path_mark),
                zindex: group_zindex,
                origin: parent_origin,
                clip: Clip::None,
                pattern_reference_frame: pattern_reference_frame.clone(),
                mark_path: mark_path.clone(),
            });
        }

        let clip = if matches!(&group.clip, Clip::None) {
            parent_clip.clone()
        } else {
            group.clip.translate(origin[0], origin[1])
        };

        for (index, mark) in group.marks.iter().enumerate() {
            let mut child_path = mark_path.clone();
            child_path.push(index);
            self.push_mark(
                mark,
                origin,
                &clip,
                pattern_reference_frame.as_ref(),
                group_zindex,
                child_path,
            );
        }
    }
}

fn mark_clip_enabled(mark: &SceneMark) -> bool {
    match mark {
        SceneMark::Arc(SceneArcMark { clip, .. })
        | SceneMark::Area(SceneAreaMark { clip, .. })
        | SceneMark::Path(ScenePathMark { clip, .. })
        | SceneMark::Symbol(SceneSymbolMark { clip, .. })
        | SceneMark::Line(SceneLineMark { clip, .. })
        | SceneMark::Trail(SceneTrailMark { clip, .. })
        | SceneMark::Rect(SceneRectMark { clip, .. })
        | SceneMark::Rule(SceneRuleMark { clip, .. }) => *clip,
        SceneMark::Text(mark) => mark.clip,
        SceneMark::Image(mark) => mark.clip,
        SceneMark::WarpedImage(mark) => mark.clip,
        SceneMark::Group(_) => false,
    }
}

/// Compute minimal z-index layers from a sequence of z-indices.
///
/// Given z-indices in document order, returns non-overlapping `(min, max)`
/// ranges that preserve both z-index ordering and document order.
pub fn compute_zindex_layers(z_indices: Vec<i32>) -> Vec<(i32, i32)> {
    if z_indices.is_empty() {
        return vec![];
    }

    let mut indices: Vec<usize> = (0..z_indices.len()).collect();
    indices.sort_by_key(|&i| z_indices[i]);

    let mut partitions = Vec::new();
    let mut current_partition = vec![indices[0]];
    let mut max_index_in_partition = indices[0];

    for &idx in &indices[1..] {
        if idx > max_index_in_partition {
            current_partition.push(idx);
            max_index_in_partition = idx;
        } else {
            partitions.push(current_partition);
            current_partition = vec![idx];
            max_index_in_partition = idx;
        }
    }

    partitions.push(current_partition);

    partitions
        .into_iter()
        .map(|partition| {
            let z_values: Vec<i32> = partition.iter().map(|&i| z_indices[i]).collect();
            let min_z = *z_values.iter().min().unwrap();
            let max_z = *z_values.iter().max().unwrap();
            (min_z, max_z)
        })
        .collect()
}

/// Verify that a partition set preserves z-index and document-order semantics.
pub fn verify_partitions(z_indices: &[i32], partitions: &[(i32, i32)]) -> bool {
    for i in 1..partitions.len() {
        if partitions[i - 1].1 >= partitions[i].0 {
            return false;
        }
    }

    for &(min_z, max_z) in partitions {
        let mut last_pos = None;
        for (pos, &z) in z_indices.iter().enumerate() {
            if z >= min_z && z <= max_z {
                if let Some(prev_pos) = last_pos {
                    if z < z_indices[prev_pos] {
                        return false;
                    }
                }
                last_pos = Some(pos);
            }
        }
    }

    let mut found = vec![false; z_indices.len()];
    for &(min_z, max_z) in partitions {
        for (pos, &z) in z_indices.iter().enumerate() {
            if z >= min_z && z <= max_z {
                if found[pos] {
                    return false;
                }
                found[pos] = true;
            }
        }
    }

    found.iter().all(|&f| f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_color::ColorOrGradient;

    fn scene_graph(marks: Vec<SceneMark>) -> SceneGraph {
        SceneGraph {
            marks,
            width: 100.0,
            height: 80.0,
            origin: [10.0, 20.0],
        }
    }

    fn rect(name: &str, zindex: Option<i32>, clip: bool) -> SceneMark {
        SceneRectMark {
            name: name.to_string(),
            zindex,
            clip,
            ..Default::default()
        }
        .into()
    }

    fn group(
        name: &str,
        origin: [f32; 2],
        clip: Clip,
        zindex: Option<i32>,
        marks: Vec<SceneMark>,
    ) -> SceneMark {
        SceneGroup {
            name: name.to_string(),
            origin,
            clip,
            zindex,
            marks,
            ..Default::default()
        }
        .into()
    }

    #[test]
    fn display_list_includes_top_level_non_group_marks() {
        let graph = scene_graph(vec![
            rect("root_rect", Some(2), true),
            group(
                "group",
                [5.0, 7.0],
                Clip::None,
                None,
                vec![rect("child_rect", None, true)],
            ),
        ]);

        let list = SceneDisplayList::from_scene_graph(&graph);

        assert_eq!(list.items.len(), 2);
        assert_eq!(list.items[0].mark_path, vec![0]);
        assert_eq!(list.items[0].origin, [10.0, 20.0]);
        assert_eq!(list.items[0].zindex, 2);
        assert!(matches!(list.items[0].mark, SceneDisplayMark::Borrowed(_)));

        assert_eq!(list.items[1].mark_path, vec![1, 0]);
        assert_eq!(list.items[1].origin, [15.0, 27.0]);
        assert_eq!(list.items[1].zindex, 0);
    }

    #[test]
    fn display_list_accumulates_group_origins() {
        let graph = scene_graph(vec![group(
            "outer",
            [5.0, 7.0],
            Clip::None,
            None,
            vec![group(
                "inner",
                [2.0, 3.0],
                Clip::None,
                None,
                vec![rect("child", None, true)],
            )],
        )]);

        let list = SceneDisplayList::from_scene_graph(&graph);

        assert_eq!(list.items.len(), 1);
        assert_eq!(list.items[0].mark_path, vec![0, 0, 0]);
        assert_eq!(list.items[0].origin, [17.0, 30.0]);
    }

    #[test]
    fn display_list_applies_zindex_inheritance() {
        let graph = scene_graph(vec![group(
            "group",
            [0.0, 0.0],
            Clip::None,
            Some(3),
            vec![
                rect("inherited", None, true),
                rect("explicit", Some(5), true),
                group(
                    "nested",
                    [0.0, 0.0],
                    Clip::None,
                    None,
                    vec![rect("nested_child", None, true)],
                ),
            ],
        )]);

        let list = SceneDisplayList::from_scene_graph(&graph);
        let zindices = list
            .items
            .iter()
            .map(|item| item.zindex)
            .collect::<Vec<_>>();

        assert_eq!(zindices, vec![3, 5, 3]);
    }

    #[test]
    fn display_list_inserts_group_fill_stroke_path_before_children() {
        let graph = scene_graph(vec![SceneGroup {
            name: "filled_group".to_string(),
            origin: [5.0, 7.0],
            clip: Clip::Rect {
                x: 1.0,
                y: 2.0,
                width: 10.0,
                height: 12.0,
            },
            fill: Some(ColorOrGradient::Color([1.0, 0.0, 0.0, 1.0])),
            zindex: Some(4),
            marks: vec![rect("child", None, true)],
            ..Default::default()
        }
        .into()]);

        let list = SceneDisplayList::from_scene_graph(&graph);

        assert_eq!(list.items.len(), 2);
        assert!(matches!(
            list.items[0].mark,
            SceneDisplayMark::OwnedGroupPath(_)
        ));
        assert_eq!(list.items[0].zindex, 4);
        assert_eq!(list.items[0].origin, [10.0, 20.0]);
        assert_eq!(list.items[0].clip, Clip::None);
        assert_eq!(list.items[0].mark_path, vec![0]);
        assert_eq!(list.items[1].mark_path, vec![0, 0]);
    }

    #[test]
    fn display_list_handles_clip_inheritance_replacement_and_mark_clip_flags() {
        let graph = scene_graph(vec![group(
            "outer",
            [5.0, 5.0],
            Clip::Rect {
                x: 1.0,
                y: 2.0,
                width: 30.0,
                height: 40.0,
            },
            None,
            vec![
                rect("clipped", None, true),
                rect("unclipped", None, false),
                group(
                    "inner_inherit",
                    [2.0, 3.0],
                    Clip::None,
                    None,
                    vec![rect("inherited_clip", None, true)],
                ),
                group(
                    "inner_replace",
                    [4.0, 5.0],
                    Clip::Rect {
                        x: 3.0,
                        y: 4.0,
                        width: 5.0,
                        height: 6.0,
                    },
                    None,
                    vec![rect("replaced_clip", None, true)],
                ),
            ],
        )]);

        let list = SceneDisplayList::from_scene_graph(&graph);

        assert_eq!(
            list.items[0].clip,
            Clip::Rect {
                x: 16.0,
                y: 27.0,
                width: 30.0,
                height: 40.0,
            }
        );
        assert_eq!(list.items[1].clip, Clip::None);
        assert_eq!(
            list.items[2].clip,
            Clip::Rect {
                x: 16.0,
                y: 27.0,
                width: 30.0,
                height: 40.0,
            }
        );
        assert_eq!(
            list.items[3].clip,
            Clip::Rect {
                x: 22.0,
                y: 34.0,
                width: 5.0,
                height: 6.0,
            }
        );
    }

    #[test]
    fn display_list_inherits_pattern_reference_frame() {
        let reference_frame = PatternReferenceFrame {
            x: 10.0,
            y: 20.0,
            width: 30.0,
            height: 40.0,
        };
        let graph = scene_graph(vec![group(
            "outer",
            [5.0, 7.0],
            Clip::None,
            None,
            vec![group(
                "inner",
                [2.0, 3.0],
                Clip::None,
                None,
                vec![rect("child", None, true)],
            )],
        )]);
        let SceneMark::Group(mut outer) = graph.marks[0].clone() else {
            panic!("expected group");
        };
        outer.pattern_reference_frame = Some(reference_frame.clone());
        let graph = scene_graph(vec![outer.into()]);

        let list = SceneDisplayList::from_scene_graph(&graph);

        assert_eq!(list.items.len(), 1);
        assert_eq!(
            list.items[0].pattern_reference_frame,
            Some(PatternReferenceFrame {
                x: 25.0,
                y: 47.0,
                width: reference_frame.width,
                height: reference_frame.height,
            })
        );
    }

    #[test]
    fn ordered_items_applies_zindex_layers() {
        let graph = scene_graph(vec![
            rect("z0", Some(0), true),
            rect("z2", Some(2), true),
            rect("zneg1", Some(-1), true),
            rect("z3", Some(3), true),
            rect("z1", Some(1), true),
            rect("z4", Some(4), true),
        ]);
        let list = SceneDisplayList::from_scene_graph(&graph);
        let ordered_zindices = list
            .ordered_items()
            .iter()
            .map(|item| item.zindex)
            .collect::<Vec<_>>();

        assert_eq!(ordered_zindices, vec![-1, 0, 1, 2, 3, 4]);
    }

    #[test]
    fn test_empty_input() {
        let result = compute_zindex_layers(vec![]);
        assert_eq!(result, vec![]);
    }

    #[test]
    fn test_single_element() {
        let result = compute_zindex_layers(vec![5]);
        assert_eq!(result, vec![(5, 5)]);
    }

    #[test]
    fn test_ascending_order() {
        let result = compute_zindex_layers(vec![1, 2, 3, 4, 5]);
        assert_eq!(result, vec![(1, 5)]);
    }

    #[test]
    fn test_descending_order() {
        let result = compute_zindex_layers(vec![5, 4, 3, 2, 1]);
        assert_eq!(result, vec![(1, 1), (2, 2), (3, 3), (4, 4), (5, 5)]);
    }

    #[test]
    fn test_example_from_spec() {
        let z_indices = vec![0, 2, -1, 3, 1, 4];
        let result = compute_zindex_layers(z_indices.clone());
        let expected = vec![(-1, -1), (0, 1), (2, 4)];

        assert_eq!(result, expected);
        assert!(verify_partitions(&z_indices, &result));
    }

    #[test]
    fn test_duplicates() {
        let z_indices = vec![1, 2, 2, 3, 1, 4];
        let result = compute_zindex_layers(z_indices.clone());
        let expected = vec![(1, 1), (2, 4)];

        assert_eq!(result, expected);
        assert!(verify_partitions(&z_indices, &result));
    }

    #[test]
    fn test_negative_values() {
        let z_indices = vec![-2, -1, 0, 1, -3, 2];
        let result = compute_zindex_layers(z_indices.clone());
        let expected = vec![(-3, -3), (-2, 2)];

        assert_eq!(result, expected);
        assert!(verify_partitions(&z_indices, &result));
    }

    #[test]
    fn test_alternating_pattern() {
        let z_indices = vec![1, 10, 2, 9, 3, 8];
        let result = compute_zindex_layers(z_indices.clone());
        let expected = vec![(1, 8), (9, 9), (10, 10)];

        assert_eq!(result, expected);
        assert!(verify_partitions(&z_indices, &result));
    }

    #[test]
    fn test_multiple_inversions() {
        let z_indices = vec![5, 3, 7, 1, 9, 2];
        let result = compute_zindex_layers(z_indices.clone());
        let expected = vec![(1, 2), (3, 3), (5, 9)];

        assert_eq!(result, expected);
        assert!(verify_partitions(&z_indices, &result));
    }

    #[test]
    fn test_complex_case() {
        let z_indices = vec![0, 5, 3, 8, 2, 6, 1, 7, 4, 9];
        let result = compute_zindex_layers(z_indices.clone());

        assert!(verify_partitions(&z_indices, &result));

        for i in 1..result.len() {
            assert!(
                result[i - 1].1 < result[i].0,
                "Layers should be non-overlapping: {:?} and {:?}",
                result[i - 1],
                result[i]
            );
        }
    }

    #[test]
    fn test_large_range() {
        let z_indices = vec![1000, 2000, 500, 3000];
        let result = compute_zindex_layers(z_indices.clone());
        let expected = vec![(500, 500), (1000, 3000)];

        assert_eq!(result, expected);
        assert!(verify_partitions(&z_indices, &result));
    }

    #[test]
    fn test_all_same() {
        let result = compute_zindex_layers(vec![5, 5, 5, 5]);
        assert_eq!(result, vec![(5, 5)]);
    }

    #[test]
    fn test_two_groups() {
        let z_indices = vec![1, 2, 3, 10, 11, 12];
        let result = compute_zindex_layers(z_indices.clone());
        let expected = vec![(1, 12)];

        assert_eq!(result, expected);
        assert!(verify_partitions(&z_indices, &result));
    }

    #[test]
    fn test_three_partitions() {
        let z_indices = vec![1, 5, 2, 6, 3, 7, 4];
        let result = compute_zindex_layers(z_indices.clone());

        assert!(verify_partitions(&z_indices, &result));
        assert_eq!(result, vec![(1, 4), (5, 7)]);
    }

    #[test]
    fn test_duplicate_values_across_positions() {
        let z_indices = vec![2, 1, 2, 1, 2];
        let result = compute_zindex_layers(z_indices.clone());

        assert!(verify_partitions(&z_indices, &result));
        assert_eq!(result, vec![(1, 1), (2, 2)]);
    }

    #[test]
    fn test_verify_invalid_overlapping_partitions() {
        let z_indices = vec![1, 2, 3];
        let partitions = vec![(1, 2), (2, 3)];
        assert!(!verify_partitions(&z_indices, &partitions));
    }

    #[test]
    fn test_verify_invalid_missing_value() {
        let z_indices = vec![1, 2, 3];
        let partitions = vec![(1, 1), (3, 3)];
        assert!(!verify_partitions(&z_indices, &partitions));
    }

    #[test]
    fn test_verify_invalid_order_within_partition() {
        let z_indices = vec![3, 1, 2];
        let partitions = vec![(1, 3)];
        assert!(!verify_partitions(&z_indices, &partitions));
    }
}
