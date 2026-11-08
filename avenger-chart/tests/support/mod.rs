//! Charts built from proto messages, as a producer without the builders writes them.

#![allow(dead_code)]

use std::sync::Arc;

use arrow::{
    array::{ArrayRef, Float64Array, StringArray},
    datatypes::{DataType, Field, Schema},
    ipc::writer::{IpcWriteOptions, StreamWriter},
    record_batch::RecordBatch,
};
use avenger_chart::{
    proto::{
        self, cartesian_layer, domain, encoding, node, plot, scale, select, tool, CartesianLayer,
        CartesianPlot, Concat, DataDomain, Domain, Encoding, LinearScale, Node, Plot, Scale,
        Selection, SymbolMark, TableRef, ValueRef,
    },
    Chart, ChartError,
};

/// A table of `Float64` columns.
pub fn table(columns: &[(&str, Vec<f64>)]) -> RecordBatch {
    let fields: Vec<Field> = (columns.iter())
        .map(|(name, _)| Field::new(*name, DataType::Float64, false))
        .collect();
    let arrays: Vec<ArrayRef> = (columns.iter())
        .map(|(_, values)| Arc::new(Float64Array::from(values.clone())) as ArrayRef)
        .collect();
    RecordBatch::try_new(Arc::new(Schema::new(fields)), arrays).unwrap()
}

/// The table of a scatter plot of `a` against `b`.
pub fn points() -> RecordBatch {
    table(&[("a", vec![1.0, 2.0, 3.0]), ("b", vec![4.0, 6.0, 5.0])])
}

/// A linear scale whose domain is `domain`.
pub fn linear(domain: domain::Domain) -> Scale {
    Scale {
        r#type: Some(scale::Type::Linear(LinearScale::default())),
        domain: Some(Domain {
            domain: Some(domain),
        }),
        ..Default::default()
    }
}

/// A linear scale fitted to the layer's data.
pub fn linear_data() -> Scale {
    linear(domain::Domain::Data(DataDomain::default()))
}

/// The fit of the data domain of `spec`, to change.
pub fn data_fit(spec: &mut Scale) -> &mut Option<proto::DomainFit> {
    match spec
        .domain
        .as_mut()
        .and_then(|domain| domain.domain.as_mut())
    {
        Some(domain::Domain::Data(data)) => &mut data.fit,
        _ => panic!("the spec has a data domain"),
    }
}

/// An encoding of the column `name` through `scale`.
pub fn column(name: &str, scale: Scale) -> Encoding {
    Encoding {
        kind: Some(encoding::Kind::ScaledField(name.to_string())),
        scale: Some(scale),
        ..Default::default()
    }
}

/// A layer of symbols at `a` and `b` of table `table`.
pub fn symbols(table: u32) -> CartesianLayer {
    CartesianLayer {
        table: Some(TableRef { index: table }),
        mark: Some(cartesian_layer::Mark::Symbol(SymbolMark {
            x: Some(column("a", linear_data())),
            y: Some(column("b", linear_data())),
            ..Default::default()
        })),
        ..Default::default()
    }
}

/// A Cartesian plot of `layers`.
pub fn plot(layers: Vec<CartesianLayer>) -> Node {
    Node {
        node: Some(node::Node::Plot(Plot {
            coord: Some(plot::Coord::Cartesian(CartesianPlot {
                layers,
                ..Default::default()
            })),
            ..Default::default()
        })),
    }
}

/// A chart of version 1 with `root`.
pub fn chart(root: Node) -> proto::Chart {
    proto::Chart {
        version: 1,
        root: Some(root),
        ..Default::default()
    }
}

/// The IPC stream of `batch`, written with `options`.
pub fn ipc(batch: &RecordBatch, options: IpcWriteOptions) -> Vec<u8> {
    let mut writer =
        StreamWriter::try_new_with_options(Vec::new(), &batch.schema(), options).unwrap();
    writer.write(batch).unwrap();
    writer.finish().unwrap();
    writer.into_inner().unwrap()
}

pub type Parts = (proto::Chart, Vec<RecordBatch>, Vec<ArrayRef>);

pub fn read(parts: Parts) -> Result<Chart, ChartError> {
    Chart::from_parts(parts.0, parts.1, parts.2)
}

#[track_caller]
pub fn invalid(parts: Parts, path: &str, reason: &str) {
    match read(parts) {
        Err(ChartError::Invalid {
            path: at,
            reason: why,
        }) => {
            assert_eq!(at, path, "the error names the path ({why})");
            assert!(why.contains(reason), "`{why}` says `{reason}`");
        }
        other => panic!("expected an error at {path}, got {other:?}"),
    }
}

/// A plot of `layers` with `id` and `partition`.
pub fn plot_with(
    id: &str,
    partition: Vec<proto::PartitionValue>,
    layers: Vec<CartesianLayer>,
) -> Node {
    let mut node = plot(layers);
    if let Some(node::Node::Plot(plot)) = node.node.as_mut() {
        plot.id = id.to_string();
        plot.partition = partition;
    }
    node
}

pub fn plot_mut(node: &mut Node) -> &mut Plot {
    match node.node.as_mut() {
        Some(node::Node::Plot(plot)) => plot,
        _ => panic!("the node is a plot"),
    }
}

/// A row or a column of `children`.
pub fn concat(direction: proto::Direction, children: Vec<Node>) -> Node {
    Node {
        node: Some(node::Node::Concat(Concat {
            direction: direction as i32,
            children,
            ..Default::default()
        })),
    }
}

pub fn concat_mut(node: &mut Node) -> &mut Concat {
    match node.node.as_mut() {
        Some(node::Node::Concat(concat)) => concat,
        _ => panic!("the node is a concat"),
    }
}

/// A partition value of `column` that refers to value `index`.
pub fn value(column: &str, index: u32) -> proto::PartitionValue {
    proto::PartitionValue {
        column: column.to_string(),
        value: Some(ValueRef { index }),
    }
}

pub fn strings(values: &[&str]) -> Vec<ArrayRef> {
    (values.iter())
        .map(|value| Arc::new(StringArray::from(vec![*value])) as ArrayRef)
        .collect()
}

pub fn parts(root: Node, values: Vec<ArrayRef>) -> Parts {
    (chart(root), vec![points()], values)
}

/// The scale of `channel` of the symbols of `layer`.
pub fn scale_of(layer: &mut CartesianLayer, y: bool) -> &mut proto::Scale {
    match layer.mark.as_mut() {
        Some(cartesian_layer::Mark::Symbol(mark)) => {
            let encoding = if y { mark.y.as_mut() } else { mark.x.as_mut() };
            encoding.unwrap().scale.as_mut().unwrap()
        }
        _ => panic!("the layer draws symbols"),
    }
}

pub fn brush(resolution: proto::SelectionResolution) -> Selection {
    Selection {
        name: "brush".to_string(),
        resolution: resolution as i32,
        ..Default::default()
    }
}

/// A plot `scatter` that selects `brush` by boxes.
pub fn selecting() -> Node {
    let mut node = plot_with("scatter", vec![], vec![symbols(0)]);
    plot_mut(&mut node).tools = vec![proto::Tool {
        kind: Some(tool::Kind::Select(proto::Select {
            selection: "brush".to_string(),
            form: Some(select::Form::Interval(proto::IntervalSelect::default())),
            ..Default::default()
        })),
        ..Default::default()
    }];
    node
}
