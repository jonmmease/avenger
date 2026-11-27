//! The plans of a `selected` call: one operator that appends the row test of the
//! selection, which a filter reads, with no other predicate.

mod common;

use std::sync::Arc;

use avenger_selection::{Clause, Condition, Resolution, SelectionState};
use datafusion::{
    arrow::array::{ArrayRef, Float64Array, Int64Array, RecordBatch, StringArray},
    physical_plan::displayable,
};

/// A table of `x` from 0 to 99 and `species`, in memory.
fn table() -> RecordBatch {
    RecordBatch::try_from_iter([
        (
            "id",
            Arc::new(Int64Array::from_iter_values(0..100)) as ArrayRef,
        ),
        (
            "x",
            Arc::new(Float64Array::from_iter_values((0..100).map(f64::from))),
        ),
        (
            "species",
            Arc::new(StringArray::from_iter_values(
                (0..100).map(|i| ["adelie", "gentoo"][i / 50]),
            )),
        ),
    ])
    .unwrap()
}

/// The physical plan of `sql`, with the state `state` bound to `$s`.
async fn plan(sql: &str, state: &SelectionState) -> String {
    let ctx = common::session();
    ctx.register_batch("t", table()).unwrap();
    let plan = ctx.sql(sql).await.unwrap().into_unoptimized_plan();
    let frame = common::frame(&ctx, plan, &[("s", state)]).unwrap();
    let plan = frame.create_physical_plan().await.unwrap();
    let text = displayable(plan.as_ref()).indent(true).to_string();
    text
}

/// An empty selection binds its operator to a test that keeps every row, which the filter reads.
#[tokio::test]
async fn an_empty_selection_binds_its_operator() {
    let plan = plan(
        "SELECT id FROM t WHERE selected($s)",
        &SelectionState::default(),
    )
    .await;
    assert!(plan.contains("SelectionExec: $s AS __selected_"), "{plan}");
    assert!(!plan.contains("unbound"), "{plan}");
}

/// A union of a range and a set is one operator, and the filter above it reads the operator's
/// column, with no comparison or list beside it.
#[tokio::test]
async fn a_selection_is_one_operator() {
    let range = Condition::Range {
        column: "x".into(),
        channel: None,
        ends: Arc::new(Float64Array::from(vec![12.0, 18.0])),
        exclude_end: false,
        cells: None,
    };
    let set = Condition::values("species", None, Arc::new(StringArray::from(vec!["gentoo"])));
    let state = SelectionState::new(
        Resolution::Union,
        vec![
            Clause::new("a").condition(range),
            Clause::new("b").condition(set),
        ],
    );
    let plan = plan("SELECT id FROM t WHERE selected($s)", &state).await;
    let filter = plan
        .lines()
        .find(|line| line.contains("FilterExec"))
        .unwrap_or_else(|| panic!("a filter: {plan}"));
    assert!(filter.contains("__selected_"), "{filter}");
    assert!(plan.contains("SelectionExec: $s AS __selected_"), "{plan}");
    for absent in [" OR ", "BETWEEN", " IN ", ">=", "<=", "resolution", "{"] {
        assert!(!filter.contains(absent), "{absent} in {filter}");
    }
}
