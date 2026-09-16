#[path = "common/scoped.rs"]
mod scoped;
use avenger_datafusion_dataflow::{
    datafusion::common::ScalarValue, Error, GraphBuilder, Inputs, PreparedGraph, Result, Runtime,
    RuntimeConfig, TableStore,
};

async fn setup() -> Result<(PreparedGraph, scoped::Fixture)> {
    let mut graph = GraphBuilder::new();
    let fixture = scoped::build(&mut graph)?;
    Ok((
        Runtime::new(RuntimeConfig::default())?
            .prepare(&graph.finish()?)
            .await?,
        fixture,
    ))
}
fn defaults(prepared: &PreparedGraph, f: &scoped::Fixture) -> Result<Inputs> {
    prepared
        .inputs()
        .table(&f.sales, scoped::sales())?
        .scalar(&f.multiplier, ScalarValue::from(2_i64))?
        .scope_defaults(&f.regions, |b| {
            b.scalar(&f.region.limit, ScalarValue::from(10_i64))
        })?
        .scope_defaults(&f.region.years, |b| {
            b.scalar(&f.region.year.fraction, ScalarValue::from(1_i64))?
                .table(&f.region.year.selected, scoped::products(&["A"]))
        })?
        .finish()
}

#[tokio::test]
async fn sparse_overrides_defaults_and_unsets_resolve_per_input() -> Result<()> {
    let (prepared, f) = setup().await?;
    let east_key = f.regions.key([ScalarValue::from("East")])?;
    let west_key = f.regions.key([ScalarValue::from("West")])?;
    let year_key = f.region.years.key([ScalarValue::from(2025_i32)])?;
    let east = f
        .regions
        .instance([ScalarValue::from("East")])?
        .child(&f.region.years, [ScalarValue::from(2025_i32)])?;
    let base = defaults(&prepared, &f)?;
    let overridden = base
        .edit()
        .at(&east, |b| {
            b.scalar(&f.region.year.fraction, ScalarValue::from(3_i64))?
                .table(&f.region.year.selected, scoped::products(&["B"]))
        })?
        .finish()?;
    let changed_default = overridden
        .edit()
        .scope_defaults(&f.region.years, |b| {
            b.scalar(&f.region.year.fraction, ScalarValue::from(2_i64))
        })?
        .finish()?;
    let result = prepared
        .query(
            &[f.region.year.rows],
            &[f.region.year.threshold],
            &changed_default,
        )
        .await?;
    let east_panel = result
        .scope(&f.regions)?
        .get(&east_key)
        .unwrap()
        .scope(&f.region.years)?
        .get(&year_key)
        .unwrap();
    let west_panel = result
        .scope(&f.regions)?
        .get(&west_key)
        .unwrap()
        .scope(&f.region.years)?
        .get(&year_key)
        .unwrap();
    assert_eq!(
        east_panel.scalar(&f.region.year.threshold)?,
        &ScalarValue::from(60_i64)
    );
    assert_eq!(
        scoped::amounts(east_panel.table(&f.region.year.rows)?),
        [80]
    );
    assert_eq!(
        west_panel.scalar(&f.region.year.threshold)?,
        &ScalarValue::from(40_i64)
    );
    assert_eq!(
        scoped::amounts(west_panel.table(&f.region.year.rows)?),
        [50]
    );
    let event_address = east_panel.instance().clone();
    let retained = east_panel.table(&f.region.year.rows)?.clone();
    drop(result);
    let inherited = changed_default
        .edit()
        .at(&event_address, |b| {
            b.unset_scalar(&f.region.year.fraction)?
                .unset_scalar(&f.region.year.fraction)
        })?
        .finish()?;
    let result = prepared
        .query(
            &[f.region.year.rows],
            &[f.region.year.threshold],
            &inherited,
        )
        .await?;
    let panel = result
        .scope(&f.regions)?
        .get(&east_key)
        .unwrap()
        .scope(&f.region.years)?
        .get(&year_key)
        .unwrap();
    assert_eq!(
        panel.scalar(&f.region.year.threshold)?,
        &ScalarValue::from(40_i64)
    );
    assert_eq!(scoped::amounts(panel.table(&f.region.year.rows)?), [80]);
    assert_eq!(scoped::amounts(&retained), [80]);
    let null = inherited
        .edit()
        .at(&east, |b| {
            b.scalar(&f.region.year.fraction, ScalarValue::Int64(None))
        })?
        .finish()?;
    let result = prepared
        .query(&[f.region.year.rows], &[f.region.year.threshold], &null)
        .await?;
    let panel = result
        .scope(&f.regions)?
        .get(&east_key)
        .unwrap()
        .scope(&f.region.years)?
        .get(&year_key)
        .unwrap();
    assert_eq!(
        panel.scalar(&f.region.year.threshold)?,
        &ScalarValue::Int64(None)
    );
    assert_eq!(panel.table(&f.region.year.rows)?.num_rows(), 0);
    let empty = inherited
        .edit()
        .at(&east, |b| {
            b.table(&f.region.year.selected, scoped::products(&[]))
        })?
        .finish()?;
    let result = prepared.query(&[f.region.year.rows], &[], &empty).await?;
    assert_eq!(
        result
            .scope(&f.regions)?
            .get(&east_key)
            .unwrap()
            .scope(&f.region.years)?
            .get(&year_key)
            .unwrap()
            .table(&f.region.year.rows)?
            .num_rows(),
        0
    );
    let original = prepared.query(&[f.region.year.rows], &[], &base).await?;
    assert_eq!(
        scoped::amounts(
            original
                .scope(&f.regions)?
                .get(&east_key)
                .unwrap()
                .scope(&f.region.years)?
                .get(&year_key)
                .unwrap()
                .table(&f.region.year.rows)?
        ),
        [40]
    );
    Ok(())
}

#[tokio::test]
async fn completeness_follows_demand_and_absent_overrides_do_not_create_instances() -> Result<()> {
    let (prepared, f) = setup().await?;
    assert!(matches!(
        prepared.inputs().finish(),
        Err(Error::MissingInput(_))
    ));
    let root_only = prepared
        .inputs()
        .table(&f.sales, scoped::sales())?
        .scalar(&f.multiplier, ScalarValue::from(2_i64))?
        .finish()?;
    let empty = prepared.query(&[], &[], &root_only).await?;
    assert_eq!(empty.report().physical_plans, 0);
    assert!(matches!(
        empty.scope(&f.regions),
        Err(Error::UnrequestedScope)
    ));
    prepared.query(&[f.region.rows], &[], &root_only).await?;
    let error = prepared
        .query(&[], &[f.region.year.threshold], &root_only)
        .await
        .unwrap_err();
    assert!(
        matches!(error, Error::Scoped { source, .. } if matches!(*source, Error::MissingInput(_)))
    );
    let empty_source = root_only
        .edit()
        .table(&f.sales, scoped::data(&[]))?
        .finish()?;
    assert!(prepared
        .query(&[f.region.year.rows], &[], &empty_source)
        .await?
        .scope(&f.regions)?
        .is_empty());
    let threshold_only = root_only
        .edit()
        .scope_defaults(&f.regions, |b| {
            b.scalar(&f.region.limit, ScalarValue::from(10_i64))
        })?
        .scope_defaults(&f.region.years, |b| {
            b.scalar(&f.region.year.fraction, ScalarValue::from(1_i64))
        })?
        .finish()?;
    prepared
        .query(&[], &[f.region.year.threshold], &threshold_only)
        .await?;
    assert!(prepared
        .query(&[f.region.year.rows], &[], &threshold_only)
        .await
        .is_err());
    let absent = f
        .regions
        .instance([ScalarValue::from("North")])?
        .child(&f.region.years, [ScalarValue::from(2030_i32)])?;
    let inputs = defaults(&prepared, &f)?
        .edit()
        .at(&absent, |b| {
            b.scalar(&f.region.year.fraction, ScalarValue::from(4_i64))
        })?
        .finish()?;
    let result = prepared
        .query(&[], &[f.region.year.threshold], &inputs)
        .await?;
    assert!(result
        .scope(&f.regions)?
        .get(&f.regions.key([ScalarValue::from("North")])?)
        .is_none());
    let next = inputs
        .edit()
        .table(&f.sales, scoped::data(&[(Some("North"), 2030, "A", 99)]))?
        .finish()?;
    let result = prepared
        .query(&[], &[f.region.year.threshold], &next)
        .await?;
    let panel = result
        .scope(&f.regions)?
        .get(&f.regions.key([ScalarValue::from("North")])?)
        .unwrap()
        .scope(&f.region.years)?
        .get(&f.region.years.key([ScalarValue::from(2030_i32)])?)
        .unwrap();
    assert_eq!(
        panel.scalar(&f.region.year.threshold)?,
        &ScalarValue::from(80_i64)
    );
    let missing_default = next
        .edit()
        .scope_defaults(&f.region.years, |b| b.unset_scalar(&f.region.year.fraction))?
        .finish()?;
    prepared
        .query(&[], &[f.region.year.threshold], &missing_default)
        .await?;
    let missing_override = missing_default
        .edit()
        .at(&absent, |b| b.unset_scalar(&f.region.year.fraction))?
        .finish()?;
    assert!(prepared
        .query(&[], &[f.region.year.threshold], &missing_override)
        .await
        .is_err());
    Ok(())
}

#[tokio::test]
async fn binding_ownership_types_and_table_snapshots_are_preserved() -> Result<()> {
    let (prepared, f) = setup().await?;
    assert!(matches!(
        prepared
            .inputs()
            .scalar(&f.region.limit, ScalarValue::from(1_i64)),
        Err(Error::OutOfScope(_))
    ));
    assert!(matches!(
        prepared.inputs().scope_defaults(&f.regions, |b| b
            .scalar(&f.multiplier, ScalarValue::from(1_i64))),
        Err(Error::OutOfScope(_))
    ));
    assert!(matches!(
        prepared.inputs().scope_defaults(&f.regions, |b| b
            .scalar(&f.region.year.fraction, ScalarValue::from(1_i64))),
        Err(Error::OutOfScope(_))
    ));
    assert!(matches!(
        prepared.inputs().scope_defaults(&f.regions, |b| b
            .scalar(&f.region.limit, ScalarValue::from("bad"))),
        Err(Error::ScalarTypeMismatch { .. })
    ));
    assert!(matches!(
        prepared.inputs().scope_defaults(&f.region.years, |b| b
            .table(&f.region.year.selected, scoped::sales())),
        Err(Error::SchemaMismatch(_))
    ));
    let (_, foreign) = setup().await?;
    assert!(matches!(
        prepared.inputs().scope_defaults(&foreign.regions, Ok),
        Err(Error::ForeignHandle)
    ));
    let address = f
        .regions
        .instance([ScalarValue::from("East")])?
        .child(&f.region.years, [ScalarValue::from(2025_i32)])?;
    let store = TableStore::new(scoped::products(&["B"]));
    let inputs = defaults(&prepared, &f)?
        .edit()
        .at(&address, |b| {
            b.table(&f.region.year.selected, store.snapshot())
        })?
        .finish()?;
    store.replace(scoped::products(&[]))?;
    let result = prepared.query(&[f.region.year.rows], &[], &inputs).await?;
    let panel = result
        .scope(&f.regions)?
        .get(&f.regions.key([ScalarValue::from("East")])?)
        .unwrap()
        .scope(&f.region.years)?
        .get(&f.region.years.key([ScalarValue::from(2025_i32)])?)
        .unwrap();
    assert_eq!(scoped::amounts(panel.table(&f.region.year.rows)?), [80]);
    let inherited = inputs
        .edit()
        .at(&address, |b| b.unset_table(&f.region.year.selected))?
        .finish()?;
    let result = prepared
        .query(&[f.region.year.rows], &[], &inherited)
        .await?;
    assert_eq!(
        scoped::amounts(
            result
                .scope(&f.regions)?
                .get(&f.regions.key([ScalarValue::from("East")])?)
                .unwrap()
                .scope(&f.region.years)?
                .get(&f.region.years.key([ScalarValue::from(2025_i32)])?)
                .unwrap()
                .table(&f.region.year.rows)?
        ),
        [40]
    );
    Ok(())
}
