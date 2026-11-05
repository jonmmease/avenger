//! Charts built from proto messages, as a producer without the builders writes them.

#![allow(dead_code)]

use std::sync::Arc;

use arrow::{
    array::{ArrayRef, Float64Array},
    datatypes::{DataType, Field, Schema},
    ipc::writer::{IpcWriteOptions, StreamWriter},
    record_batch::RecordBatch,
};
use avenger_chart::proto::{
    self, cartesian_layer, domain, encoding, node, plot, scale_spec, CartesianLayer, CartesianPlot,
    DataDomain, Domain, Encoding, LinearScale, Node, Plot, ScaleSpec, SymbolMark, TableRef,
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
pub fn linear(domain: domain::Domain) -> ScaleSpec {
    ScaleSpec {
        r#type: Some(scale_spec::Type::Linear(LinearScale::default())),
        domain: Some(Domain {
            domain: Some(domain),
        }),
        ..Default::default()
    }
}

/// A linear scale fitted to the layer's data.
pub fn linear_data() -> ScaleSpec {
    linear(domain::Domain::Data(DataDomain {}))
}

/// An encoding of the column `name` through `scale`.
pub fn column(name: &str, scale: ScaleSpec) -> Encoding {
    Encoding {
        value: Some(encoding::Value::Column(name.to_string())),
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
