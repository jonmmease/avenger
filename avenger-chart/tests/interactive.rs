//! Whether a chart declares what a reader acts on: a tool of a plot or of a layer, or a widget,
//! anywhere in its tree.

use std::sync::Arc;

use arrow::{
    array::{ArrayRef, Int64Array, StringArray},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use avenger_chart::prelude::*;

fn rows() -> RecordBatch {
    let schema = Schema::new(vec![
        Field::new("origin", DataType::Utf8, false),
        Field::new("cars", DataType::Int64, false),
    ]);
    let origin: ArrayRef = Arc::new(StringArray::from(vec!["Europe", "USA"]));
    let cars: ArrayRef = Arc::new(Int64Array::from(vec![3, 8]));
    RecordBatch::try_new(Arc::new(schema), vec![origin, cars]).unwrap()
}

fn bars() -> PlotBuilder {
    let mark = BarMark::builder()
        .x(Encoding::scaled_field("origin"))
        .y(Encoding::scaled_field("cars"));
    Plot::cartesian([CartesianLayer::bar(rows(), mark)])
}

fn chart(root: impl Into<NodeBuilder>) -> Chart {
    Chart::builder(root).build().unwrap()
}

#[test]
fn a_chart_with_a_tool_anywhere_in_its_tree_is_interactive() {
    assert!(!chart(bars()).is_interactive());
    let navigated = || bars().tools([ToolBuilder::from(Navigate::builder())]);
    assert!(chart(navigated()).is_interactive());
    // A plot of a concat.
    let row = Concat::new(Direction::Row).children([NodeBuilder::from(bars()), navigated().into()]);
    assert!(chart(row).is_interactive());
    let still = Concat::new(Direction::Row).children([NodeBuilder::from(bars()), bars().into()]);
    assert!(!chart(still).is_interactive());
    // Modes without a widget or a tool give the reader nothing to act on.
    let modes = Chart::builder(bars())
        .modes([Mode::new("pan")])
        .build()
        .unwrap();
    assert!(!modes.is_interactive());
}
