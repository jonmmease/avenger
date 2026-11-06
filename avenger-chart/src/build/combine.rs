use std::collections::HashMap;

use arrow::array::{ArrayRef, RecordBatch};
use prost::bytes::Bytes;

use super::Fragment;
use crate::{
    layer::CoordKind,
    proto::{self, node::Node, plot::Coord},
    validate::value_key,
    walk::{VisitTables, VisitValues},
    Chart, ChartError,
};

type Result<T> = std::result::Result<T, ChartError>;

/// The parts of the charts that combine: their tables with their streams, their values, their
/// selections, and their format syntaxes, with the references of each child's root renumbered to
/// them.
#[derive(Default)]
struct Combined {
    message: proto::Chart,
    tables: Vec<RecordBatch>,
    streams: Vec<Option<Bytes>>,
    values: Vec<ArrayRef>,
}

impl Combined {
    /// Adds the parts of each child and returns their roots, in order. A number or a datetime
    /// syntax that one child holds is the syntax of the chart, and two different syntaxes are an
    /// error. The outer sizes and the paddings of the children do not carry over.
    fn add(&mut self, children: Vec<Chart>) -> Result<Vec<proto::Node>> {
        let mut roots = Vec::with_capacity(children.len());
        for (index, child) in children.into_iter().enumerate() {
            let Chart {
                mut message,
                tables,
                streams,
                values,
            } = child;
            let (tables_offset, values_offset) =
                (self.tables.len() as u32, self.values.len() as u32);
            message.visit_values_mut(&mut |value| value.index += values_offset);
            let mut root = message.root.take().expect("a valid chart has a root");
            root.visit_tables_mut(&mut |table| table.index += tables_offset);
            for (field, syntax, known) in [
                (
                    "number_format_syntax",
                    message.number_format_syntax,
                    &mut self.message.number_format_syntax,
                ),
                (
                    "datetime_format_syntax",
                    message.datetime_format_syntax,
                    &mut self.message.datetime_format_syntax,
                ),
            ] {
                if known.is_empty() {
                    *known = syntax;
                } else if !syntax.is_empty() && syntax != *known {
                    return Err(ChartError::combine(
                        Some(index),
                        format!("the charts that combine have the {field} `{known}` and `{syntax}`, which is a conflict"),
                    ));
                }
            }
            self.message.selections.extend(message.selections);
            self.tables.extend(tables);
            self.streams.extend(streams);
            self.values.extend(values);
            roots.push(root);
        }
        Ok(roots)
    }

    /// Adds the values of `fragment` and returns its message, whose `ValueRef`s refer to them.
    fn embed<M: VisitValues>(&mut self, fragment: Fragment<M>) -> M {
        fragment.embed_in(&mut self.values)
    }

    /// The chart with `root`, which the reader checks and rewrites into the canonical form.
    fn finish(mut self, root: Node) -> Result<Chart> {
        self.message.version = crate::FORMAT_VERSION;
        self.message.root = Some(proto::Node { node: Some(root) });
        Chart::read_parts(self.message, self.tables, self.streams, self.values)
    }
}

/// Fails for a node of a combination that holds children of its own.
fn no_children(count: usize) -> Result<()> {
    match count {
        0 => Ok(()),
        _ => Err(ChartError::combine(
            None,
            "the node of a combination takes its children from the charts that combine",
        )),
    }
}

/// A row or a column of `children`: a `Concat` with the options of `node`, whose children are the
/// roots of the charts, in order. A facet is a concat whose options hold `facet`.
pub fn concat(node: impl Into<Fragment<proto::Concat>>, children: Vec<Chart>) -> Result<Chart> {
    let node = node.into();
    no_children(node.message.children.len())?;
    let mut combined = Combined::default();
    let roots = combined.add(children)?;
    let mut concat = combined.embed(node);
    concat.children = roots;
    combined.finish(Node::Concat(concat))
}

/// A wrap of `children`: a `Wrap` with the options of `node`, whose children are the roots of the
/// charts, in order. A wrapped facet grid is a wrap whose options hold `facet`.
pub fn wrap(node: impl Into<Fragment<proto::Wrap>>, children: Vec<Chart>) -> Result<Chart> {
    let node = node.into();
    no_children(node.message.children.len())?;
    let mut combined = Combined::default();
    let roots = combined.add(children)?;
    let mut wrap = combined.embed(node);
    wrap.children = roots;
    combined.finish(Node::Wrap(wrap))
}

/// A facet grid of `children`: a column, with the options of `node` and a facet of `row`, of rows
/// with a facet of `column`. Each child stands at the row of its value of `row` and at the column
/// of its value of `column`, which its partition holds. Rows and columns take the order in which
/// their values first appear among the children. A cell without a child is an empty node with the
/// values of its row and its column, so its header labels it. It fails for a child without either
/// value, and for two children in one cell.
pub fn facet_grid(
    row: &str,
    column: &str,
    node: impl Into<Fragment<proto::Concat>>,
    children: Vec<Chart>,
) -> Result<Chart> {
    let node = node.into();
    no_children(node.message.children.len())?;
    // The values of each child's cell, and the order of the rows and of the columns.
    let mut rows: Vec<(Vec<u8>, ArrayRef)> = Vec::new();
    let mut columns: Vec<(Vec<u8>, ArrayRef)> = Vec::new();
    let mut cells: HashMap<(usize, usize), usize> = HashMap::new();
    for (index, child) in children.iter().enumerate() {
        let partition = root_partition(child);
        let position = |name: &str, known: &mut Vec<(Vec<u8>, ArrayRef)>| {
            let value = partition.get(name).ok_or_else(|| {
                ChartError::combine(
                    Some(index),
                    format!("the chart has no partition value of `{name}`, so it has no cell"),
                )
            })?;
            let key =
                value_key(value).map_err(|reason| ChartError::combine(Some(index), reason))?;
            Ok::<_, ChartError>(match known.iter().position(|(known, _)| *known == key) {
                Some(position) => position,
                None => {
                    known.push((key, value.clone()));
                    known.len() - 1
                }
            })
        };
        let cell = (position(row, &mut rows)?, position(column, &mut columns)?);
        if let Some(first) = cells.insert(cell, index) {
            return Err(ChartError::combine(
                Some(index),
                format!("charts {first} and {index} stand in one cell of the grid"),
            ));
        }
    }
    let mut combined = Combined::default();
    let mut roots: Vec<Option<proto::Node>> =
        combined.add(children)?.into_iter().map(Some).collect();
    let mut row_nodes = Vec::with_capacity(rows.len());
    for (row_index, (_, row_value)) in rows.iter().enumerate() {
        let mut cells_of_row = Vec::with_capacity(columns.len());
        for (column_index, (_, column_value)) in columns.iter().enumerate() {
            let cell = match cells.get(&(row_index, column_index)) {
                Some(child) => roots[*child].take().expect("each child stands in one cell"),
                None => {
                    let partition = vec![
                        partition_value(&mut combined, row, row_value),
                        partition_value(&mut combined, column, column_value),
                    ];
                    proto::Node {
                        node: Some(Node::Empty(proto::Empty { partition })),
                    }
                }
            };
            cells_of_row.push(cell);
        }
        row_nodes.push(proto::Node {
            node: Some(Node::Concat(proto::Concat {
                direction: proto::Direction::Row as i32,
                children: cells_of_row,
                facet: Some(proto::Facet {
                    column: column.to_string(),
                    header: None,
                }),
                ..Default::default()
            })),
        });
    }
    let mut grid = combined.embed(node);
    grid.direction = proto::Direction::Column as i32;
    grid.facet = Some(proto::Facet {
        column: row.to_string(),
        header: grid.facet.and_then(|facet| facet.header),
    });
    grid.children = row_nodes;
    combined.finish(Node::Concat(grid))
}

/// A partition value of `column` that refers to a new value of `combined`.
fn partition_value(
    combined: &mut Combined,
    column: &str,
    value: &ArrayRef,
) -> proto::PartitionValue {
    combined.values.push(value.clone());
    proto::PartitionValue {
        column: column.to_string(),
        value: Some(proto::ValueRef {
            index: (combined.values.len() - 1) as u32,
        }),
    }
}

/// The partition of the root of `chart`, by column.
fn root_partition(chart: &Chart) -> std::collections::BTreeMap<String, ArrayRef> {
    let kind = match chart.root().node.as_ref() {
        Some(Node::Plot(_)) => "plot",
        Some(Node::Concat(_)) => "concat",
        Some(Node::Wrap(_)) => "wrap",
        _ => "empty",
    };
    let tree = crate::resolve::Tree::new(chart).expect("a valid chart has a valid tree");
    (tree.partitions.get(&format!("root.{kind}")).cloned()).unwrap_or_default()
}

/// One plot with the layers of `children`, whose roots are plots of one coordinate system, in
/// order. The id, the title, the attribution, the sizes, and the options of the coordinate system
/// are those of `node` when it holds them, and otherwise those of the children, where a field that
/// one child holds is the field of the plot and two different values are an error. The resolve
/// rules of `node` replace the children's rules of their channels. The tools are those of `node`,
/// then those of the children, and the classes are those of `node`, then those of the children
/// that it lacks. Each child's partition values move to its layers.
pub fn overlay(node: impl Into<Fragment<proto::Plot>>, children: Vec<Chart>) -> Result<Chart> {
    let mut combined = Combined::default();
    let roots = combined.add(children)?;
    let mut plot = combined.embed(node.into());
    let mut plots = Vec::with_capacity(roots.len());
    for (index, root) in roots.into_iter().enumerate() {
        match root.node {
            Some(Node::Plot(child)) => plots.push(child),
            _ => {
                return Err(ChartError::combine(
                    Some(index),
                    "the root of a chart that an overlay combines is a plot",
                ))
            }
        }
    }
    let kind = (plots.first().and_then(CoordKind::of))
        .or_else(|| CoordKind::of(&plot))
        .ok_or_else(|| ChartError::combine(None, "an overlay combines one chart or more"))?;
    if let Some(index) = (plots.iter()).position(|child| CoordKind::of(child) != Some(kind)) {
        return Err(ChartError::combine(
            Some(index),
            format!(
                "the plots of an overlay are of one coordinate system, and the first is {}",
                kind.name()
            ),
        ));
    }
    if (plot.coord.as_ref()).is_some_and(|_| CoordKind::of(&plot) != Some(kind)) {
        return Err(ChartError::combine(
            None,
            "the coordinate system of the node of an overlay is that of its charts",
        ));
    }
    if layer_count(&plot).is_some_and(|count| count > 0) {
        return Err(ChartError::combine(
            None,
            "the node of an overlay takes its layers from the charts that combine",
        ));
    }

    plot.id = pick_text(&plot.id, plots.iter().map(|child| child.id.as_str()), "id")?;
    plot.title = pick(
        plot.title,
        plots.iter().map(|child| child.title.clone()),
        "title",
    )?;
    plot.attribution = pick(
        plot.attribution,
        plots.iter().map(|child| child.attribution.clone()),
        "attribution",
    )?;
    plot.width = pick(plot.width, plots.iter().map(|child| child.width), "width")?;
    plot.height = pick(
        plot.height,
        plots.iter().map(|child| child.height),
        "height",
    )?;
    let mut resolve = std::mem::take(&mut plot.resolve);
    for child in &plots {
        resolve.extend(
            (child.resolve.iter())
                .filter(|rule| !plot.resolve.iter().any(|own| own.channel == rule.channel))
                .copied(),
        );
    }
    plot.resolve = resolve;
    for child in &plots {
        plot.tools.extend(child.tools.iter().cloned());
        for class in &child.classes {
            if !plot.classes.contains(class) {
                plot.classes.push(class.clone());
            }
        }
    }

    // The options of the node's coordinate system win over those of the children.
    let node_options = has_options(plot.coord.as_ref());
    let mut coord = plot.coord.take();
    for (index, child) in plots.into_iter().enumerate() {
        let partition = child.partition;
        let child_coord = child.coord.expect("a valid plot has a coordinate system");
        coord = Some(merge_coord(
            coord,
            child_coord,
            &partition,
            index,
            node_options,
        )?);
    }
    plot.coord = coord;
    combined.finish(Node::Plot(plot))
}

/// The count of layers of `plot`, or `None` without a coordinate system.
fn layer_count(plot: &proto::Plot) -> Option<usize> {
    Some(match plot.coord.as_ref()? {
        Coord::Cartesian(cartesian) => cartesian.layers.len(),
        Coord::Polar(polar) => polar.layers.len(),
        Coord::Geo(geo) => geo.layers.len(),
    })
}

/// Whether `coord` holds the options of its coordinate system.
fn has_options(coord: Option<&Coord>) -> bool {
    match coord {
        Some(Coord::Cartesian(cartesian)) => cartesian.options.is_some(),
        Some(Coord::Polar(polar)) => polar.options.is_some(),
        Some(Coord::Geo(geo)) => geo.options.is_some(),
        None => false,
    }
}

/// `coord` with the layers of `child`, the chart at `index`, each with the plot partition
/// `partition`. Unless the node's options win, the options of `child` merge into those of `coord`,
/// where two different options are an error.
fn merge_coord(
    coord: Option<Coord>,
    child: Coord,
    partition: &[proto::PartitionValue],
    index: usize,
    node_options: bool,
) -> Result<Coord> {
    macro_rules! merge {
        ($variant:ident, $message:ident, $own:expr, $child:expr) => {{
            let mut own: proto::$message = $own.unwrap_or_default();
            if !node_options {
                own.options = merge_present(own.options, $child.options, "options", index)?;
            }
            for mut layer in $child.layers {
                let mut values = partition.to_vec();
                values.append(&mut layer.partition);
                layer.partition = values;
                own.layers.push(layer);
            }
            Coord::$variant(own)
        }};
    }
    Ok(match (coord, child) {
        (None, Coord::Cartesian(child)) => merge!(Cartesian, CartesianPlot, None, child),
        (Some(Coord::Cartesian(own)), Coord::Cartesian(child)) => {
            merge!(Cartesian, CartesianPlot, Some(own), child)
        }
        (None, Coord::Polar(child)) => merge!(Polar, PolarPlot, None, child),
        (Some(Coord::Polar(own)), Coord::Polar(child)) => {
            merge!(Polar, PolarPlot, Some(own), child)
        }
        (None, Coord::Geo(child)) => merge!(Geo, GeoPlot, None, child),
        (Some(Coord::Geo(own)), Coord::Geo(child)) => merge!(Geo, GeoPlot, Some(own), child),
        _ => unreachable!("the plots of an overlay are of one coordinate system"),
    })
}

/// `own` when it is present, and otherwise the value that the children hold, in order, where two
/// different values are an error.
fn pick<T: PartialEq>(
    own: Option<T>,
    children: impl IntoIterator<Item = Option<T>>,
    field: &str,
) -> Result<Option<T>> {
    if own.is_some() {
        return Ok(own);
    }
    let mut picked: Option<T> = None;
    for (index, value) in children.into_iter().enumerate() {
        picked = merge_present(picked, value, field, index)?;
    }
    Ok(picked)
}

/// The value that `known` or `value` holds, where `value` is that of the chart at `index`, and two
/// different values are an error.
fn merge_present<T: PartialEq>(
    known: Option<T>,
    value: Option<T>,
    field: &str,
    index: usize,
) -> Result<Option<T>> {
    match (known, value) {
        (Some(known), Some(value)) if known != value => Err(ChartError::combine(
            Some(index),
            format!(
                "the plots of an overlay have different values of `{field}`, which is a conflict"
            ),
        )),
        (Some(known), _) => Ok(Some(known)),
        (None, value) => Ok(value),
    }
}

/// As for [`pick`], for a text field, which is absent when it is empty.
fn pick_text<'a>(
    own: &'a str,
    children: impl Iterator<Item = &'a str>,
    field: &str,
) -> Result<String> {
    let present = |text: &'a str| (!text.is_empty()).then_some(text);
    let own = present(own);
    Ok(pick(own, children.map(present), field)?
        .unwrap_or_default()
        .to_string())
}

/// `chart` with the partition value `value` of `column` declared on its root.
pub fn with_partition(chart: Chart, column: &str, value: ArrayRef) -> Result<Chart> {
    let Chart {
        mut message,
        tables,
        streams,
        mut values,
    } = chart;
    values.push(value);
    let partition_value = proto::PartitionValue {
        column: column.to_string(),
        value: Some(proto::ValueRef {
            index: (values.len() - 1) as u32,
        }),
    };
    match message.root.as_mut().and_then(|root| root.node.as_mut()) {
        Some(Node::Plot(plot)) => plot.partition.push(partition_value),
        Some(Node::Concat(concat)) => concat.partition.push(partition_value),
        Some(Node::Wrap(wrap)) => wrap.partition.push(partition_value),
        Some(Node::Empty(empty)) => empty.partition.push(partition_value),
        None => unreachable!("a valid chart has a root"),
    }
    Chart::read_parts(message, tables, streams, values)
}
