//! The constructor functions of the `datafusion` feature: every constructor is callable, a constant
//! call folds to a fragment, and the SQL surface matches its snapshot. The tests of their arguments,
//! which name parameters out of order, are in avenger-sql, whose rewrite fills the omitted ones.

use std::path::Path;

use avenger_chart::datafusion::{constructor_names, constructor_surface, register};
use datafusion::prelude::SessionContext;

fn session() -> SessionContext {
    let ctx = SessionContext::new();
    register(&ctx);
    ctx
}

#[tokio::test]
async fn a_constant_call_folds_to_a_literal() {
    let ctx = session();
    let plan = ctx
        .sql("SELECT axis(title => 'Mass') AS a")
        .await
        .unwrap()
        .into_optimized_plan()
        .unwrap();
    let display = plan.display_indent().to_string();
    assert!(!display.contains("axis("), "the call folds: {display}");
}

#[tokio::test]
async fn every_constructor_is_callable_and_none_shadows_a_function() {
    let builtins = SessionContext::new().state();
    for name in constructor_names() {
        assert!(
            !builtins.scalar_functions().contains_key(name)
                && !builtins.aggregate_functions().contains_key(name)
                && !builtins.window_functions().contains_key(name),
            "`{name}` is the name of a DataFusion function"
        );
        // A call parses and reaches the constructor, so a keyword does not take its name.
        match session().sql(&format!("SELECT {name}()")).await {
            Ok(_) => {}
            Err(error) => {
                let message = error.to_string();
                assert!(message.contains(&format!("{name}():")), "{name}: {message}");
            }
        }
    }
}

#[test]
fn the_sql_surface_matches_its_snapshot() {
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let output = tests.join("output").join("constructors.txt");
    std::fs::create_dir_all(output.parent().unwrap()).unwrap();
    std::fs::write(&output, constructor_surface()).unwrap();
    let baseline = tests.join("baselines").join("constructors.txt");
    let expected = std::fs::read_to_string(&baseline).unwrap_or_default();
    assert!(
        expected == constructor_surface(),
        "the constructors changed; to accept {}, copy it to {}",
        output.display(),
        baseline.display()
    );
}

#[tokio::test]
async fn a_column_of_fragments_has_the_extension_of_fragments() {
    let batches = session()
        .sql("SELECT axis() AS a")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let schema = batches[0].schema();
    let metadata = schema.field(0).metadata();
    assert_eq!(
        metadata.get("ARROW:extension:name").map(String::as_str),
        Some(avenger_chart::datafusion::FRAGMENT_EXTENSION_NAME)
    );
    assert_eq!(
        metadata.get("ARROW:extension:metadata").map(String::as_str),
        Some("avenger.chart.v1.Axis")
    );
}
