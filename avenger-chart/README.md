# avenger-chart

The avenger chart format and its reader. A chart is a protobuf message of package `avenger.chart.v1`: a tree of plots, concats, and wraps over Arrow data tables, with the selections that its plots write and read. A chart holds no runtime state and no size of its own, so a renderer draws it at any size, and a chart renders the same after a round trip through bytes. The crate depends on prost, Arrow, and thiserror only.

## The format

The `.proto` files are in `proto/avenger/chart/v1/`, and their comments are the specification of the format. The comment of `Chart` in `chart.proto` holds the rules that apply to every message. prost generates the messages of the `proto` module from the files, with the comments as their documentation, so a producer in Rust builds a chart from the messages, and a producer in another language generates its own from the same files.

The data tables are Arrow IPC streams inside the message. A layer refers to its table by a `TableRef`, the position of the table in `Chart.tables`. The other Arrow values of a chart, such as constants, explicit domains, domain summaries, and partition values, are scalars that the fields of the message refer to by a `ValueRef`, so each value keeps its exact Arrow type.

The schema decides the shape of a chart: a scale always has a type, a plot always has a coordinate system, and the oneofs hold the kinds of each. A rule that spans messages accepts each way of writing one chart that has one meaning. Keyed lists, such as resolve rules and partition values, come in any order, and two equal entries with one key count as one. Partition values can be declared on any node above the layers that hold them. Selections with one name join. Equal copies of a table can stand in for one table. Two different entries with one key are a conflict, as are two layers whose shared scale has specs that disagree.

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
    proto::{self, cartesian_layer, domain, encoding, node, plot, scale_spec},
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

// A scatter plot of `a` against `b`, each through a linear scale fitted to the data.
let column = |name: &str| proto::Encoding {
    value: Some(encoding::Value::Column(name.to_string())),
    scale: Some(proto::ScaleSpec {
        r#type: Some(scale_spec::Type::Linear(proto::LinearScale::default())),
        domain: Some(proto::Domain {
            domain: Some(domain::Domain::Data(proto::DataDomain::default())),
        }),
        ..Default::default()
    }),
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

A producer writes the messages of `proto`, and the module `build` does the work that every producer would otherwise repeat:

| Helper | What it does |
|---|---|
| `ChartParts` | Collects the tables and the values of a chart, hands out the `TableRef`s, the `ValueRef`s, and the partition values that refer to them, and reads the finished chart with its inferred scales |
| `Fragment` | A message with the values that its `ValueRef`s refer to, such as the options of a node, which `ChartParts::embed` adds to a chart. A message without values converts to a fragment |
| `concat`, `wrap`, `facet_grid`, `overlay` | Combine finished charts into a row or a column, a wrap, a grid of facets, or one plot, by the combination rules of the format |
| `with_partition` | Declares a partition value on the root of a finished chart |
| `infer_scales` | Infers the scale types and the domains that the marks and the types of their values imply, which `ChartParts::finish` does for its chart |

A combination copies the tables of its children: a chart read from bytes writes the IPC stream of each table again, unchanged, so a facet grid of charts from bytes does not encode its data again. A combination that breaks a rule of its own, such as an overlay of plots of two coordinate systems, fails with `ChartError::Combine`, which names the position of the chart that breaks it among the charts that combine. A combined chart that breaks a rule of the format fails as any chart does, with `ChartError::Invalid` and the path of the field.

`infer_scales` infers only by the conventions that the common charting tools share. A position of numbers takes a linear scale, of dates and timestamps a time scale, and of categories a band scale for bars and rects and a point scale for other marks. Categories on another channel take an ordinal scale, and an absent domain is the layer's data domain. An encoding of `x` or `y` always has a scale. An encoding of another channel gets one only when it holds a scale whose type is absent, such as the scale of `build::scaled_column`, because an encoding without a scale draws its values as they are. `infer_scales` leaves `zero`, `nice`, sort orders, guides, titles, and styles to the producer and the renderer's stylesheets, and it leaves the kind of a discrete scale absent. It never changes a field that the producer set.

```rust
use std::sync::Arc;

use arrow::{
    array::{ArrayRef, Float64Array, StringArray},
    record_batch::RecordBatch,
};
use avenger_chart::{
    build::{self, ChartParts},
    proto::{self, cartesian_layer::Mark},
};

let table = RecordBatch::try_from_iter([
    ("species", Arc::new(StringArray::from(vec!["Adelie", "Gentoo"])) as ArrayRef),
    ("count", Arc::new(Float64Array::from(vec![152.0, 124.0])) as ArrayRef),
])?;

// Bars of `count` by `species`, whose scales `infer_scales` infers: a band scale of the species
// and a linear scale of the counts.
let mut parts = ChartParts::new();
let layer = proto::CartesianLayer {
    table: Some(parts.table(table)),
    mark: Some(Mark::Bar(proto::BarMark {
        x: Some(build::column("species")),
        y: Some(build::column("count")),
        ..Default::default()
    })),
    ..Default::default()
};
let bars = parts.finish(build::message(build::cartesian(vec![layer])))?;

// Two copies of the chart side by side, which share one table.
let row = proto::Concat {
    direction: proto::Direction::Row as i32,
    ..Default::default()
};
let both = build::concat(row, vec![bars.clone(), bars])?;
assert_eq!(both.tables().len(), 1);
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Models

A renderer or an app reads a chart through `Chart::resolve`, which builds the models that a reader also builds to check the rules that span messages. A chart is valid, so it always resolves.

| Field of `Resolved` | What it holds |
|---|---|
| `tree` | The plots, the layers, and the concats, wraps, and facet headers with ids, in tree order, with their paths, their identities, and their partitions: the values that apply to every layer and empty cell below each node |
| `scales` | The scales: the encodings that share each scale by the resolve rules and the share families, with the spec that merges their specs, the node that owns the scale's guides, and the identity of its guide |
| `projections` | The map plots that share each projection |
| `tables` | The place of each node in its table of tracks, with the length that plots and concats fix for each track |
| `matches` | The tracks whose lengths the plots of shared position scales and projections match |

The `layer` module gives a view of a layer of any coordinate system, and its mark's encodings by channel.

Plots, layers, concats, wraps, guides, and facet headers share one namespace of identities, and two parts with one id and one partition are a conflict whatever their kinds, so an identity names one part of a chart. An app carries the views and the selections of a plot to a new chart by its identity, its id and its partition, so the plot of a facet keeps them when a new query reorders the facets.

## Columns of charts

A column of charts is a `Binary`, `LargeBinary`, or `BinaryView` column whose field has the extension name `avenger.chart`, which `chart_field` gives. `charts_to_arrow` writes charts to a `BinaryView` column, with each chart at a multiple of 64 bytes, and `charts_from_arrow` reads them, in place or from a copy as `Decode` says.
