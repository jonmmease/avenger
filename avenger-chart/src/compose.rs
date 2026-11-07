//! The methods of the builders that are not fields of the format: the building of a chart, a
//! finished chart as a node of a builder tree, the overlay of plots, and partition values of
//! nodes, from a table or one by one.

use std::collections::HashMap;

use arrow::{
    array::{ArrayRef, RecordBatch, UInt32Array},
    compute::{cast, take_record_batch, SortOptions},
    datatypes::DataType,
    row::{RowConverter, SortField},
};

use crate::{
    builder::{PartitionOrder, Parts, Syntaxes, Value},
    infer::infer_scales,
    layer::CoordKind,
    proto::{
        self, node::Node, plot::Coord, ChartBuilder, NodeBuilder, PartitionValue,
        PartitionValueBuilder, PlotBuilder,
    },
    Chart, ChartError, FORMAT_VERSION,
};

impl ChartBuilder {
    /// Asks [`build`](Self::build) to infer the absent scales of the whole tree: for each encoding
    /// whose value or one of whose conditional values is a `scaled_column` or a `scaled_constant`,
    /// the scale when the encoding has none, its type when it is absent, and its domain when it is
    /// absent. It applies at `build`, wherever it stands in the chain, and it never changes a field
    /// that is set.
    ///
    /// The scale type follows the Arrow type of the values, the channel, and for categories on a
    /// position, the mark: numbers take a linear scale, except on `shape`, `pattern`, and
    /// `stroke_dash`, which take an ordinal one; dates and timestamps take a time scale; strings
    /// and booleans take a band scale on a position of a bar or a rect, a point scale on another
    /// position, and an ordinal scale elsewhere; and structs of paths take a nested band scale on a
    /// position. An absent domain is the layer's data domain. Fits, guides, titles, and styles
    /// stay absent.
    pub fn infer_scales(mut self) -> Self {
        self.infer_scales = true;
        self
    }

    /// The chart: the builder's message with the version of the format, the tables and the values
    /// that its references refer to, and the selections and the format syntaxes of the charts that
    /// it embeds, after the inference that the builder asks for, checked against every rule of the
    /// format and in its canonical form.
    ///
    /// The selections of the embedded charts join the builder's own. A format syntax that the
    /// builder or one embedded chart sets is the chart's, and two different syntaxes are an error.
    /// The width, the height, and the padding of an embedded chart do not carry over.
    pub fn build(self) -> Result<Chart, ChartError> {
        let ChartBuilder {
            mut message,
            parts,
            infer_scales: infer,
        } = self;
        parts.error("")?;
        for embedded in &parts.syntaxes {
            agree(
                &mut message.number_format_syntax,
                &embedded.number,
                "number_format_syntax",
                &embedded.path,
            )?;
            agree(
                &mut message.datetime_format_syntax,
                &embedded.datetime,
                "datetime_format_syntax",
                &embedded.path,
            )?;
        }
        message.selections.extend(parts.selections);
        message.version = FORMAT_VERSION;
        let (tables, streams): (Vec<RecordBatch>, Vec<_>) = parts.tables.into_iter().unzip();
        if infer {
            infer_scales(&mut message, &tables, &parts.values);
        }
        Chart::read_parts(message, tables, streams, parts.values)
    }
}

/// Sets `known` to `syntax`, the syntax of the chart at `path`, when it is empty, where two
/// different syntaxes are an error.
fn agree(known: &mut String, syntax: &str, field: &str, path: &str) -> Result<(), ChartError> {
    if syntax.is_empty() {
        return Ok(());
    }
    if known.is_empty() {
        *known = syntax.to_string();
        return Ok(());
    }
    match known == syntax {
        true => Ok(()),
        false => Err(ChartError::invalid(
            path,
            format!(
                "the chart here has the {field} `{syntax}`, and the chart that embeds it has \
                 `{known}`, which is a conflict"
            ),
        )),
    }
}

/// A finished chart as a node: its root, with its tables, their IPC streams, its values, its
/// selections, and its format syntaxes, which [`ChartBuilder::build`] merges.
impl From<Chart> for NodeBuilder {
    fn from(chart: Chart) -> Self {
        let Chart {
            mut message,
            tables,
            streams,
            values,
        } = chart;
        let root = message.root.take().expect("a valid chart has a root");
        let parts = Parts {
            tables: tables.into_iter().zip(streams).collect(),
            values,
            selections: std::mem::take(&mut message.selections),
            syntaxes: vec![Syntaxes {
                path: String::new(),
                number: message.number_format_syntax,
                datetime: message.datetime_format_syntax,
            }],
            errors: Vec::new(),
            datum: Some(String::new()),
        };
        NodeBuilder {
            message: root,
            parts,
        }
    }
}

impl NodeBuilder {
    /// Adds partition values to the member of the node, whichever it is, as its `partition` field
    /// describes, such as the value of the cell of a finished chart in a facet:
    /// `NodeBuilder::from(chart).partition([..])`. A node without a member is an error of
    /// [`ChartBuilder::build`].
    pub fn partition(
        mut self,
        values: impl IntoIterator<Item = impl Into<PartitionValueBuilder>>,
    ) -> Self {
        for value in values {
            let NodeBuilder { message, parts } = &mut self;
            let (member, partition) = match message.node.as_mut() {
                Some(Node::Plot(plot)) => ("plot", &mut plot.partition),
                Some(Node::Concat(concat)) => ("concat", &mut concat.partition),
                Some(Node::Wrap(wrap)) => ("wrap", &mut wrap.partition),
                Some(Node::Empty(empty)) => ("empty", &mut empty.partition),
                None => {
                    parts.errors.push((
                        String::new(),
                        "a node without a member takes no partition value".to_string(),
                    ));
                    continue;
                }
            };
            let path = format!("{member}.partition[{}]", partition.len());
            let value: PartitionValue = parts.embed(&path, value.into());
            partition.push(value);
        }
        self
    }
}

impl proto::Node {
    /// One node for each distinct value of the column `column` of `batch`, in the order `order`:
    /// `cell` builds each node from the rows that hold the value, and the node's partition declares
    /// the value. The rows keep their order in `batch` and keep the key column. The value has the
    /// Arrow type of the column, and a dictionary column gives the type of its values. A null key
    /// forms its own node, last in both sorted orders, and NaN sorts above every number.
    ///
    /// Two nested calls build a facet grid: a column concat that facets the rows, of row concats
    /// that facet the columns. The function adds no empty cells. It fails when `batch` has no
    /// column `column`, and with the first error of `cell`.
    pub fn partitions<N: Into<NodeBuilder>>(
        batch: &RecordBatch,
        column: &str,
        order: PartitionOrder,
        mut cell: impl FnMut(RecordBatch) -> Result<N, ChartError>,
    ) -> Result<Vec<NodeBuilder>, ChartError> {
        let key = batch.column_by_name(column).ok_or_else(|| {
            ChartError::invalid(column, format!("the table has no column `{column}`"))
        })?;
        let key: ArrayRef = match key.data_type() {
            DataType::Dictionary(_, values) => cast(key, values)?,
            _ => key.clone(),
        };
        let options = SortOptions {
            descending: order == PartitionOrder::Descending,
            nulls_first: false,
        };
        let converter = RowConverter::new(vec![SortField::new_with_options(
            key.data_type().clone(),
            options,
        )])?;
        let rows = converter.convert_columns(std::slice::from_ref(&key))?;
        // The rows of each value, in the order in which the values first appear.
        let mut groups: Vec<(Vec<u8>, Vec<u32>)> = Vec::new();
        let mut known: HashMap<Vec<u8>, usize> = HashMap::new();
        for (index, row) in rows.iter().enumerate() {
            let bytes = row.as_ref().to_vec();
            match known.get(&bytes) {
                Some(group) => groups[*group].1.push(index as u32),
                None => {
                    known.insert(bytes.clone(), groups.len());
                    groups.push((bytes, vec![index as u32]));
                }
            }
        }
        // The encoded rows compare in the order of `options`, and the values are distinct.
        if order != PartitionOrder::Appearance {
            groups.sort_by(|a, b| a.0.cmp(&b.0));
        }
        (groups.into_iter())
            .map(|(_, indices)| {
                let value = key.slice(indices[0] as usize, 1);
                let rows = take_record_batch(batch, &UInt32Array::from(indices))?;
                let node: NodeBuilder = cell(rows)?.into();
                Ok(node.partition([PartitionValue::builder()
                    .column(column)
                    .value(Value::scalar(value))]))
            })
            .collect()
    }
}

impl PlotBuilder {
    /// Adds plots whose layers join this plot, finished charts or builders whose roots are plots of
    /// the plot's coordinate system. The plot's own fields win whenever they are set, and
    /// otherwise:
    ///
    /// - the id, the title, the attribution, the sizes, and the options of the coordinate system
    ///   are those that one overlaid plot sets, where two different values are an error
    /// - the resolve rules of the plot replace the overlaid plots' rules of their channels
    /// - the tools are those of the plot, then those of the overlaid plots, and the classes are
    ///   those of the plot, then the overlaid plots' classes that the plot lacks
    /// - the plot's own layers come first, then the layers of each overlaid plot in order, and the
    ///   partition values of each overlaid plot move to its layers
    pub fn overlay(mut self, plots: impl IntoIterator<Item = impl Into<NodeBuilder>>) -> Self {
        self.overlays.extend(plots.into_iter().map(Into::into));
        self
    }
}

/// The message and the parts of `builder`, with the plots that it overlays merged into it.
pub(crate) fn overlaid(builder: PlotBuilder) -> (proto::Plot, Parts) {
    let PlotBuilder {
        mut message,
        mut parts,
        overlays,
    } = builder;
    let mut plots = Vec::with_capacity(overlays.len());
    for (index, overlay) in overlays.into_iter().enumerate() {
        let path = format!("overlay[{index}]");
        match parts.embed(&path, overlay).node {
            Some(Node::Plot(plot)) => plots.push((path, plot)),
            _ => parts
                .errors
                .push((path, "an overlaid node is a plot".to_string())),
        }
    }
    if !plots.is_empty() {
        if let Err((path, reason)) = merge(&mut message, plots) {
            parts.errors.push((path, reason));
        }
    }
    (message, parts)
}

type Conflict = (String, String);

/// Merges `plots`, each at its path from the plot, into `plot`.
fn merge(plot: &mut proto::Plot, plots: Vec<(String, proto::Plot)>) -> Result<(), Conflict> {
    let kind = CoordKind::of(plot)
        .or_else(|| CoordKind::of(&plots[0].1))
        .ok_or_else(|| {
            (
                plots[0].0.clone(),
                "an overlaid plot has a coordinate system".to_string(),
            )
        })?;
    if let Some((path, _)) = (plots.iter()).find(|(_, child)| CoordKind::of(child) != Some(kind)) {
        return Err((
            path.clone(),
            format!(
                "an overlaid plot is a plot of the coordinate system of the plot, which is {}",
                kind.name()
            ),
        ));
    }
    if plot.id.is_empty() {
        plot.id = pick(
            plots
                .iter()
                .map(|(path, child)| (path, (!child.id.is_empty()).then(|| child.id.clone()))),
            "id",
        )?
        .unwrap_or_default();
    }
    if plot.title.is_none() {
        plot.title = pick(
            plots
                .iter()
                .map(|(path, child)| (path, child.title.clone())),
            "title",
        )?;
    }
    if plot.attribution.is_none() {
        plot.attribution = pick(
            plots
                .iter()
                .map(|(path, child)| (path, child.attribution.clone())),
            "attribution",
        )?;
    }
    if plot.width.is_none() {
        plot.width = pick(
            plots.iter().map(|(path, child)| (path, child.width)),
            "width",
        )?;
    }
    if plot.height.is_none() {
        plot.height = pick(
            plots.iter().map(|(path, child)| (path, child.height)),
            "height",
        )?;
    }
    let own_rules = plot.resolve.clone();
    for (_, child) in &plots {
        plot.resolve.extend(
            (child.resolve.iter())
                .filter(|rule| !own_rules.iter().any(|own| own.key == rule.key))
                .cloned(),
        );
    }
    for (_, child) in &plots {
        plot.tools.extend(child.tools.iter().cloned());
        for class in &child.classes {
            if !plot.classes.contains(class) {
                plot.classes.push(class.clone());
            }
        }
    }
    // The options of the plot's own coordinate system win over those of the overlaid plots.
    let own_options = has_options(plot.coord.as_ref());
    let mut coord = plot.coord.take();
    let mut options_path: Option<String> = None;
    for (path, child) in plots {
        let child_coord = child
            .coord
            .expect("an overlaid plot has a coordinate system");
        coord = Some(merge_coord(
            coord,
            child_coord,
            &child.partition,
            &path,
            own_options,
            &mut options_path,
        )?);
    }
    plot.coord = coord;
    Ok(())
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

/// `coord` with the layers of `child`, the overlaid plot at `path`, each with the plot partition
/// `partition`. Unless the plot's own options win, the options of `child` merge into those of
/// `coord`, where two different options are an error.
fn merge_coord(
    coord: Option<Coord>,
    child: Coord,
    partition: &[proto::PartitionValue],
    path: &str,
    own_options: bool,
    options_path: &mut Option<String>,
) -> Result<Coord, Conflict> {
    macro_rules! merge {
        ($variant:ident, $message:ident, $own:expr, $child:expr) => {{
            let mut own: proto::$message = $own.unwrap_or_default();
            if !own_options {
                match (&own.options, $child.options) {
                    (_, None) => {}
                    (None, Some(options)) => {
                        own.options = Some(options);
                        *options_path = Some(path.to_string());
                    }
                    (Some(known), Some(options)) if *known != options => {
                        return Err((
                            path.to_string(),
                            conflict("options", options_path.as_deref()),
                        ))
                    }
                    _ => {}
                }
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
        _ => unreachable!("the overlaid plots are of the plot's coordinate system"),
    })
}

/// The value that the overlaid plots set, each at its path, where two different values are an
/// error at the path of the second.
fn pick<'a, T: PartialEq>(
    values: impl IntoIterator<Item = (&'a String, Option<T>)>,
    field: &str,
) -> Result<Option<T>, Conflict> {
    let mut picked: Option<(T, &'a String)> = None;
    for (path, value) in values {
        let Some(value) = value else {
            continue;
        };
        match &picked {
            Some((known, first)) if *known != value => {
                return Err((path.clone(), conflict(field, Some(first))))
            }
            Some(_) => {}
            None => picked = Some((value, path)),
        }
    }
    Ok(picked.map(|(value, _)| value))
}

/// The reason of a conflict of `field` with the overlaid plot at `first`.
fn conflict(field: &str, first: Option<&str>) -> String {
    match first {
        Some(first) => format!(
            "the overlaid plot sets `{field}` to a value other than that of `{first}`, which is a \
             conflict"
        ),
        None => format!("the overlaid plot sets `{field}` to a value other than another's"),
    }
}
