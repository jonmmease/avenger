//! The canonical form of a chart, into which a reader rewrites every chart that it reads, so the
//! ways of writing one chart that the format accepts make equal charts and equal bytes.
//!
//! The canonical form:
//!
//! - declares each partition value once, on the highest node whose layers and empty cells all
//!   hold it
//! - joins the selections with one name, orders the selections by name, and orders the initial
//!   clauses of each by the tree order of their owners
//! - orders each other keyed list by its key, with one entry for each key
//! - leaves out the fields that a reader ignores, and the messages whose comments say that leaving
//!   them out means the default of each of their fields, when every field is at its default
//! - holds one copy of each table, in the order of the first layer that refers to it, and the
//!   values in the order that `Chart.values` describes
//!
//! A field that sets a value equal to the default that its comment gives when it is not set, such
//! as a share's weight of 1, stays as it is written.

use std::collections::{BTreeMap, HashMap};

use arrow::{
    array::{ArrayRef, RecordBatch},
    datatypes::DataType,
};
use prost::bytes::Bytes;

use crate::{
    layer::MarkMessage,
    proto::{
        self, domain, domain_fit, geo, node::Node, plot::Coord, scale, selection_domain, spherical,
    },
    resolve::{Identity, Tree},
    walk::{VisitTables, VisitValues},
    Chart,
};

/// Rewrites what each message of `chart`, whose messages are each valid, decides alone: keyed lists
/// in order, the fields that a reader ignores left out, and the option messages at every default
/// left out. The rules that span messages compare the specs and the options in this form.
pub(crate) fn local(chart: &mut Chart) {
    let root = (chart.message.root.as_mut()).expect("a valid chart has a root");
    local_node(root, Place::Root);
}

/// Rewrites the tree of `chart`, which is valid, into the canonical form: the partition values,
/// the selections, the tables, and the values.
pub(crate) fn tree(chart: &mut Chart) {
    let tree = Tree::new(chart).expect("a valid chart has a valid tree");
    let mut partitions = tree.partitions.clone();
    for layer in &tree.layers {
        partitions.insert(layer.path.clone(), layer.partition.clone());
    }
    let owners = owner_order(&tree);
    drop(tree);

    let mut hoist = Hoist {
        partitions,
        values: &mut chart.values,
    };
    let root = (chart.message.root.as_mut()).expect("a valid chart has a root");
    hoist.node(root, "root", &BTreeMap::new());
    selections(&mut chart.message.selections, &owners, &chart.values);
    (chart.tables, chart.streams) =
        canonical_tables(&mut chart.message, &chart.tables, &chart.streams);
    chart.values = canonical_values(&mut chart.message, &chart.values);
}

/// The position of each owner of clauses in tree order, by the key of its widget and its identity:
/// each identified plot, then its identified layers, then the widgets that its layers declare.
fn owner_order(tree: &Tree) -> HashMap<Vec<u8>, usize> {
    let mut order = HashMap::new();
    for plot in &tree.plots {
        let layers = tree.layers[plot.layers.clone()].iter();
        let identities =
            (plot.identity.iter()).chain(layers.filter_map(|layer| layer.identity.as_ref()));
        for identity in identities {
            let position = order.len();
            order.entry(owner_key("", identity)).or_insert(position);
        }
        // The widgets that the plot's layers declare, by their keys and anchors, after the plot's
        // tools and layers.
        for layer in &tree.layers[plot.layers.clone()] {
            for widget in layer.layer.widgets() {
                if let Some(owner) = crate::resolve::widget_owner(tree, layer, &widget.key) {
                    let position = order.len();
                    order
                        .entry(owner_key(&widget.key, &owner))
                        .or_insert(position);
                }
            }
        }
    }
    order
}

/// The key of the owner of a clause: the key of the widget that wrote it, or empty for a tool's
/// clause, with the identity of its owner.
fn owner_key(widget: &str, owner: &Identity) -> Vec<u8> {
    let mut key = widget.as_bytes().to_vec();
    key.push(0);
    key.extend(owner.key());
    key
}

/// Where a node stands, which decides the fields of it that a reader ignores.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    /// The root.
    Root,
    /// A child of a concat.
    Child,
    /// A cell of a wrap.
    Cell,
}

fn local_node(node: &mut proto::Node, place: Place) {
    match node.node.as_mut().expect("a valid node has a kind") {
        Node::Plot(plot) => {
            by_key(&mut plot.resolve, rule_order);
            distinct(&mut plot.classes);
            if place == Place::Cell {
                plot.width = None;
                plot.height = None;
            }
            match (plot.coord.as_mut()).expect("a valid plot has a coordinate system") {
                Coord::Cartesian(cartesian) => {
                    default_to_none(&mut cartesian.options);
                    for layer in &mut cartesian.layers {
                        distinct(&mut layer.classes);
                        encodings(layer.mark.as_mut().expect("a valid layer has a mark"));
                        widgets(&mut layer.widgets);
                    }
                }
                Coord::Polar(polar) => {
                    default_to_none(&mut polar.options);
                    for layer in &mut polar.layers {
                        distinct(&mut layer.classes);
                        encodings(layer.mark.as_mut().expect("a valid layer has a mark"));
                        widgets(&mut layer.widgets);
                    }
                }
                Coord::Geo(geo) => {
                    geo_options(geo.options.as_mut().expect("a valid geo plot has options"));
                    for layer in &mut geo.layers {
                        distinct(&mut layer.classes);
                        encodings(layer.mark.as_mut().expect("a valid layer has a mark"));
                        widgets(&mut layer.widgets);
                    }
                }
            }
        }
        Node::Empty(_) => {}
        Node::Concat(concat) => {
            by_key(&mut concat.resolve, rule_order);
            distinct(&mut concat.classes);
            if let Some(layout) = &mut concat.layout {
                by_key(&mut layout.part_rules, |rule| rule.part);
                // Only the outermost concat of a table distributes its free length.
                if place == Place::Child {
                    layout.horizontal_distribute = 0;
                    layout.vertical_distribute = 0;
                }
            }
            default_to_none(&mut concat.layout);
            if let Some(facet) = &mut concat.facet {
                distinct(&mut facet.classes);
            }
            for child in &mut concat.children {
                local_node(child, Place::Child);
            }
        }
        Node::Wrap(wrap) => {
            by_key(&mut wrap.resolve, rule_order);
            distinct(&mut wrap.classes);
            if let Some(layout) = &mut wrap.layout {
                by_key(&mut layout.part_rules, |rule| rule.part);
            }
            default_to_none(&mut wrap.layout);
            if let Some(facet) = &mut wrap.facet {
                distinct(&mut facet.classes);
            }
            for child in &mut wrap.children {
                local_node(child, Place::Cell);
            }
        }
    }
}

/// Declares each partition value on the highest node whose layers and empty cells all hold it.
struct Hoist<'a> {
    /// The partition of each node and layer, by its path, as `Tree` gives it.
    partitions: HashMap<String, BTreeMap<String, ArrayRef>>,
    /// The values of the chart, to which the partition values that move are added.
    values: &'a mut Vec<ArrayRef>,
}

impl Hoist<'_> {
    /// The partition values that the node or layer at `path` declares: the values of its
    /// partition that the partition of its parent, `parent`, does not hold. With them, its
    /// partition.
    fn declare(
        &mut self,
        path: &str,
        parent: &BTreeMap<String, ArrayRef>,
    ) -> (Vec<proto::PartitionValue>, BTreeMap<String, ArrayRef>) {
        let partition = self.partitions[path].clone();
        let declared = (partition.iter())
            .filter(|(column, value)| {
                parent
                    .get(*column)
                    .is_none_or(|known| known.as_ref() != value.as_ref())
            })
            .map(|(column, value)| {
                self.values.push(value.clone());
                proto::PartitionValue {
                    column: column.clone(),
                    value: Some(proto::ValueRef {
                        index: (self.values.len() - 1) as u32,
                    }),
                }
            })
            .collect();
        (declared, partition)
    }

    /// The partition values of a concat or a wrap at `path`, and those of its children.
    fn children(
        &mut self,
        path: &str,
        partition: &mut Vec<proto::PartitionValue>,
        children: &mut [proto::Node],
        parent: &BTreeMap<String, ArrayRef>,
    ) {
        let (declared, own) = self.declare(path, parent);
        *partition = declared;
        for (index, child) in children.iter_mut().enumerate() {
            self.node(child, &format!("{path}.children[{index}]"), &own);
        }
    }

    fn node(&mut self, node: &mut proto::Node, path: &str, parent: &BTreeMap<String, ArrayRef>) {
        match node.node.as_mut().expect("a valid node has a kind") {
            Node::Plot(plot) => {
                let path = format!("{path}.plot");
                let (declared, partition) = self.declare(&path, parent);
                plot.partition = declared;
                let mut layer = |coord: &str, index: usize, values: &mut Vec<_>| {
                    *values = self
                        .declare(&format!("{path}.{coord}.layers[{index}]"), &partition)
                        .0;
                };
                match (plot.coord.as_mut()).expect("a valid plot has a coordinate system") {
                    Coord::Cartesian(cartesian) => {
                        for (index, each) in cartesian.layers.iter_mut().enumerate() {
                            layer("cartesian", index, &mut each.partition);
                        }
                    }
                    Coord::Polar(polar) => {
                        for (index, each) in polar.layers.iter_mut().enumerate() {
                            layer("polar", index, &mut each.partition);
                        }
                    }
                    Coord::Geo(geo) => {
                        for (index, each) in geo.layers.iter_mut().enumerate() {
                            layer("geo", index, &mut each.partition);
                        }
                    }
                }
            }
            Node::Empty(empty) => {
                empty.partition = self.declare(&format!("{path}.empty"), parent).0;
            }
            Node::Concat(concat) => {
                let path = format!("{path}.concat");
                self.children(&path, &mut concat.partition, &mut concat.children, parent);
            }
            Node::Wrap(wrap) => {
                let path = format!("{path}.wrap");
                self.children(&path, &mut wrap.partition, &mut wrap.children, parent);
            }
        }
    }
}

/// Orders the entries of a keyed list by their keys and keeps one entry for each key. The chart
/// is valid, so entries with one key are equal.
/// The order of resolve rules: the rules of channels by channel, then the rules of widgets by key.
fn rule_order(rule: &proto::Resolve) -> (u8, i32, String) {
    match &rule.key {
        Some(proto::resolve::Key::Channel(channel)) => (0, *channel, String::new()),
        Some(proto::resolve::Key::Widget(key)) => (1, 0, key.clone()),
        None => (2, 0, String::new()),
    }
}

fn by_key<T, K: Ord>(items: &mut Vec<T>, key: impl Fn(&T) -> K) {
    items.sort_by_key(&key);
    items.dedup_by(|a, b| key(a) == key(b));
}

/// Leaves out a message whose fields are all at their defaults, where its comment says that
/// leaving it out means the default of each field.
fn default_to_none<T: Default + PartialEq>(message: &mut Option<T>) {
    if message
        .as_ref()
        .is_some_and(|message| *message == T::default())
    {
        *message = None;
    }
}

fn geo_options(options: &mut proto::Geo) {
    let Some(geo::Surface::Spherical(spherical)) = &mut options.surface else {
        return;
    };
    if let Some(spherical::Projection::Custom(custom)) = &mut spherical.projection {
        by_key(&mut custom.parameters, |parameter| parameter.name.clone());
    }
    if let Some(sides) =
        (spherical.graticule.as_mut()).and_then(|graticule| graticule.label_sides.as_mut())
    {
        sides.sides.sort_unstable();
        sides.sides.dedup();
    }
}

/// Rewrites each encoding of `mark`.
fn encodings(mark: &mut dyn MarkMessage) {
    for channel in mark.channels() {
        if let Some(encoding) = mark.encoding_mut(*channel) {
            self::encoding(encoding);
        }
    }
}

/// Rewrites an encoding: leaves out the title of an encoding without a scale, which has no axis or
/// legend, orders the options of a custom scale, leaves out the empty fits of its domain, and
/// removes the repeated classes of its axis or legend.
fn encoding(encoding: &mut proto::Encoding) {
    let Some(spec) = &mut encoding.scale else {
        encoding.title = None;
        return;
    };
    if let Some(axis) = spec.axis.as_mut() {
        distinct(&mut axis.classes);
    }
    if let Some(legend) = spec.legend.as_mut() {
        distinct(&mut legend.classes);
    }
    let scale_type = spec.r#type.as_mut().expect("a valid spec has a type");
    if let scale::Type::Custom(custom) = scale_type {
        by_key(&mut custom.options, |option| option.name.clone());
    }
    use selection_domain::Fallback;
    match spec
        .domain
        .as_mut()
        .and_then(|domain| domain.domain.as_mut())
    {
        Some(domain::Domain::Summary(summary))
        | Some(domain::Domain::Selection(proto::SelectionDomain {
            fallback: Some(Fallback::Summary(summary)),
            ..
        })) => empty_fit(&mut summary.fit),
        Some(domain::Domain::Data(data))
        | Some(domain::Domain::Selection(proto::SelectionDomain {
            fallback: Some(Fallback::Data(data)),
            ..
        })) => empty_fit(&mut data.fit),
        _ => {}
    }
}

/// Rewrites the widgets of a layer: distinct classes, and fits that set no field left out.
fn widgets(widgets: &mut [proto::Widget]) {
    use proto::{widget::Kind, widget_domain::Domain};
    for widget in widgets {
        distinct(&mut widget.classes);
        let domain = match widget.kind.as_mut() {
            Some(Kind::Slider(slider)) => slider.domain.as_mut(),
            Some(Kind::RangeSlider(slider)) => slider.domain.as_mut(),
            Some(Kind::Radio(radio)) => radio.domain.as_mut(),
            Some(Kind::Checkboxes(checkboxes)) => checkboxes.domain.as_mut(),
            Some(Kind::Checkbox(_)) | None => None,
        };
        match domain.and_then(|domain| domain.domain.as_mut()) {
            Some(Domain::Summary(summary)) => empty_fit(&mut summary.fit),
            Some(Domain::Data(data)) => empty_fit(&mut data.fit),
            _ => {}
        }
    }
}

/// Leaves out a fit that sets no field, which is the fit that applies when a domain sets none.
fn empty_fit(fit: &mut Option<proto::DomainFit>) {
    let empty = fit.as_ref().is_some_and(|fit| match &fit.fit {
        None => true,
        Some(domain_fit::Fit::Interval(interval)) => *interval == proto::IntervalFit::default(),
        Some(domain_fit::Fit::Categories(categories)) => {
            *categories == proto::CategoryFit::default()
        }
    });
    if empty {
        *fit = None;
    }
}

/// Joins the selections with one name, orders them by name, and orders the initial clauses of
/// each by the tree order of their owners, with one clause for each owner and the partition of
/// each clause by column.
fn selections(
    selections: &mut Vec<proto::Selection>,
    owners: &HashMap<Vec<u8>, usize>,
    values: &[ArrayRef],
) {
    let mut joined: Vec<proto::Selection> = Vec::new();
    for selection in std::mem::take(selections) {
        match joined.iter_mut().find(|known| known.name == selection.name) {
            Some(known) => known.initial.extend(selection.initial),
            None => joined.push(selection),
        }
    }
    joined.sort_by(|a, b| a.name.cmp(&b.name));
    for selection in &mut joined {
        let mut clauses: Vec<(usize, proto::Clause)> = Vec::new();
        for mut clause in std::mem::take(&mut selection.initial) {
            by_key(&mut clause.partition, |value| value.column.clone());
            let owner = Identity {
                id: clause.owner.clone(),
                partition: (clause.partition.iter())
                    .map(|value| {
                        let index = value.value.as_ref().expect("a valid value is set").index;
                        (value.column.clone(), values[index as usize].clone())
                    })
                    .collect(),
            };
            let position = owners[&owner_key(&clause.widget, &owner)];
            // Clauses of one owner are equal in a valid chart.
            if !clauses.iter().any(|(known, _)| *known == position) {
                clauses.push((position, clause));
            }
        }
        clauses.sort_by_key(|(position, _)| *position);
        selection.initial = clauses.into_iter().map(|(_, clause)| clause).collect();
    }
    *selections = joined;
}

/// The tables that the layers of `message` refer to, one copy of each, in the order of the first
/// layer that refers to each, with the layers' `TableRef`s renumbered, and the stream of each,
/// which is the stream of its first copy. Tables with equal streams are equal.
fn canonical_tables(
    message: &mut proto::Chart,
    tables: &[RecordBatch],
    streams: &[Option<Bytes>],
) -> (Vec<RecordBatch>, Vec<Option<Bytes>>) {
    let mut renumbered: HashMap<u32, u32> = HashMap::new();
    let mut ordered: Vec<(RecordBatch, Option<Bytes>)> = Vec::new();
    if let Some(root) = &mut message.root {
        root.visit_tables_mut(&mut |table| {
            let index = *renumbered.entry(table.index).or_insert_with(|| {
                let batch = &tables[table.index as usize];
                let stream = &streams[table.index as usize];
                let same = |(known, known_stream): &(RecordBatch, Option<Bytes>)| {
                    matches!((known_stream, stream), (Some(a), Some(b)) if a == b)
                        || same_table(known, batch)
                };
                match ordered.iter().position(same) {
                    Some(position) => position as u32,
                    None => {
                        ordered.push((batch.clone(), stream.clone()));
                        (ordered.len() - 1) as u32
                    }
                }
            });
            table.index = index;
        });
    }
    ordered.into_iter().unzip()
}

/// Whether two tables are equal: their schemas, with their metadata, and their columns.
fn same_table(a: &RecordBatch, b: &RecordBatch) -> bool {
    a.schema() == b.schema()
        && a.num_rows() == b.num_rows()
        && (a.columns().iter())
            .zip(b.columns())
            .all(|(a, b)| std::sync::Arc::ptr_eq(a, b) || a.as_ref() == b.as_ref())
}

/// The values that the fields of `message` refer to, in the order that `Chart.values` describes,
/// with the fields renumbered: one value for each field, the values of each type together, the
/// types and the values of each type in the order of the first field that refers to them.
fn canonical_values(message: &mut proto::Chart, values: &[ArrayRef]) -> Vec<ArrayRef> {
    let mut references: Vec<usize> = Vec::new();
    message.visit_values(&mut |value| references.push(value.index as usize));
    let mut types: Vec<&DataType> = Vec::new();
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (position, index) in references.iter().enumerate() {
        let data_type = values[*index].data_type();
        let group = match types.iter().position(|known| *known == data_type) {
            Some(group) => group,
            None => {
                types.push(data_type);
                groups.push(Vec::new());
                groups.len() - 1
            }
        };
        groups[group].push(position);
    }
    let mut renumbered = vec![0; references.len()];
    let mut ordered = Vec::with_capacity(references.len());
    for position in groups.into_iter().flatten() {
        renumbered[position] = ordered.len() as u32;
        ordered.push(values[references[position]].clone());
    }
    let mut position = 0;
    message.visit_values_mut(&mut |value| {
        value.index = renumbered[position];
        position += 1;
    });
    ordered
}

/// Removes each class that an earlier class of `classes` repeats, so a node lists each once.
fn distinct(classes: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    classes.retain(|class| seen.insert(class.clone()));
}
