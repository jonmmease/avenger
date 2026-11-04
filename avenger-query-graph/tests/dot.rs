//! Snapshots of the DOT graphs of the example's graph, of its variant, and of each of its runs.
//!
//! Each test writes its graphs to `tests/output/dot/` and compares them with `tests/baselines/`.
//! To accept a change, copy the output over the baseline with the command that a failure prints.

mod common;

use std::path::PathBuf;

use avenger_query_graph::{Inputs, SourceOptions};
use avenger_scales::scales::linear::LinearScale;
use avenger_selection::{PixelGrid, SelectionState};
use avenger_selection_datafusion::preaggregate::{Focus, FocusDimension};
use common::{context, gridded, inputs, plan, state, store};
use datafusion::scalar::ScalarValue;

const EXAMPLE: &str = "SELECT carrier, count(*) AS n FROM flights \
                       WHERE year = $year AND selected($brush) GROUP BY carrier";

fn grid() -> PixelGrid {
    PixelGrid::new(LinearScale::configured((-20.0, 100.0), (0.0, 240.0)), 4.0).unwrap()
}

/// `dot` with the parts that change between runs and machines masked: each node's time and
/// bytes.
fn masked(dot: &str) -> String {
    let mut masked = String::new();
    let mut rest = dot;
    while let Some(start) = (rest.find("time: ").into_iter())
        .chain(rest.find("bytes: "))
        .min()
    {
        let label = if rest[start..].starts_with("time: ") {
            "time: "
        } else {
            "bytes: "
        };
        masked.push_str(&rest[..start + label.len()]);
        masked.push('…');
        let after = &rest[start + label.len()..];
        let end = after.find("\\l").unwrap_or(after.len());
        let end = match label {
            "bytes: " => end.min(after.find(',').unwrap_or(end)),
            _ => end,
        };
        rest = &after[end..];
    }
    masked.push_str(rest);
    masked
}

/// Compare `dots` by name with their baselines, after writing them to the output directory.
fn compare(dots: &[(String, String)]) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests");
    let output = root.join("output/dot");
    let baselines = root.join("baselines/dot");
    std::fs::create_dir_all(&output).unwrap();
    let mut changed = Vec::new();
    for (name, dot) in dots {
        let file = format!("{name}.dot");
        std::fs::write(output.join(&file), dot).unwrap();
        match std::fs::read_to_string(baselines.join(&file)) {
            Ok(baseline) if baseline == *dot => {}
            _ => changed.push(file),
        }
    }
    assert!(
        changed.is_empty(),
        "the graphs {changed:?} differ from their baselines; to accept them, run\n  \
         mkdir -p {baselines} && cp {output}/*.dot {baselines}/",
        baselines = baselines.display(),
        output = output.display(),
    );
}

#[tokio::test]
async fn the_example_graphs_match_their_baselines() {
    let ctx = context();
    let store = store();
    store.set_source_options(
        "flights",
        SourceOptions {
            materialize: true,
            ttl: None,
        },
    );
    let graph = store
        .compile(&ctx.state(), vec![plan(&ctx, EXAMPLE).await])
        .unwrap();
    let focus = Focus {
        parameter: "$brush".into(),
        owner: "delay".into(),
        dimensions: vec![FocusDimension {
            column: "delay".into(),
            grid: Some(grid()),
        }],
    };
    let mut dots = vec![
        ("graph".to_string(), graph.to_dot(None).unwrap()),
        ("variant".to_string(), graph.to_dot(Some(&focus)).unwrap()),
    ];
    let brushed = |low, high| state(vec![("delay", gridded("delay", &grid(), low, high))]);
    let gesture = |year: i64, brush: SelectionState, focus: Option<Focus>| Inputs {
        focus,
        ..inputs(&[("year", ScalarValue::from(year))], &[("brush", brush)])
    };
    let gestures = [
        ("1-opens", gesture(2023, SelectionState::default(), None)),
        (
            "2-enters",
            gesture(2023, SelectionState::default(), Some(focus.clone())),
        ),
        (
            "3-moves",
            gesture(2023, brushed(40.0, 120.0), Some(focus.clone())),
        ),
        (
            "4-moves-again",
            gesture(2023, brushed(60.0, 180.0), Some(focus.clone())),
        ),
        (
            "5-slides",
            gesture(2024, brushed(60.0, 180.0), Some(focus.clone())),
        ),
        ("6-leaves", gesture(2024, brushed(60.0, 180.0), None)),
    ];
    for (name, inputs) in gestures {
        let run = graph.run(&inputs).await.unwrap();
        dots.push((format!("run-{name}"), masked(&run.to_dot())));
    }
    compare(&dots);
}

/// A document of a cross filter's two plots and a summary that combines two of its parameters,
/// over materialized flights: the plots share the scan, and the summary's branches are nodes.
#[tokio::test]
async fn a_document_graph_matches_its_baseline() {
    let ctx = context();
    let store = store();
    store.set_source_options(
        "flights",
        SourceOptions {
            materialize: true,
            ttl: None,
        },
    );
    let sqls = [
        "SELECT carrier, count(*) AS n FROM flights \
         WHERE year = $year AND selected($brush, exclude_own => true, owner => 'carriers') \
         GROUP BY carrier",
        "SELECT floor(delay / 20) AS bin, count(*) AS n FROM flights \
         WHERE year = $year AND selected($brush, exclude_own => true, owner => 'delays') \
         GROUP BY floor(delay / 20)",
        "SELECT 'late' AS kind, count(*) AS n FROM flights WHERE delay > $late \
         UNION ALL SELECT 'long', count(*) FROM flights WHERE distance > $long",
    ];
    let mut plans = Vec::new();
    for sql in sqls {
        plans.push(plan(&ctx, sql).await);
    }
    let graph = store.compile(&ctx.state(), plans).unwrap();
    let values = inputs(
        &[
            ("year", ScalarValue::from(2023i64)),
            ("late", ScalarValue::from(30.0)),
            ("long", ScalarValue::from(1500.0)),
        ],
        &[(
            "brush",
            state(vec![("delays", gridded("delay", &grid(), 40.0, 120.0))]),
        )],
    );
    graph.run(&values).await.unwrap();
    // The brush moves: only the carriers plot reads the delays plot's clause.
    let moved = Inputs {
        selections: inputs(
            &[],
            &[(
                "brush",
                state(vec![("delays", gridded("delay", &grid(), 60.0, 180.0))]),
            )],
        )
        .selections,
        ..values
    };
    let run = graph.run(&moved).await.unwrap();
    compare(&[
        ("document".to_string(), graph.to_dot(None).unwrap()),
        ("run-document-moves".to_string(), masked(&run.to_dot())),
    ]);
}
