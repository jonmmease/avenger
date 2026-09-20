use crate::{
    config::Config,
    dataflow::{Engine, Request, Tables, batch, number},
    selection::{Focus, Selections},
};
use anyhow::{Result, ensure};
use arrow::util::pretty::print_batches;

pub async fn run(config: Config) -> Result<()> {
    let mut engine = Engine::load(config).await?;
    let metadata = engine.metadata.clone();
    let selections = Selections::new(
        &metadata.carriers,
        metadata.domains,
        [600., 350.],
        engine.config.exact,
    )?;
    let mut request = Request {
        selections,
        scatter_size: [600., 350.],
        airline_size: [300., 350.],
        panel_size: [260., 120.],
        bins: Default::default(),
        focus: None,
    };
    let initial = engine.job(&request, false).await?.unwrap().run().await?;
    initial.print("Initial");
    let tables = initial.tables_for(&metadata)?;
    ensure!(
        tables.scatter.num_rows() == metadata.eligible_rows,
        "initial scatter count mismatch"
    );
    print_batches(tables.summary.batches())?;
    request.focus = Some(Focus::Scatter);
    if let Some(job) = engine.job(&request, true).await? {
        job.run().await?.print("Hover warm-up");
    }
    for (label, brush) in [
        ("Brush", [0., 100., -30., 30.]),
        ("Drag", [10., 110., -25., 35.]),
        ("Repeat", [10., 110., -25., 35.]),
    ] {
        request.selections.brush(Some(brush))?;
        let result = engine.job(&request, false).await?.unwrap().run().await?;
        result.print(label);
        let tables = result.tables_for(&metadata)?;
        print_batches(tables.summary.batches())?;
        ensure!(
            tables.scatter.num_rows() == metadata.eligible_rows,
            "brush filtered its own scatter"
        );
        let focus = request.focus.take();
        let direct = engine
            .job(&request, false)
            .await?
            .unwrap()
            .run()
            .await?
            .tables_for(&metadata)?;
        assert_same(&tables, &direct)?;
        request.focus = focus;
    }
    request.focus = Some(Focus::Airline);
    if let Some(job) = engine.job(&request, true).await? {
        job.run().await?.print("Airline warm-up");
    }
    request
        .selections
        .select_carriers([metadata.carriers[0].clone()].into_iter().collect())?;
    let result = engine.job(&request, false).await?.unwrap().run().await?;
    result.print("One airline");
    print_batches(result.tables_for(&metadata)?.airlines.batches())?;
    request.bins.insert(metadata.destinations[0].clone(), 15);
    let result = engine.job(&request, false).await?.unwrap().run().await?;
    result.print("One destination: 15-minute bins");
    print_batches(result.tables_for(&metadata)?.panels[&metadata.destinations[0]].batches())?;
    request.selections.select_carriers(Default::default())?;
    let result = engine.job(&request, false).await?.unwrap().run().await?;
    result.print("Select none");
    let tables = result.tables_for(&metadata)?;
    ensure!(
        tables.scatter.num_rows() == 0,
        "select none must empty scatter"
    );
    ensure!(
        number(&batch(&tables.summary)?, "count", 0)? == Some(0.),
        "select none count"
    );
    println!(
        "Headless trace passed: complete scatter, direct/rollup equality, scoped bins, and select-none."
    );
    Ok(())
}
pub fn assert_same(a: &Tables, b: &Tables) -> Result<()> {
    for (left, right) in std::iter::once((&a.airlines, &b.airlines))
        .chain(std::iter::once((&a.summary, &b.summary)))
        .chain(a.panels.iter().map(|(key, value)| (value, &b.panels[key])))
    {
        let a = batch(left)?;
        let b = batch(right)?;
        ensure!(
            a.num_rows() == b.num_rows(),
            "different aggregate row counts"
        );
        for name in ["count", "mean"] {
            if a.column_by_name(name).is_none() {
                continue;
            }
            for row in 0..a.num_rows() {
                let x = number(&a, name, row)?;
                let y = number(&b, name, row)?;
                ensure!(
                    match (x, y) {
                        (Some(x), Some(y)) => (x - y).abs() <= 1e-9 * (1. + y.abs()),
                        (None, None) => true,
                        _ => false,
                    },
                    "{name}: {x:?} != {y:?}"
                );
            }
        }
    }
    Ok(())
}
