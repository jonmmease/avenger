//! The composition functions: `layer`, which overlays the plots of charts, `hconcat` and
//! `vconcat`, which concatenate charts, and `facet_row`, `facet_col`, and `facet_wrap`, which lay
//! out charts by the values of a key. A binding collects the charts of the rows of a group, with
//! their keys, and composes them with [`Composition::compose`].

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use arrow::{
    array::{Array, ArrayRef},
    compute::{concat, SortOptions},
    row::{RowConverter, SortField},
};

use super::{generated, Arg, ArgError, Param};
use crate::{
    builder::Value,
    proto::{self, node::Node, Direction, NodeBuilder, PartitionValue},
    Chart,
};

/// The kind of a composition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CompositionKind {
    Layer,
    HConcat,
    VConcat,
    FacetRow,
    FacetCol,
    FacetWrap,
}

impl CompositionKind {
    /// Whether the composition takes a key: the facets do.
    pub fn takes_key(self) -> bool {
        matches!(self, Self::FacetRow | Self::FacetCol | Self::FacetWrap)
    }
}

/// A composition function: its name, its kind, its documentation, and its parameters, the options
/// of its node. A facet also takes a key, which is not one of its parameters.
pub struct Composition {
    pub name: &'static str,
    pub kind: CompositionKind,
    pub doc: &'static str,
    pub params: &'static [Param],
}

impl std::fmt::Debug for Composition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Composition")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// A chart to compose, the chart of one row: `None` for a `NULL` chart, and for a facet the key of
/// the row, as an array of one element.
#[derive(Clone, Debug)]
pub struct Child {
    pub chart: Option<Chart>,
    pub key: Option<ArrayRef>,
}

/// The order of the children of a composition: as the binding gives them, such as the order of an
/// `ORDER BY`, or for a facet the order of their keys, ascending with `NULL` last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChildOrder {
    AsGiven,
    ByKey,
}

/// The compositions.
pub fn compositions() -> &'static [Composition] {
    generated::COMPOSITIONS
}

/// The composition `name`.
pub fn composition(name: &str) -> Option<&'static Composition> {
    compositions()
        .iter()
        .find(|composition| composition.name == name)
}

impl Composition {
    /// The chart of `children` composed with the options `options`, one for each parameter, where
    /// `None` leaves an option unset. A facet takes the name of its key's column, `column`, and the
    /// key of each child. A `NULL` chart is not a child of a layer or a concat, and it is an empty
    /// cell at its key in a facet. Without children, the chart is empty: its root is an empty node.
    pub fn compose(
        &self,
        column: Option<&str>,
        children: Vec<Child>,
        order: ChildOrder,
        options: &[Option<Arg>],
    ) -> Result<Chart, ArgError> {
        if options.len() > self.params.len() {
            return Err(ArgError::arity(self.params.len(), options.len()));
        }
        let mut options = options.to_vec();
        options.resize_with(self.params.len(), || None);
        let kind = self.kind;
        if kind.takes_key() {
            let column = column.ok_or_else(|| {
                ArgError::composition(format!("{}() takes the column of its key", self.name))
            })?;
            return self.facet(column, children, order, &options);
        }
        let charts: Vec<Chart> = children
            .into_iter()
            .filter_map(|child| child.chart)
            .collect();
        if charts.is_empty() {
            return empty();
        }
        let node: NodeBuilder = match kind {
            CompositionKind::Layer => {
                for (index, chart) in charts.iter().enumerate() {
                    if !matches!(root(chart), Some(Node::Plot(_))) {
                        return Err(ArgError::composition(format!(
                            "layer() overlays plots, and the chart of child {index} has a {} at its root",
                            member(root(chart))
                        )));
                    }
                }
                generated::apply_all_layer(proto::Plot::overlay(charts), &options)?.into()
            }
            CompositionKind::HConcat | CompositionKind::VConcat => {
                let direction = match kind {
                    CompositionKind::HConcat => Direction::Row,
                    _ => Direction::Column,
                };
                let concat = proto::Concat::new(direction).children(charts);
                let apply = match kind {
                    CompositionKind::HConcat => generated::apply_all_hconcat,
                    _ => generated::apply_all_vconcat,
                };
                apply(concat, &options)?.into()
            }
            _ => unreachable!("a composition without a key"),
        };
        Chart::builder(node).build().map_err(ArgError::invalid)
    }

    /// Checks the options `options` of a call, one for each parameter, as [`compose`] applies
    /// them, with the column of the key of a facet, so that a binding reports their errors before
    /// it composes rows.
    ///
    /// [`compose`]: Composition::compose
    pub fn check(&self, column: Option<&str>, options: &[Option<Arg>]) -> Result<(), ArgError> {
        if options.len() > self.params.len() {
            return Err(ArgError::arity(self.params.len(), options.len()));
        }
        let mut options = options.to_vec();
        options.resize_with(self.params.len(), || None);
        let column = column.unwrap_or_default();
        match self.kind {
            CompositionKind::Layer => {
                generated::apply_all_layer(proto::PlotBuilder::default(), &options)?;
            }
            CompositionKind::HConcat => {
                generated::apply_all_hconcat(proto::Concat::new(Direction::Row), &options)?;
            }
            CompositionKind::VConcat => {
                generated::apply_all_vconcat(proto::Concat::new(Direction::Column), &options)?;
            }
            CompositionKind::FacetRow | CompositionKind::FacetCol => {
                let apply = match self.kind {
                    CompositionKind::FacetRow => generated::apply_all_facet_row,
                    _ => generated::apply_all_facet_col,
                };
                let mut concat = apply(proto::Concat::new(Direction::Row), &options)?;
                set_column(&mut concat.message.facet, column, self.name)?;
            }
            CompositionKind::FacetWrap => {
                let mut wrap = generated::apply_all_facet_wrap(proto::Wrap::builder(), &options)?;
                set_column(&mut wrap.message.facet, column, self.name)?;
            }
        }
        Ok(())
    }

    /// The facet of `children` by the key column `column`.
    fn facet(
        &self,
        column: &str,
        children: Vec<Child>,
        order: ChildOrder,
        options: &[Option<Arg>],
    ) -> Result<Chart, ArgError> {
        if children.is_empty() {
            return empty();
        }
        let mut keys = Vec::with_capacity(children.len());
        for child in &children {
            match &child.key {
                Some(key) if key.len() == 1 => keys.push(key.clone()),
                _ => {
                    return Err(ArgError::composition(format!(
                        "{}() takes one key value for each chart",
                        self.name
                    )))
                }
            }
        }
        let refs: Vec<&dyn Array> = keys.iter().map(|key| key.as_ref()).collect();
        let all = concat(&refs).map_err(|error| ArgError::composition(error.to_string()))?;
        let rows = key_rows(&all)?;
        // Two children with one key value are an error.
        let mut seen: HashMap<&[u8], usize> = HashMap::new();
        for (index, row) in rows.iter().enumerate() {
            if seen.insert(row.as_slice(), index).is_some() {
                return Err(ArgError::composition(format!(
                    "{}() takes one chart for each value of its key, and two rows have the key value {}",
                    self.name,
                    describe(&all, index)
                )));
            }
        }
        let mut indices: Vec<usize> = (0..children.len()).collect();
        if order == ChildOrder::ByKey {
            indices.sort_by(|a, b| rows[*a].cmp(&rows[*b]));
        }
        let mut slots: Vec<Option<Child>> = children.into_iter().map(Some).collect();
        let mut nodes: Vec<NodeBuilder> = Vec::with_capacity(indices.len());
        for index in indices {
            let child = slots[index].take().expect("each child once");
            let node: NodeBuilder = match child.chart {
                Some(chart) => chart.into(),
                None => proto::Empty::builder().into(),
            };
            let value = PartitionValue::new(column, Value::scalar(keys[index].clone()));
            nodes.push(node.partition([value]));
        }
        let node: NodeBuilder = match self.kind {
            CompositionKind::FacetRow | CompositionKind::FacetCol => {
                let direction = match self.kind {
                    CompositionKind::FacetRow => Direction::Column,
                    _ => Direction::Row,
                };
                align(&mut nodes, direction)?;
                let concat = proto::Concat::new(direction).children(nodes);
                let apply = match self.kind {
                    CompositionKind::FacetRow => generated::apply_all_facet_row,
                    _ => generated::apply_all_facet_col,
                };
                let mut concat = apply(concat, options)?;
                set_column(&mut concat.message.facet, column, self.name)?;
                concat.into()
            }
            CompositionKind::FacetWrap => {
                let wrap = proto::Wrap::builder().children(nodes);
                let mut wrap = generated::apply_all_facet_wrap(wrap, options)?;
                set_column(&mut wrap.message.facet, column, self.name)?;
                wrap.into()
            }
            _ => unreachable!("a composition with a key"),
        };
        Chart::builder(node).build().map_err(ArgError::invalid)
    }
}

/// The empty chart of a composition without children, whose root is an empty node, which takes no
/// options.
fn empty() -> Result<Chart, ArgError> {
    Chart::builder(proto::Empty::builder())
        .build()
        .map_err(ArgError::invalid)
}

/// The root of a chart.
fn root(chart: &Chart) -> Option<&Node> {
    chart.message.root.as_ref()?.node.as_ref()
}

/// The member of a node, in the words of an error.
fn member(node: Option<&Node>) -> &'static str {
    match node {
        Some(Node::Plot(_)) => "plot",
        Some(Node::Concat(_)) => "concat",
        Some(Node::Wrap(_)) => "wrap",
        Some(Node::Empty(_)) => "empty node",
        None => "node without a member",
    }
}

/// The encoded rows of the key values `keys`, which compare ascending with `NULL` last.
fn key_rows(keys: &ArrayRef) -> Result<Vec<Vec<u8>>, ArgError> {
    let options = SortOptions {
        descending: false,
        nulls_first: false,
    };
    let converter = RowConverter::new(vec![SortField::new_with_options(
        keys.data_type().clone(),
        options,
    )])
    .map_err(|error| ArgError::composition(error.to_string()))?;
    let rows = converter
        .convert_columns(std::slice::from_ref(keys))
        .map_err(|error| ArgError::composition(error.to_string()))?;
    Ok(rows.iter().map(|row| row.as_ref().to_vec()).collect())
}

/// The key value at `index` of `keys`, in the words of an error.
fn describe(keys: &ArrayRef, index: usize) -> String {
    match keys.is_null(index) {
        true => "NULL".to_string(),
        false => arrow::util::display::array_value_to_string(keys, index)
            .unwrap_or_else(|_| format!("at row {index}")),
    }
}

/// Sets the column of the facet `facet` to `column`. A facet that names another column is an
/// error.
fn set_column(facet: &mut Option<proto::Facet>, column: &str, name: &str) -> Result<(), ArgError> {
    let facet = facet.get_or_insert_with(Default::default);
    if !facet.column.is_empty() && facet.column != column {
        return Err(ArgError::composition(format!(
            "{name}() facets its key `{column}`, and its `facet` names the column `{}`",
            facet.column
        ))
        .within("facet"));
    }
    facet.column = column.to_string();
    Ok(())
}

/// Aligns the children of a facet in `direction` whose roots are facets in the other direction:
/// each inner facet gets a child for each value of the inner key across the children, with an
/// empty node for a value that it lacks, so the cells of a grid line up. The values are ascending
/// with `NULL` last when each inner facet's values are ascending, and otherwise in the order of
/// their first appearance. Children of another kind are not aligned.
fn align(nodes: &mut [NodeBuilder], direction: Direction) -> Result<(), ArgError> {
    let inner = match direction {
        Direction::Row => Direction::Column,
        _ => Direction::Row,
    };
    // The inner facets, and their column, when every child that is not empty is one.
    let mut column: Option<String> = None;
    let mut facets = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        match &node.message.node {
            Some(Node::Empty(_)) => {}
            Some(Node::Concat(concat))
                if concat.direction == inner as i32 && concat.facet.is_some() =>
            {
                let facet_column = &concat.facet.as_ref().expect("a facet").column;
                match &column {
                    Some(known) if known != facet_column => return Ok(()),
                    _ => column = Some(facet_column.clone()),
                }
                facets.push(index);
            }
            _ => return Ok(()),
        }
    }
    let Some(column) = column else {
        return Ok(());
    };
    // The values of each inner facet, in its order. The canonical form declares a value that every
    // child of a facet holds on the facet, as for a facet of one child, so it moves to the children
    // first, which then hold their values whatever cells join them.
    let mut values: Vec<Vec<ArrayRef>> = Vec::with_capacity(facets.len());
    for index in &facets {
        let node = &mut nodes[*index];
        let Some(Node::Concat(concat)) = &mut node.message.node else {
            unreachable!("an inner facet")
        };
        if let Some(at) = (concat.partition.iter()).position(|value| value.column == column) {
            let value = concat.partition.remove(at);
            for child in &mut concat.children {
                if let Some(partition) = partition_mut(child) {
                    partition.push(value.clone());
                }
            }
        }
        let node = &nodes[*index];
        let Some(Node::Concat(concat)) = &node.message.node else {
            unreachable!("an inner facet")
        };
        let mut own = Vec::with_capacity(concat.children.len());
        for child in &concat.children {
            match partition_value(child, &column, &node.parts.values) {
                Some(value) => own.push(value),
                None => return Ok(()),
            }
        }
        values.push(own);
    }
    let flat: Vec<&dyn Array> = values
        .iter()
        .flatten()
        .map(|value| value.as_ref())
        .collect();
    if flat.is_empty() {
        return Ok(());
    }
    let all = match concat(&flat) {
        Ok(all) => all,
        Err(_) => return Ok(()),
    };
    let rows = key_rows(&all)?;
    let mut offset = 0;
    let mut ascending = true;
    let mut per_facet: Vec<Vec<usize>> = Vec::with_capacity(values.len());
    for own in &values {
        let range: Vec<usize> = (offset..offset + own.len()).collect();
        ascending &= range.windows(2).all(|pair| rows[pair[0]] < rows[pair[1]]);
        offset += own.len();
        per_facet.push(range);
    }
    // The distinct values, each by its first row.
    let mut union: Vec<usize> = Vec::new();
    let mut known: HashSet<&[u8]> = HashSet::new();
    for (index, row) in rows.iter().enumerate() {
        if known.insert(row.as_slice()) {
            union.push(index);
        }
    }
    if ascending {
        union.sort_by(|a, b| rows[*a].cmp(&rows[*b]));
    }
    for (facet, own) in facets.iter().zip(per_facet) {
        let node = &mut nodes[*facet];
        let NodeBuilder { message, parts } = node;
        let Some(Node::Concat(concat)) = &mut message.node else {
            unreachable!("an inner facet")
        };
        let mut existing: HashMap<&[u8], proto::Node> = HashMap::new();
        for (child, row) in std::mem::take(&mut concat.children).into_iter().zip(&own) {
            existing.insert(rows[*row].as_slice(), child);
        }
        let mut children = Vec::with_capacity(union.len());
        for (position, row) in union.iter().enumerate() {
            match existing.remove(rows[*row].as_slice()) {
                Some(child) => children.push(child),
                None => {
                    let value = all.slice(*row, 1);
                    let value = PartitionValue::new(column.as_str(), Value::scalar(value));
                    let empty = NodeBuilder::from(proto::Empty::builder()).partition([value]);
                    let path = format!("concat.children[{position}]");
                    children.push(parts.embed(&path, empty));
                }
            }
        }
        concat.children = children;
    }
    Ok(())
}

/// The value of the partition column `column` of the node `node`, whose values are `values`.
fn partition_value(node: &proto::Node, column: &str, values: &[ArrayRef]) -> Option<ArrayRef> {
    let partition = match node.node.as_ref()? {
        Node::Plot(plot) => &plot.partition,
        Node::Concat(concat) => &concat.partition,
        Node::Wrap(wrap) => &wrap.partition,
        Node::Empty(empty) => &empty.partition,
    };
    let value = partition.iter().find(|value| value.column == column)?;
    let index = value.value.as_ref()?.index as usize;
    values.get(index).map(Arc::clone)
}

/// The partition of the node `node`.
fn partition_mut(node: &mut proto::Node) -> Option<&mut Vec<PartitionValue>> {
    Some(match node.node.as_mut()? {
        Node::Plot(plot) => &mut plot.partition,
        Node::Concat(concat) => &mut concat.partition,
        Node::Wrap(wrap) => &mut wrap.partition,
        Node::Empty(empty) => &mut empty.partition,
    })
}
