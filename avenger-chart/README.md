# avenger-chart

The avenger chart format and its reader. A chart is a protobuf message of package `avenger.chart.v1`: a tree of plots, concats, and wraps over Arrow data tables, with the selections that its plots and widgets write and read. A chart holds no runtime state and no size of its own, so a renderer draws it at any size, and a chart renders the same after a round trip through bytes. The crate depends on prost, Arrow, and thiserror only.

## The format

The `.proto` files are in `proto/avenger/chart/v1/`, and their comments are the specification of the format. The comment of `Chart` in `chart.proto` holds the rules that apply to every message. prost generates the messages of the `proto` module from the files, with the comments as their documentation, and build.rs generates a builder for each message beside it, so a producer in Rust builds a chart with the builders, and a producer in another language generates its own messages from the same files.

The data tables are Arrow IPC streams inside the message. A layer refers to its table by a `TableRef`, the position of the table in `Chart.tables`. The other Arrow values of a chart, such as constants, explicit domains, domain summaries, and partition values, are scalars that the fields of the message refer to by a `ValueRef`, so each value keeps its exact Arrow type.

The schema decides the shape of a chart: a scale always has a type, a plot always has a coordinate system, and the oneofs hold the kinds of each. A rule that spans messages accepts each way of writing one chart that has one meaning. Keyed lists, such as resolve rules and partition values, come in any order, and two equal entries with one key count as one. Partition values can be declared on any node above the layers that hold them. Selections with one name join. Equal copies of a table can stand in for one table. Two different entries with one key are a conflict, as are two layers whose shared scale has specs that disagree, and two copies of one widget that disagree.

## Reading a chart

Every constructor of a `Chart` decodes the chart, checks it against every rule of the format, and rewrites it into one canonical form:

| Constructor | Input |
|---|---|
| `Chart::from_bytes` | The bytes of a serialized chart |
| `Chart::from_message` | A `proto::Chart` whose tables and values are IPC streams inside it |
| `Chart::from_parts` | A `proto::Chart` with its tables as `RecordBatch`es and its values as `ArrayRef`s, for a producer that holds its Arrow data already |

A chart that breaks a rule, or two parts that conflict, give a `ChartError::Invalid` that names the path of the field and the rule, such as `root.plot.cartesian.layers: a plot has a layer or more`. The checks that need a renderer are left to the renderer: the names of custom scales and custom projections, and whether its text engine has formatters of the chart's format syntaxes.

The canonical form:

- declares each partition value once, on the highest node whose layers and empty cells all hold it
- joins the selections with one name, orders the selections by name, and orders the initial clauses of each selection by the tree order of their owners
- orders each other keyed list by its key, with one entry for each key
- leaves out the fields that a comment says a reader ignores, such as the width of a plot in a cell of a wrap, and the option messages whose fields are all at their defaults where the comment says that leaving the message out means each default
- holds one copy of each table, in the order of the first layer that refers to it, and the values in the order that `Chart.values` describes

So the ways of writing one chart read as equal charts, and `Chart::to_bytes` writes them to equal bytes, unless their Arrow data holds equal values in different buffers, such as different bytes under a null. A field set to the default that its comment gives when it is not set stays as it is written. The writer places each table and the values at a multiple of 64 bytes, and `Chart::from_bytes` decodes their columns in place in its input, without a copy.

```rust
use std::sync::Arc;

use arrow::{
    array::{ArrayRef, Float64Array},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use avenger_chart::{
    proto::{self, cartesian_layer, encoding, node, plot},
    Chart, ChartError, FORMAT_VERSION,
};

// A table of two columns.
let schema = Schema::new(vec![
    Field::new("a", DataType::Float64, false),
    Field::new("b", DataType::Float64, false),
]);
let columns: Vec<ArrayRef> = vec![
    Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])),
    Arc::new(Float64Array::from(vec![4.0, 6.0, 5.0])),
];
let table = RecordBatch::try_new(Arc::new(schema), columns)?;

// A scatter plot of `a` against `b`. The reader infers a linear scale of each, fitted to the data.
let column = |name: &str| proto::Encoding {
    kind: Some(encoding::Kind::ScaledField(name.to_string())),
    ..Default::default()
};
let layer = proto::CartesianLayer {
    table: Some(proto::TableRef { index: 0 }),
    mark: Some(cartesian_layer::Mark::Symbol(proto::SymbolMark {
        x: Some(column("a")),
        y: Some(column("b")),
        ..Default::default()
    })),
    ..Default::default()
};
let message = proto::Chart {
    version: FORMAT_VERSION,
    root: Some(proto::Node {
        node: Some(node::Node::Plot(proto::Plot {
            coord: Some(plot::Coord::Cartesian(proto::CartesianPlot {
                layers: vec![layer],
                ..Default::default()
            })),
            ..Default::default()
        })),
    }),
    ..Default::default()
};

let chart = Chart::from_parts(message.clone(), vec![table.clone()], vec![])?;
let bytes = chart.to_bytes()?;
assert_eq!(Chart::from_bytes(bytes)?, chart);

// A plot without layers breaks a rule, and the error names the field.
let mut empty = message;
if let Some(node::Node::Plot(plot)) = &mut empty.root.as_mut().unwrap().node {
    if let Some(plot::Coord::Cartesian(cartesian)) = &mut plot.coord {
        cartesian.layers.clear();
    }
}
match Chart::from_parts(empty, vec![table], vec![]) {
    Err(ChartError::Invalid { path, .. }) => assert_eq!(path, "root.plot.cartesian.layers"),
    other => panic!("expected an invalid chart, got {other:?}"),
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Producing charts

Each message `Foo` of `proto` has a builder, `FooBuilder`, whose methods set the fields of the message and nothing else: a builder applies no defaults, so an absent field keeps the meaning that its comment gives. The constructors take the content that the `required` and `required_oneof` options of `annotation.proto` mark as required, so a builder cannot leave it out:

| Message | Constructors |
|---|---|
| No required content | `Foo::builder()`, such as `BarMark::builder()` |
| Required fields | `Foo::new(..)`, with the required fields in the order of their numbers, such as `Selection::new("brush", SelectionResolution::Global)` |
| A required oneof | One constructor for each member, which takes the required fields first and the member last, such as `ConditionalValue::direct_value(predicate, "lightgray")` or `CartesianLayer::symbol(table, mark)` |

Each member of a oneof that is not required also has a constructor, which takes the required fields first, such as `Scale::band(..)` or `Select::interval("brush", ..)`. A member whose message has no fields takes no argument, such as `Scale::quantile()`, and a member whose message has one repeated field takes its items, such as `Predicate::all([..])`.

Each field and member that is not required content has one method, named after it, with the field's comment as its documentation:

| Field | Method takes |
|---|---|
| A scalar | Its exact Rust type, `impl Into<String>` for a string, or `impl Into<Bytes>` for bytes |
| An enum | The Rust enum, such as `Orient::Vertical` |
| An encoding of a channel | `impl IntoEncoding`: a string, a number, a bool, or an `EncodingBuilder` |
| Another message | `impl Into` of its builder |
| A `TableRef` | A `RecordBatch` |
| A `ValueRef` | `impl IntoValue`: a Rust scalar, which becomes one element of its own Arrow type, an array, a `Vec`, or a slice of them, which becomes a list, or a `builder::Value` of any Arrow type |
| A repeated field | `impl IntoIterator` of the items, which it appends |

An encoding holds one of four kinds of values: a `scaled_field` or a `direct_field`, a column of the layer's table, and a `scaled_value` or a `direct_value`, one value. A scaled value goes through the channel's scale, and a direct value is what the mark draws, such as a CSS color. A string names a column: a scaled field on a channel that has a scale or follows the scale of its partner, such as `x`, `color`, or `x2`, and a direct field on a channel of direct values, such as `text`, `longitude`, or `width`, as the `channel_scale` option of the channel's value says. A number is a scaled value on a position of a coordinate system with a scale, such as `.x(2.5)` for a reference line at 2.5 in the data, and a direct value on any other channel, such as `.size(100.0)`, as the `position` option of the channel's value says. A bool is a direct value. Any other value is explicit, such as `Encoding::direct_value("#e45756")`, since `.color("#e45756")` names a column, and the error of a missing column says so.

A builder converts from other types where the conversion is unambiguous:

- A message whose only required content is one field converts from that field's argument, so `Plot::cartesian([layer])`, `Predicate::selected("brush")`, and `Scale::custom("dots")` build the message with that field.
- A message with one oneof and no required field converts from each member that is a message of its own type, so `.scale(BandScale::builder())` and `Chart::builder(plot)` set the member.
- A scale converts from its domain, its range, its axis, and its legend, the fields that the `lift` option marks, so `.scale(Domain::explicit([0.0, 10.0]))` and `.scale(Axis::builder().show_grid(true))` set that field. A conversion is one step, so it takes part in no other conversion.

`EncodingBuilder::no_axis`, `EncodingBuilder::no_legend`, and the same methods of `ScaleBuilder` hide the axis or the legend of a scale, and `no_guides` on the builders of Cartesian, polar, and map plots hides the axes and the legends of the scales of the plot's layers.

A builder holds the tables and the values that its fields take, and `ChartBuilder::build` writes them into the chart with the references to them and the format version, then reads the chart, which infers its absent scales, checks it, and puts it in the canonical form, as every reader does. A builder of a message other than a chart builds alone when it holds no data, such as a scale for a unit test.

The reader infers the absent scale of each encoding of a scaled value, and the absent type and domain of its scale, so a chart that another language writes gets the defaults that a builder's chart gets: a position of numbers takes a linear scale, of dates and timestamps a time scale, and of categories a band scale for bars and rects and a point scale for other marks, categories on another channel take an ordinal scale, and an absent domain is the layer's data domain. The comments of `Scale.type` and `Scale.domain` give the rules. Inference leaves fits, sort orders, axes, legends, titles, and styles to the producer and the renderer's stylesheets, and it never changes a field that is set.

A producer in another language reads the same rules from the descriptors: the `required` and `required_oneof` options of the fields and the oneofs, the `lift` option of the fields of `Scale`, and the `channel_scale` and `position` options of the values of `Channel`.

The builders also compose charts:

| Method | What it does |
|---|---|
| `From<Chart> for NodeBuilder` | A finished chart as a node, such as a child of a concat or a wrap, whose selections join the chart's and whose format syntaxes must agree with the chart's; its tables keep their IPC streams, and its width, height, and padding do not carry over |
| `Plot::overlay`, `PlotBuilder::overlay` | Plots whose layers join the plot, after its own, where the plot's own fields win and the overlaid plots' fields must agree |
| `NodeBuilder::partition` | Partition values on the node's member, such as the cell of a finished chart in a facet |
| `Node::partitions` | One node for each value of a column of a table, in the order in which the values appear or sorted, from the rows that hold the value |

Every error of a builder, such as two overlaid plots with different titles, is a `ChartError::Invalid` at the path of its field in the chart.

`use avenger_chart::prelude::*` imports everything that building a chart takes: the checked `Chart`, every message and its builder, every enum of the format, `Value`, `IntoValue`, `IntoEncoding`, `PartitionOrder`, and `ChartError`. The prelude names the message `proto::Chart`, the wire form of a chart, `ChartMessage`, so `Chart` is the checked chart, whose `Chart::builder(root)` builds one.

```rust
use std::sync::Arc;

use arrow::{
    array::{ArrayRef, Float64Array, StringArray},
    record_batch::RecordBatch,
};
use avenger_chart::prelude::*;

let table = RecordBatch::try_from_iter([
    ("species", Arc::new(StringArray::from(vec!["Adelie", "Gentoo"])) as ArrayRef),
    ("count", Arc::new(Float64Array::from(vec![152.0, 124.0])) as ArrayRef),
])?;

// Bars of `count` by `species`: a band scale of the species and a linear scale of the counts,
// which the reader infers, and whose domain contains zero.
let fit = DataDomain::builder().fit(IntervalFit::builder().zero(true));
let bars = CartesianLayer::bar(
    table,
    BarMark::builder()
        .x("species")
        .y(Encoding::scaled_field("count").scale(Scale::builder().domain(fit))),
);
let chart = Chart::builder(Plot::cartesian([bars])).build()?;

// Two copies of the chart side by side, which share one table.
let both = Chart::builder(Concat::new(Direction::Row).children([chart.clone(), chart])).build()?;
assert_eq!(both.tables().len(), 1);
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Models

A renderer or an app reads a chart through `Chart::resolve`, which builds the models that a reader also builds to check the rules that span messages. A chart is valid, so it always resolves.

| Field of `Resolved` | What it holds |
|---|---|
| `tree` | The plots, the layers, and the concats, wraps, and facet headers with ids, in tree order, with their paths, their identities, and their partitions: the values that apply to every layer and empty cell below each node |
| `scales` | The scales: the encodings that share each scale by the resolve rules and the share families, with the spec that merges their specs, the node that owns the scale's axis or legend, and its identity |
| `projections` | The map plots that share each projection |
| `tables` | The place of each node in its table of tracks, with the length that plots and concats fix for each track |
| `matches` | The tracks whose lengths the plots of shared position scales and projections match |
| `widgets` | The widgets: the copies of each key that share one widget by the resolve rules, with the declaration that merges theirs, the anchor that identifies the widget, and the node that holds its copies, whose part rules decide how many times it draws |

The `layer` module gives a view of a layer of any coordinate system, and its mark's encodings by channel.

Plots, layers, concats, wraps, axes, legends, and facet headers share one namespace of identities, and two parts with one id and one partition are a conflict whatever their kinds, so an identity names one part of a chart. An app carries the views and the selections of a plot to a new chart by its identity, its id and its partition, so the plot of a facet keeps them when a new query reorders the facets.

A widget is not one of these parts. Layers declare widgets under keys, which repeat: the declarations of one key are copies, and the resolve rules decide how many widgets they make, as they decide how many scales the encodings of a channel make. A widget's identity is its key with its anchor, the child or the layer that an independent rule made its group for, so a widget that the whole chart shares is its key alone, and the widgets of the facets of a facet grid are its key with each facet's plot. A widget's clause names its key in `Clause.widget` and its anchor in `Clause.owner` and `Clause.partition`, so it keeps its value across chart updates as a brush does.

## Columns of charts

A column of charts is a `Binary`, `LargeBinary`, or `BinaryView` column whose field has the extension name `avenger.chart`, which `chart_field` gives. `charts_to_arrow` writes charts to a `BinaryView` column, with each chart at a multiple of 64 bytes, and `charts_from_arrow` reads them, in place or from a copy as `Decode` says.
