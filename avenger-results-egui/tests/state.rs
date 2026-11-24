//! The line that names what a gesture on a chart changed.

use std::collections::BTreeMap;
use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array, StringArray};
use avenger_chart::proto::Channel;
use avenger_chart::resolve::{Id, Identity};
use avenger_chart_app::ChartChange;
use avenger_chart_eval::ScaleIdentity;
use avenger_results_egui::{describe_change, plot_name, ChartState, ScaleState};
use avenger_selection::{Clause, Condition};

fn numbers(values: &[f64]) -> ArrayRef {
    Arc::new(Float64Array::from(values.to_vec()))
}

#[test]
fn a_change_names_its_selections_scales_and_modes() {
    let brush = Clause {
        owner: "plot".into(),
        partition: BTreeMap::new(),
        conditions: vec![Condition::Range {
            column: "Horsepower".into(),
            channel: Some("x".into()),
            ends: numbers(&[80.0, 140.0]),
            exclude_end: false,
            cells: None,
        }],
    };
    let click = Clause {
        conditions: vec![Condition::Set {
            columns: vec!["Origin".into()],
            channels: vec![None],
            values: vec![Arc::new(StringArray::from(vec!["USA", "Japan"]))],
            complement: false,
        }],
        ..brush.clone()
    };
    let change = ChartChange {
        selections: vec![("brush".into(), vec![brush]), ("pick".into(), vec![click])],
        domains: vec![(
            ScaleIdentity::Plot {
                plot: Identity {
                    id: Id::Given("cars".into()),
                    partition: BTreeMap::new(),
                },
                channel: Channel::X,
                index: 0,
            },
            numbers(&[50.0, 200.0]),
        )],
        modes: Some(vec!["select".into()]),
        settled: true,
        ..ChartChange::default()
    };
    let state = ChartState::default();
    assert_eq!(
        describe_change(&change, &state).unwrap(),
        "brush: Horsepower 80.0 to 140.0, pick: Origin in USA, Japan, x of cars: 50.0 to 200.0, \
         modes: select"
    );
    // A clear selection, and a change of nothing.
    let clear = ChartChange {
        selections: vec![("brush".into(), Vec::new())],
        ..ChartChange::default()
    };
    assert_eq!(describe_change(&clear, &state).unwrap(), "brush: clear");
    assert_eq!(describe_change(&ChartChange::default(), &state), None);
    // A scale of the chart's state takes its name there, as a scale of several facet cells does.
    let identity = ScaleIdentity::Plot {
        plot: Identity {
            id: Id::Path("root.facet.plot".into()),
            partition: BTreeMap::new(),
        },
        channel: Channel::Y,
        index: 0,
    };
    let state = ChartState {
        scales: vec![ScaleState {
            identity: identity.clone(),
            name: "y of plots 1–3".into(),
            domain: "0.0 to 50.0".into(),
        }],
        ..ChartState::default()
    };
    let zoom = ChartChange {
        domains: vec![(identity, numbers(&[-0.0, 260.0]))],
        ..ChartChange::default()
    };
    assert_eq!(
        describe_change(&zoom, &state).unwrap(),
        "y of plots 1–3: 0.0 to 260.0"
    );
}

#[test]
fn a_plot_is_named_by_its_id_or_its_place_with_its_facet_values() {
    let mut partition = BTreeMap::new();
    partition.insert(
        "Origin".to_string(),
        Arc::new(StringArray::from(vec!["Europe"])) as ArrayRef,
    );
    let cell = Identity {
        id: Id::Path("root.facet.plot".into()),
        partition,
    };
    assert_eq!(plot_name(1, &cell), "plot 2 (Origin = Europe)");
    let named = Identity {
        id: Id::Given("cars".into()),
        partition: BTreeMap::new(),
    };
    assert_eq!(plot_name(0, &named), "cars");
}
