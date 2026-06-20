use std::collections::HashMap;

use avenger_chart_core::AvengerChartError;
use datafusion::{
    arrow::record_batch::RecordBatch,
    common::{ScalarValue, utils::quote_identifier},
};
use indexmap::IndexMap;

use crate::coord::{
    HierarchyViewWindow, ROOT_PATH_ID, TreemapNode, TreemapPathComponent, TreemapPathLevel,
    TreemapRect, VisibleTreemapNode,
};

#[derive(Clone, Debug, PartialEq)]
pub struct HierarchyInputRow {
    pub path: Vec<TreemapPathComponent>,
    pub value: f64,
}

#[derive(Clone, Debug)]
pub struct HierarchyLayout {
    pub nodes: Vec<TreemapNode>,
    pub visible_nodes: Vec<VisibleTreemapNode>,
    pub breadcrumbs: Vec<TreemapNode>,
    index_by_path_id: HashMap<String, usize>,
}

impl HierarchyLayout {
    pub fn node(&self, path_id: &str) -> Option<&TreemapNode> {
        self.index_by_path_id
            .get(path_id)
            .and_then(|index| self.nodes.get(*index))
    }
}

#[derive(Clone, Debug)]
struct NodeBuilder {
    path_id: String,
    depth: usize,
    label: String,
    path: Vec<TreemapPathComponent>,
    value: f64,
    parent_path_id: Option<String>,
    child_path_ids: Vec<String>,
    is_data_leaf: bool,
}

pub fn collect_hierarchy_rows(
    batches: &[RecordBatch],
    levels: &[TreemapPathLevel],
) -> Result<Vec<HierarchyInputRow>, AvengerChartError> {
    let mut rows = Vec::new();
    for batch in batches {
        let path_columns = levels
            .iter()
            .map(|level| {
                batch.column_by_name(&level.name).ok_or_else(|| {
                    AvengerChartError::InternalError(format!(
                        "Treemap aggregate result is missing path column '{}'",
                        level.name
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let value_column = batch.column_by_name("__treemap_value").ok_or_else(|| {
            AvengerChartError::InternalError(
                "Treemap aggregate result is missing value column".to_string(),
            )
        })?;
        for row_index in 0..batch.num_rows() {
            let mut path = Vec::with_capacity(levels.len());
            for (level, column) in levels.iter().zip(path_columns.iter()) {
                let value = ScalarValue::try_from_array(column, row_index)
                    .map_err(AvengerChartError::DataFusionError)?;
                if scalar_is_null(&value) {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "Treemap path level '{}' contains null at aggregated row {row_index}",
                        level.name
                    )));
                }
                path.push(TreemapPathComponent {
                    name: level.name.clone(),
                    label: scalar_label(&value),
                    value,
                });
            }
            let value = ScalarValue::try_from_array(value_column, row_index)
                .map_err(AvengerChartError::DataFusionError)?;
            rows.push(HierarchyInputRow {
                path,
                value: scalar_to_f64(&value)?,
            });
        }
    }
    rows.sort_by(|left, right| path_sort_key(&left.path).cmp(&path_sort_key(&right.path)));
    Ok(rows)
}

pub fn build_hierarchy_layout(
    rows: Vec<HierarchyInputRow>,
    view_window: &HierarchyViewWindow,
    root_rect: TreemapRect,
) -> Result<HierarchyLayout, AvengerChartError> {
    if rows.is_empty() {
        return Err(AvengerChartError::InvalidArgument(
            "Treemap data produced no hierarchy rows".to_string(),
        ));
    }

    let mut nodes = IndexMap::<String, NodeBuilder>::new();
    nodes.insert(
        ROOT_PATH_ID.to_string(),
        NodeBuilder {
            path_id: ROOT_PATH_ID.to_string(),
            depth: 0,
            label: String::new(),
            path: Vec::new(),
            value: 0.0,
            parent_path_id: None,
            child_path_ids: Vec::new(),
            is_data_leaf: false,
        },
    );

    for row in rows {
        if row.value < 0.0 {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Treemap value for path '{}' is negative ({})",
                path_id_for_components(&row.path),
                row.value
            )));
        }
        let mut parent_id = ROOT_PATH_ID.to_string();
        for depth in 1..=row.path.len() {
            let prefix = row.path[..depth].to_vec();
            let path_id = path_id_for_components(&prefix);
            let label = prefix
                .last()
                .map(|component| component.label.clone())
                .unwrap_or_default();
            let is_leaf = depth == row.path.len();
            nodes.entry(path_id.clone()).or_insert_with(|| NodeBuilder {
                path_id: path_id.clone(),
                depth,
                label,
                path: prefix,
                value: 0.0,
                parent_path_id: Some(parent_id.clone()),
                child_path_ids: Vec::new(),
                is_data_leaf: false,
            });
            if let Some(parent) = nodes.get_mut(&parent_id)
                && !parent.child_path_ids.iter().any(|id| id == &path_id)
            {
                parent.child_path_ids.push(path_id.clone());
            }
            if is_leaf {
                let leaf = nodes.get_mut(&path_id).expect("leaf inserted");
                leaf.value += row.value;
                leaf.is_data_leaf = true;
            }
            parent_id = path_id;
        }
    }

    let path_by_id = nodes
        .iter()
        .map(|(id, node)| (id.clone(), path_sort_key(&node.path)))
        .collect::<HashMap<_, _>>();
    for node in nodes.values_mut() {
        node.child_path_ids
            .sort_by(|left, right| path_by_id.get(left).cmp(&path_by_id.get(right)));
    }

    let root_total = rollup_value(ROOT_PATH_ID, &mut nodes)?;
    if root_total <= 0.0 {
        return Err(AvengerChartError::InvalidArgument(
            "Treemap aggregate values must sum to a positive value".to_string(),
        ));
    }

    let mut node_vec = nodes
        .values()
        .map(|node| TreemapNode {
            path_id: node.path_id.clone(),
            depth: node.depth,
            label: node.label.clone(),
            path: node.path.clone(),
            value: node.value,
            parent_path_id: node.parent_path_id.clone(),
            child_path_ids: node.child_path_ids.clone(),
            is_data_leaf: node.is_data_leaf,
        })
        .collect::<Vec<_>>();
    node_vec.sort_by(|left, right| path_sort_key(&left.path).cmp(&path_sort_key(&right.path)));
    if let Some(root_index) = node_vec
        .iter()
        .position(|node| node.path_id == ROOT_PATH_ID)
    {
        node_vec.swap(0, root_index);
    }
    let index_by_path_id = node_vec
        .iter()
        .enumerate()
        .map(|(index, node)| (node.path_id.clone(), index))
        .collect::<HashMap<_, _>>();

    let root_id = view_window
        .root_path_id
        .clone()
        .unwrap_or_else(|| ROOT_PATH_ID.to_string());
    if !index_by_path_id.contains_key(&root_id) {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Treemap root_path_id '{root_id}' does not exist in the hierarchy"
        )));
    }

    let mut visible_nodes = Vec::new();
    let root_node = node_by_id(&node_vec, &index_by_path_id, &root_id)?;
    let view_depth = root_node.depth;
    visible_nodes.push(VisibleTreemapNode {
        node: root_node.clone(),
        rect: root_rect,
        view_depth,
        display_levels: view_window.display_levels,
        is_visible_leaf: root_node.child_path_ids.is_empty() || view_window.display_levels == 0,
        has_hidden_descendants: !root_node.child_path_ids.is_empty()
            && view_window.display_levels == 0,
        can_zoom: root_id != ROOT_PATH_ID,
    });
    layout_visible_children(
        &node_vec,
        &index_by_path_id,
        &mut visible_nodes,
        &root_id,
        root_rect,
        view_depth,
        view_window.display_levels,
        0,
    )?;

    let breadcrumbs = breadcrumb_nodes(&node_vec, &index_by_path_id, &root_id)?;
    Ok(HierarchyLayout {
        nodes: node_vec,
        visible_nodes,
        breadcrumbs,
        index_by_path_id,
    })
}

fn rollup_value(
    path_id: &str,
    nodes: &mut IndexMap<String, NodeBuilder>,
) -> Result<f64, AvengerChartError> {
    let children = nodes
        .get(path_id)
        .ok_or_else(|| AvengerChartError::InternalError(format!("Missing treemap node {path_id}")))?
        .child_path_ids
        .clone();
    if children.is_empty() {
        return Ok(nodes
            .get(path_id)
            .ok_or_else(|| {
                AvengerChartError::InternalError(format!("Missing treemap node {path_id}"))
            })?
            .value);
    }
    let mut total = 0.0;
    for child in children {
        total += rollup_value(&child, nodes)?;
    }
    let node = nodes.get_mut(path_id).ok_or_else(|| {
        AvengerChartError::InternalError(format!("Missing treemap node {path_id}"))
    })?;
    node.value += total;
    Ok(node.value)
}

fn layout_visible_children(
    nodes: &[TreemapNode],
    index_by_path_id: &HashMap<String, usize>,
    out: &mut Vec<VisibleTreemapNode>,
    parent_id: &str,
    parent_rect: TreemapRect,
    view_depth: usize,
    display_levels: usize,
    relative_depth: usize,
) -> Result<(), AvengerChartError> {
    if relative_depth >= display_levels {
        return Ok(());
    }
    let parent = node_by_id(nodes, index_by_path_id, parent_id)?;
    if parent.child_path_ids.is_empty() {
        return Ok(());
    }
    let children = parent
        .child_path_ids
        .iter()
        .map(|id| node_by_id(nodes, index_by_path_id, id))
        .collect::<Result<Vec<_>, _>>()?;
    let rects = slice_dice_rects(parent_rect, &children, relative_depth);
    for (child, rect) in children.into_iter().zip(rects) {
        let child_relative_depth = relative_depth + 1;
        let has_hidden_descendants =
            !child.child_path_ids.is_empty() && child_relative_depth >= display_levels;
        let is_visible_leaf = child.child_path_ids.is_empty() || has_hidden_descendants;
        out.push(VisibleTreemapNode {
            node: child.clone(),
            rect,
            view_depth,
            display_levels,
            is_visible_leaf,
            has_hidden_descendants,
            can_zoom: !child.child_path_ids.is_empty(),
        });
        if !is_visible_leaf {
            layout_visible_children(
                nodes,
                index_by_path_id,
                out,
                &child.path_id,
                rect,
                view_depth,
                display_levels,
                child_relative_depth,
            )?;
        }
    }
    Ok(())
}

fn slice_dice_rects(
    parent: TreemapRect,
    children: &[&TreemapNode],
    depth: usize,
) -> Vec<TreemapRect> {
    let total = children
        .iter()
        .map(|child| child.value.max(0.0))
        .sum::<f64>();
    if total <= 0.0 {
        return children
            .iter()
            .map(|_| TreemapRect::new(parent.x, parent.y, 0.0, 0.0))
            .collect();
    }

    let split_x = depth % 2 == 0;
    let mut cursor = if split_x { parent.x } else { parent.y };
    let mut rects = Vec::with_capacity(children.len());
    for (index, child) in children.iter().enumerate() {
        let fraction = (child.value.max(0.0) / total) as f32;
        let is_last = index + 1 == children.len();
        if split_x {
            let width = if is_last {
                parent.x + parent.width - cursor
            } else {
                parent.width * fraction
            };
            rects.push(TreemapRect::new(
                cursor,
                parent.y,
                width.max(0.0),
                parent.height,
            ));
            cursor += width;
        } else {
            let height = if is_last {
                parent.y + parent.height - cursor
            } else {
                parent.height * fraction
            };
            rects.push(TreemapRect::new(
                parent.x,
                cursor,
                parent.width,
                height.max(0.0),
            ));
            cursor += height;
        }
    }
    rects
}

fn breadcrumb_nodes(
    nodes: &[TreemapNode],
    index_by_path_id: &HashMap<String, usize>,
    root_id: &str,
) -> Result<Vec<TreemapNode>, AvengerChartError> {
    let mut breadcrumbs = vec![node_by_id(nodes, index_by_path_id, ROOT_PATH_ID)?.clone()];
    if root_id == ROOT_PATH_ID {
        return Ok(breadcrumbs);
    }
    let node = node_by_id(nodes, index_by_path_id, root_id)?;
    for depth in 1..=node.path.len() {
        let id = path_id_for_components(&node.path[..depth]);
        breadcrumbs.push(node_by_id(nodes, index_by_path_id, &id)?.clone());
    }
    Ok(breadcrumbs)
}

fn node_by_id<'a>(
    nodes: &'a [TreemapNode],
    index_by_path_id: &HashMap<String, usize>,
    path_id: &str,
) -> Result<&'a TreemapNode, AvengerChartError> {
    index_by_path_id
        .get(path_id)
        .and_then(|index| nodes.get(*index))
        .ok_or_else(|| AvengerChartError::InternalError(format!("Missing treemap node {path_id}")))
}

pub fn path_id_for_components(path: &[TreemapPathComponent]) -> String {
    if path.is_empty() {
        return ROOT_PATH_ID.to_string();
    }
    path.iter()
        .map(|component| {
            format!(
                "{}={}",
                escape_path_part(&component.name),
                escape_path_part(&component.label)
            )
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn path_sort_key(path: &[TreemapPathComponent]) -> Vec<String> {
    path.iter()
        .flat_map(|component| [component.name.clone(), component.label.clone()])
        .collect()
}

fn escape_path_part(value: &str) -> String {
    value.replace('\\', "\\\\").replace('/', "\\/")
}

fn scalar_is_null(value: &ScalarValue) -> bool {
    matches!(
        value,
        ScalarValue::Null
            | ScalarValue::Utf8(None)
            | ScalarValue::LargeUtf8(None)
            | ScalarValue::Utf8View(None)
            | ScalarValue::Boolean(None)
            | ScalarValue::Float16(None)
            | ScalarValue::Float32(None)
            | ScalarValue::Float64(None)
            | ScalarValue::Int8(None)
            | ScalarValue::Int16(None)
            | ScalarValue::Int32(None)
            | ScalarValue::Int64(None)
            | ScalarValue::UInt8(None)
            | ScalarValue::UInt16(None)
            | ScalarValue::UInt32(None)
            | ScalarValue::UInt64(None)
    )
}

fn scalar_label(value: &ScalarValue) -> String {
    match value {
        ScalarValue::Utf8(Some(value))
        | ScalarValue::LargeUtf8(Some(value))
        | ScalarValue::Utf8View(Some(value)) => value.clone(),
        other => other.to_string(),
    }
}

fn scalar_to_f64(value: &ScalarValue) -> Result<f64, AvengerChartError> {
    match value {
        ScalarValue::Float64(Some(value)) => Ok(*value),
        ScalarValue::Float32(Some(value)) => Ok(*value as f64),
        ScalarValue::Int8(Some(value)) => Ok(*value as f64),
        ScalarValue::Int16(Some(value)) => Ok(*value as f64),
        ScalarValue::Int32(Some(value)) => Ok(*value as f64),
        ScalarValue::Int64(Some(value)) => Ok(*value as f64),
        ScalarValue::UInt8(Some(value)) => Ok(*value as f64),
        ScalarValue::UInt16(Some(value)) => Ok(*value as f64),
        ScalarValue::UInt32(Some(value)) => Ok(*value as f64),
        ScalarValue::UInt64(Some(value)) => Ok(*value as f64),
        ScalarValue::Null
        | ScalarValue::Float64(None)
        | ScalarValue::Float32(None)
        | ScalarValue::Int8(None)
        | ScalarValue::Int16(None)
        | ScalarValue::Int32(None)
        | ScalarValue::Int64(None)
        | ScalarValue::UInt8(None)
        | ScalarValue::UInt16(None)
        | ScalarValue::UInt32(None)
        | ScalarValue::UInt64(None) => Err(AvengerChartError::InvalidArgument(
            "Treemap aggregate value is null".to_string(),
        )),
        other => Err(AvengerChartError::InvalidArgument(format!(
            "Treemap aggregate value must be numeric, got {}",
            quote_identifier(&other.to_string())
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn component(name: &str, label: &str) -> TreemapPathComponent {
        TreemapPathComponent {
            name: name.to_string(),
            value: ScalarValue::Utf8(Some(label.to_string())),
            label: label.to_string(),
        }
    }

    fn row(path: &[(&str, &str)], value: f64) -> HierarchyInputRow {
        HierarchyInputRow {
            path: path
                .iter()
                .map(|(name, value)| component(name, value))
                .collect(),
            value,
        }
    }

    #[test]
    fn hierarchy_rolls_leaf_values_up_to_parents() {
        let layout = build_hierarchy_layout(
            vec![
                row(&[("region", "East"), ("product", "A")], 2.0),
                row(&[("region", "East"), ("product", "B")], 3.0),
                row(&[("region", "West"), ("product", "A")], 5.0),
            ],
            &HierarchyViewWindow::default(),
            TreemapRect::new(0.0, 0.0, 100.0, 50.0),
        )
        .unwrap();

        assert_eq!(layout.node(ROOT_PATH_ID).unwrap().value, 10.0);
        assert_eq!(layout.node("region=East").unwrap().value, 5.0);
        assert_eq!(layout.node("region=West").unwrap().value, 5.0);
        assert_eq!(layout.node("region=East/product=A").unwrap().value, 2.0);
    }

    #[test]
    fn path_ids_and_sibling_order_are_deterministic() {
        let first = build_hierarchy_layout(
            vec![
                row(&[("region", "West"), ("product", "B")], 3.0),
                row(&[("region", "East"), ("product", "A")], 2.0),
            ],
            &HierarchyViewWindow::default(),
            TreemapRect::new(0.0, 0.0, 100.0, 50.0),
        )
        .unwrap();
        let second = build_hierarchy_layout(
            vec![
                row(&[("region", "East"), ("product", "A")], 2.0),
                row(&[("region", "West"), ("product", "B")], 3.0),
            ],
            &HierarchyViewWindow::default(),
            TreemapRect::new(0.0, 0.0, 100.0, 50.0),
        )
        .unwrap();

        let ids = |layout: &HierarchyLayout| {
            layout
                .nodes
                .iter()
                .map(|node| node.path_id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&first), ids(&second));
        assert_eq!(
            first.node(ROOT_PATH_ID).unwrap().child_path_ids,
            vec!["region=East".to_string(), "region=West".to_string()]
        );
    }

    #[test]
    fn slice_dice_cells_cover_root_without_overlap() {
        let layout = build_hierarchy_layout(
            vec![
                row(&[("region", "East")], 2.0),
                row(&[("region", "West")], 3.0),
            ],
            &HierarchyViewWindow::default(),
            TreemapRect::new(0.0, 0.0, 100.0, 50.0),
        )
        .unwrap();
        let terminals = layout
            .visible_nodes
            .iter()
            .filter(|node| node.node.depth == 1)
            .collect::<Vec<_>>();
        assert_eq!(terminals.len(), 2);
        assert_eq!(terminals[0].rect, TreemapRect::new(0.0, 0.0, 40.0, 50.0));
        assert_eq!(terminals[1].rect, TreemapRect::new(40.0, 0.0, 60.0, 50.0));
        let area = terminals.iter().map(|node| node.rect.area()).sum::<f32>();
        assert!((area - 5000.0).abs() < 0.01);
    }

    #[test]
    fn display_levels_marks_deeper_nodes_as_collapsed_visible_terminals() {
        let layout = build_hierarchy_layout(
            vec![
                row(&[("division", "D1"), ("team", "T1"), ("person", "P1")], 4.0),
                row(&[("division", "D1"), ("team", "T2"), ("person", "P2")], 6.0),
            ],
            &HierarchyViewWindow::new().display_levels(1),
            TreemapRect::new(0.0, 0.0, 100.0, 100.0),
        )
        .unwrap();
        let d1 = layout
            .visible_nodes
            .iter()
            .find(|node| node.node.path_id == "division=D1")
            .unwrap();
        assert!(d1.is_visible_leaf);
        assert!(d1.has_hidden_descendants);
        assert!(d1.can_zoom);
        assert_eq!(d1.node.value, 10.0);
    }

    #[test]
    fn root_path_id_sets_view_depth_and_breadcrumbs() {
        let layout = build_hierarchy_layout(
            vec![
                row(&[("division", "D1"), ("team", "T1"), ("person", "P1")], 4.0),
                row(&[("division", "D1"), ("team", "T2"), ("person", "P2")], 6.0),
            ],
            &HierarchyViewWindow::new()
                .root_path_id("division=D1")
                .display_levels(1),
            TreemapRect::new(0.0, 0.0, 100.0, 100.0),
        )
        .unwrap();

        assert_eq!(layout.visible_nodes[0].node.path_id, "division=D1");
        assert_eq!(layout.visible_nodes[0].view_depth, 1);
        assert_eq!(
            layout
                .breadcrumbs
                .iter()
                .map(|node| node.path_id.as_str())
                .collect::<Vec<_>>(),
            vec![ROOT_PATH_ID, "division=D1"]
        );
    }

    #[test]
    fn invalid_root_path_id_errors() {
        let err = build_hierarchy_layout(
            vec![row(&[("region", "East")], 2.0)],
            &HierarchyViewWindow::new().root_path_id("region=West"),
            TreemapRect::new(0.0, 0.0, 100.0, 50.0),
        )
        .unwrap_err();
        assert!(err.to_string().contains("root_path_id"));
    }

    #[test]
    fn negative_values_error() {
        let err = build_hierarchy_layout(
            vec![row(&[("region", "East")], -2.0)],
            &HierarchyViewWindow::default(),
            TreemapRect::new(0.0, 0.0, 100.0, 50.0),
        )
        .unwrap_err();
        assert!(err.to_string().contains("negative"));
    }

    #[test]
    fn all_zero_values_error() {
        let err = build_hierarchy_layout(
            vec![row(&[("region", "East")], 0.0)],
            &HierarchyViewWindow::default(),
            TreemapRect::new(0.0, 0.0, 100.0, 50.0),
        )
        .unwrap_err();
        assert!(err.to_string().contains("positive value"));
    }
}
