//! `register` adds the transforms to a session that already has tables, functions, and
//! configuration, and places its optimizer rules once, after the first simplification.

use avenger_datafusion_transforms::register;
use datafusion::common::Result;
use datafusion::prelude::{SessionConfig, SessionContext};

#[tokio::test]
async fn existing_session() -> Result<()> {
    let ctx = SessionContext::new_with_config(SessionConfig::new().with_target_partitions(3));
    ctx.sql("CREATE TABLE t (x DOUBLE) AS VALUES (1.0), (2.0), (9.0)")
        .await?
        .collect()
        .await?;
    register(&ctx)?;
    register(&ctx)?;

    // The session keeps its table and its configuration.
    assert_eq!(ctx.state().config().target_partitions(), 3);
    let n = ctx
        .sql("SELECT bin(x) OVER () AS b FROM t")
        .await?
        .count()
        .await?;
    assert_eq!(n, 3);

    // The rules come once, right after the first simplification.
    let names: Vec<String> = ctx
        .state()
        .optimizers()
        .iter()
        .map(|r| r.name().to_string())
        .collect();
    let at = names
        .iter()
        .position(|n| n == "simplify_expressions")
        .unwrap();
    assert!(
        names[at + 1..]
            .iter()
            .take(5)
            .any(|n| n == "bin_window_rewrite"),
        "{names:?}"
    );
    assert_eq!(
        names.iter().filter(|n| *n == "bin_window_rewrite").count(),
        1
    );
    Ok(())
}
