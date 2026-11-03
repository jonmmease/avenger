//! The plans that `selected` lowers to: no filter for an empty selection, and otherwise one row
//! test of the selection, with no other predicate.

use std::sync::Arc;

use avenger_selection::{Clause, Condition, Resolution, SelectionState};
use datafusion::{
    arrow::array::{ArrayRef, Float64Array, Int64Array, RecordBatch, StringArray},
    physical_plan::displayable,
    prelude::SessionContext,
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
    let mut ctx = SessionContext::new();
    avenger_selection_datafusion::register(&mut ctx).unwrap();
    ctx.register_batch("t", table()).unwrap();
    let plan = ctx.sql(sql).await.unwrap().into_unoptimized_plan();
    let plan = avenger_selection_datafusion::bind_selections(plan, &[("s", state)]).unwrap();
    let frame = ctx.execute_logical_plan(plan).await.unwrap();
    let plan = frame.create_physical_plan().await.unwrap();
    let text = displayable(plan.as_ref()).indent(true).to_string();
    text
}

#[tokio::test]
async fn an_empty_selection_has_no_filter() {
    let plan = plan(
        "SELECT id FROM t WHERE selected($s)",
        &SelectionState::default(),
    )
    .await;
    assert!(!plan.contains("FilterExec"), "{plan}");
}

/// A union of a range and a set lowers to one row test, which takes only the conditions' columns,
/// with no comparison or list beside it.
#[tokio::test]
async fn a_selection_lowers_to_one_row_test() {
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
    assert!(
        filter.contains("selection_matches(x@1, species@2)"),
        "{filter}"
    );
    for absent in [" OR ", "BETWEEN", " IN ", ">=", "<=", "resolution", "{"] {
        assert!(!filter.contains(absent), "{absent} in {filter}");
    }
}
