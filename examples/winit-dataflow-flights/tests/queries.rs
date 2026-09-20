use anyhow::Result;
use arrow::{
    array::{Array, Int32Array, StringArray},
    record_batch::RecordBatch,
};
use datafusion::prelude::SessionContext;
use std::sync::Arc;
use winit_dataflow_flights::{
    config::Config,
    dataflow::{Engine, Request, batch, number},
    headless::assert_same,
    selection::{Focus, Selections},
};

async fn tiny() -> Result<Engine> {
    let b = RecordBatch::try_from_iter(vec![
        (
            "flight_id",
            Arc::new(Int32Array::from((0..8).collect::<Vec<_>>())) as _,
        ),
        (
            "dep_delay",
            Arc::new(Int32Array::from(vec![
                Some(0),
                Some(10),
                Some(20),
                Some(30),
                Some(-5),
                Some(40),
                None,
                Some(50),
            ])) as _,
        ),
        (
            "arr_delay",
            Arc::new(Int32Array::from(vec![
                Some(0),
                Some(5),
                Some(30),
                Some(10),
                Some(-10),
                Some(35),
                Some(20),
                None,
            ])) as _,
        ),
        (
            "carrier",
            Arc::new(StringArray::from(vec![
                "AA", "AA", "BB", "BB", "AA", "CC", "AA", "AA",
            ])) as _,
        ),
        (
            "dest",
            Arc::new(StringArray::from(vec![
                "X", "X", "Y", "Y", "X", "Z", "X", "Z",
            ])) as _,
        ),
        (
            "scheduled_minute",
            Arc::new(Int32Array::from(vec![5, 65, 125, 185, 245, 305, 365, 425])) as _,
        ),
    ])?;
    let ctx = SessionContext::new();
    let plan = ctx.read_batch(b)?.into_unoptimized_plan();
    Engine::from_plan(
        Config {
            exact: true,
            ..Default::default()
        },
        ctx,
        plan,
    )
    .await
}
fn request(engine: &Engine) -> Result<Request> {
    Ok(Request {
        selections: Selections::new(
            &engine.metadata.carriers,
            engine.metadata.domains,
            [600., 350.],
            engine.config.exact,
        )?,
        scatter_size: [600., 350.],
        airline_size: [300., 350.],
        panel_size: [260., 120.],
        bins: Default::default(),
        focus: None,
    })
}
#[tokio::test]
async fn known_counts_crossfilters_rollups_and_scoped_reuse() -> Result<()> {
    let mut engine = tiny().await?;
    let m = engine.metadata.clone();
    assert_eq!((m.source_rows, m.eligible_rows), (8, 6));
    assert_eq!(m.destinations, ["X", "Y", "Z"]);
    let mut req = request(&engine)?;
    let first = engine.job(&req, false).await?.unwrap().run().await?;
    assert_eq!(first.summary()?.0, 6);
    assert!((first.summary()?.1.unwrap() - 70. / 6.).abs() < 1e-10);
    let first = first.tables_for(&m)?;
    assert_eq!(first.scatter.num_rows(), 6);
    req.focus = Some(Focus::Scatter);
    let warm = engine.job(&req, true).await?.unwrap().run().await?;
    assert!(
        warm.result
            .report()
            .executed_nodes
            .iter()
            .any(|n| n.ends_with("states"))
    );
    req.selections.brush(Some([0., 35., -1., 20.]))?;
    let brush = engine.job(&req, false).await?.unwrap().run().await?;
    assert_eq!(brush.summary()?, (3, Some(5.)));
    assert!(
        !brush
            .result
            .report()
            .executed_nodes
            .iter()
            .any(|n| n.ends_with("states"))
    );
    let actual = brush.tables_for(&m)?;
    assert_eq!(actual.scatter.id(), first.scatter.id());
    let counts = batch(&actual.airlines)?;
    assert_eq!(
        (0..3)
            .map(|i| number(&counts, "count", i).unwrap().unwrap() as usize)
            .collect::<Vec<_>>(),
        [2, 1, 0]
    );
    req.focus = None;
    let direct = engine
        .job(&req, false)
        .await?
        .unwrap()
        .run()
        .await?
        .tables_for(&m)?;
    assert_same(&actual, &direct)?;
    req.focus = Some(Focus::Scatter);
    req.selections.brush(Some([5., 35., -1., 20.]))?;
    let drag = engine.job(&req, false).await?.unwrap().run().await?;
    assert_eq!(drag.summary()?, (2, Some(7.5)));
    assert!(
        !drag
            .result
            .report()
            .executed_nodes
            .iter()
            .any(|n| n.ends_with("states"))
    );
    let before = drag.tables_for(&m)?;
    req.bins.insert("X".into(), 15);
    let after = engine
        .job(&req, false)
        .await?
        .unwrap()
        .run()
        .await?
        .tables_for(&m)?;
    assert_eq!(after.panels["X"].num_rows(), 96);
    assert_ne!(before.panels["X"].id(), after.panels["X"].id());
    assert_eq!(before.panels["Y"].id(), after.panels["Y"].id());
    req.focus = Some(Focus::Airline);
    engine.job(&req, true).await?.unwrap().run().await?;
    req.selections
        .select_carriers(["BB".into()].into_iter().collect())?;
    let selected = engine.job(&req, false).await?.unwrap().run().await?;
    assert_eq!(selected.summary()?, (1, Some(10.)));
    assert_eq!(selected.tables_for(&m)?.scatter.num_rows(), 2);
    req.selections.select_carriers(Default::default())?;
    let empty = engine.job(&req, false).await?.unwrap().run().await?;
    assert_eq!(empty.summary()?, (0, None));
    assert_eq!(empty.tables_for(&m)?.scatter.num_rows(), 0);
    Ok(())
}
#[tokio::test]
async fn forced_direct_has_no_warmup_or_states() -> Result<()> {
    let mut engine = tiny().await?;
    engine.config.preaggregate = false;
    let mut req = request(&engine)?;
    req.focus = Some(Focus::Scatter);
    assert!(engine.job(&req, true).await?.is_none());
    req.selections.brush(Some([0., 35., -1., 20.]))?;
    let result = engine.job(&req, false).await?.unwrap().run().await?;
    assert_eq!(result.summary()?.0, 3);
    assert!(
        result
            .result
            .report()
            .executed_nodes
            .iter()
            .all(|n| !n.contains("states") && !n.contains("rollup"))
    );
    Ok(())
}
#[tokio::test]
async fn full_fixture_draws_exactly_every_eligible_id() -> Result<()> {
    let config = Config::default();
    let reader =
        datafusion::parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(
            std::fs::File::open(&config.data)?,
        )?
        .build()?;
    let mut expected = vec![];
    for b in reader {
        let b = b?;
        let ids = b
            .column_by_name("flight_id")
            .unwrap()
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap();
        for row in 0..b.num_rows() {
            if !b.column_by_name("dep_delay").unwrap().is_null(row)
                && !b.column_by_name("arr_delay").unwrap().is_null(row)
            {
                expected.push(ids.value(row));
            }
        }
    }
    let mut engine = Engine::load(config).await?;
    let req = request(&engine)?;
    let table = engine
        .job(&req, false)
        .await?
        .unwrap()
        .run()
        .await?
        .tables_for(&engine.metadata)?
        .scatter;
    let b = batch(&table)?;
    let ids = b
        .column_by_name("flight_id")
        .unwrap()
        .as_any()
        .downcast_ref::<Int32Array>()
        .unwrap();
    assert_eq!(ids.len(), expected.len());
    if let Some((row, (actual, expected))) = ids
        .values()
        .iter()
        .zip(&expected)
        .enumerate()
        .find(|(_, (a, b))| a != b)
    {
        panic!("first differing flight ID at row {row}: {actual} != {expected}");
    }
    assert_eq!(expected.len(), 327346);
    Ok(())
}
#[test]
fn pixel_regrid_preserves_raw_values_and_reverses_y() -> Result<()> {
    let mut s = Selections::new(
        &["AA".into()],
        [[0., 100.], [0., 100.]],
        [100., 100.],
        false,
    )?;
    s.brush(Some([10., 20., 30., 40.]))?;
    let contribution = s
        .state
        .contributions(s.scatter.selection())?
        .find(|c| c.producer().id() == s.scatter.id())
        .unwrap();
    let raw = contribution.value().clone();
    let effective = contribution.effective_value().clone();
    s.regrid([[0., 100.], [0., 100.]], [200., 200.], false)?;
    let c = s
        .state
        .contributions(s.scatter.selection())?
        .find(|c| c.producer().id() == s.scatter.id())
        .unwrap();
    assert_eq!(c.value(), &raw);
    assert_ne!(c.effective_value(), &effective);
    assert!(
        s.scatter
            .pixel_grid(&avenger_selection::ProjectionId::new("y")?)
            .unwrap()
            .is_decreasing()
    );
    Ok(())
}
