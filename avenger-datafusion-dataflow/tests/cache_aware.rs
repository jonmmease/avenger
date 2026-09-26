mod common;

use avenger_datafusion_dataflow::{
    arrow::datatypes::DataType,
    datafusion::{
        common::ScalarValue,
        functions_aggregate::expr_fn::sum,
        logical_expr::{col, lit, Expr, LogicalPlanBuilder},
    },
    CacheAwareOptions, CacheNode, CacheRead, CacheTargets, DataflowBuilder, Error, ExprInput,
    Inputs, PlanNode, PreparedDataflow, QueryInputs, Reference, Result, Runtime, RuntimeConfig,
    ScalarInput, TableInput, TableOutput, TableSnapshot,
};

struct Fixture {
    flow: PreparedDataflow,
    source: TableInput,
    factor: ScalarInput,
    brush: ScalarInput,
    measure: ExprInput,
    target: PlanNode,
    total: TableOutput,
    output: TableOutput,
}

impl Fixture {
    async fn new(config: RuntimeConfig) -> Result<Self> {
        let mut b = DataflowBuilder::new();
        let source = b.table_input("source", common::schema())?;
        let factor = b.scalar_input("factor", DataType::Int64)?;
        let brush = b.scalar_input("brush", DataType::Int64)?;
        let measure = b.expr_input("measure", DataType::Int64)?;
        let target = b.add_plan(
            "total",
            LogicalPlanBuilder::from(source.plan_ref())
                .aggregate(
                    Vec::<Expr>::new(),
                    vec![sum(measure.expr_ref() * factor.expr_ref()).alias("value")],
                )?
                .build()?,
        )?;
        let total = b.table_output("total_rows", &target)?;
        let view = b.add_plan(
            "view",
            LogicalPlanBuilder::from(target.plan_ref())
                .filter(col("value").gt(brush.expr_ref()))?
                .build()?,
        )?;
        let output = b.table_output("rows", &view)?;
        let flow = Runtime::new(config)?.prepare(&b.finish()?).await?;
        Ok(Self {
            flow,
            source,
            factor,
            brush,
            measure,
            target,
            total,
            output,
        })
    }

    fn inputs(&self, source: TableSnapshot, factor: i64, brush: i64) -> Result<Inputs> {
        self.flow
            .inputs()
            .table(&self.source, source)?
            .scalar(&self.factor, factor.into())?
            .scalar(&self.brush, brush.into())?
            .expr(&self.measure, col("value"))?
            .finish()
    }

    fn options(&self, start_latest: bool) -> CacheAwareOptions {
        CacheAwareOptions {
            targets: CacheTargets::Nodes(vec![CacheNode::Plan(self.target.clone())]),
            start_latest,
        }
    }
}

#[tokio::test]
async fn sparse_fallback_preserves_current_brush_and_skips_aggregate() -> Result<()> {
    let f = Fixture::new(Default::default()).await?;
    let old = common::snapshot(&[1, 2, 3]);
    let latest = f.inputs(common::snapshot(&[10, 20]), 3, 11)?;
    f.flow
        .query(&[f.total], &[], &f.inputs(old.clone(), 2, 0)?)
        .await?;
    let fallback = f
        .flow
        .inputs()
        .table(&f.source, old.clone())?
        .scalar(&f.factor, 2_i64.into())?
        .finish_overrides()?;
    let query = f.flow.cache_aware_query(
        &[f.output],
        &[],
        QueryInputs::new(latest.clone()).fallbacks([fallback.clone()])?,
        f.options(false),
    )?;
    assert!(matches!(
        query.read(CacheRead::CachedOnly).await,
        Err(Error::CacheMiss {
            candidates_checked: 2,
            ..
        })
    ));
    let result = query.read(CacheRead::FromCachedTargets).await?;
    assert_eq!(result.candidate_index(), 1);
    assert_eq!(common::values(result.result().table(&f.output)?), [12]);
    assert_eq!(result.inputs().table_value(&f.source)?.id(), old.id());
    assert_eq!(result.inputs().scalar_value(&f.factor)?, &2_i64.into());
    assert_eq!(result.inputs().scalar_value(&f.brush)?, &11_i64.into());
    assert_eq!(result.result().report().executed_nodes, ["view"]);
    assert_eq!(
        query
            .read(CacheRead::CachedOnly)
            .await?
            .result()
            .report()
            .physical_plans,
        0
    );

    let next = latest.edit().scalar(&f.brush, 13_i64.into())?.finish()?;
    let query = f.flow.cache_aware_query(
        &[f.output],
        &[],
        QueryInputs::new(next).fallbacks([fallback])?,
        f.options(false),
    )?;
    let result = query.read(CacheRead::FromCachedTargets).await?;
    assert_eq!(result.result().table(&f.output)?.num_rows(), 0);
    assert_eq!(result.result().report().executed_nodes, ["view"]);
    Ok(())
}

#[tokio::test]
async fn each_fallback_inherits_preferred_bindings_and_preserves_indices() -> Result<()> {
    let f = Fixture::new(Default::default()).await?;
    let old = common::snapshot(&[1, 2]);
    let latest = f.inputs(common::snapshot(&[10]), 3, 0)?;
    f.flow
        .query(&[f.total], &[], &f.inputs(old.clone(), 3, 0)?)
        .await?;
    let wrong = f
        .flow
        .inputs()
        .table(&f.source, old.clone())?
        .scalar(&f.factor, 7_i64.into())?
        .finish_overrides()?;
    let old_only = f.flow.inputs().table(&f.source, old)?.finish_overrides()?;
    let inputs = QueryInputs::new(latest.clone())
        .fallbacks([wrong])?
        .fallbacks([old_only.clone(), old_only])?;
    let query = f
        .flow
        .cache_aware_query(&[f.total], &[], inputs, Default::default())?;
    let result = query.read(CacheRead::CachedOnly).await?;
    assert_eq!(result.candidate_index(), 2);
    assert_eq!(common::values(result.result().table(&f.total)?), [9]);
    f.flow.query(&[f.total], &[], &latest).await?;
    assert_eq!(
        query.read(CacheRead::CachedOnly).await?.candidate_index(),
        0
    );
    let duplicate = f.flow.inputs().finish_overrides()?;
    let query = f.flow.cache_aware_query(
        &[f.total],
        &[],
        QueryInputs::new(latest).fallbacks([duplicate])?,
        Default::default(),
    )?;
    assert_eq!(
        query.read(CacheRead::CachedOnly).await?.candidate_index(),
        0
    );
    Ok(())
}

#[tokio::test]
async fn all_targets_and_outputs_must_use_one_supplied_candidate() -> Result<()> {
    let mut b = DataflowBuilder::new();
    let left = b.table_input("left", common::schema())?;
    let right = b.table_input("right", common::schema())?;
    let scalar = b.scalar_input("scalar", DataType::Int64)?;
    let l = b.add_plan("left", left.plan_ref())?;
    let r = b.add_plan("right", right.plan_ref())?;
    let s = b.add_scalar("scalar", scalar.expr_ref())?;
    let lo = b.table_output("left", &l)?;
    let ro = b.table_output("right", &r)?;
    let so = b.scalar_output("scalar", &s)?;
    let flow = Runtime::new(Default::default())?
        .prepare(&b.finish()?)
        .await?;
    let a = flow
        .inputs()
        .table(&left, common::snapshot(&[1]))?
        .table(&right, common::snapshot(&[2]))?
        .scalar(&scalar, 1_i64.into())?
        .finish()?;
    let older = flow
        .inputs()
        .table(&left, common::snapshot(&[3]))?
        .table(&right, common::snapshot(&[4]))?
        .scalar(&scalar, 2_i64.into())?
        .finish()?;
    flow.query(&[lo], &[so], &a).await?;
    flow.query(&[ro], &[], &older).await?;
    let query = flow.cache_aware_query(
        &[lo, ro],
        &[so],
        QueryInputs::new(a).fallbacks([older.edit().finish_overrides()?])?,
        Default::default(),
    )?;
    let error = query.read(CacheRead::CachedOnly).await.unwrap_err();
    let Error::CacheMiss {
        missing,
        candidates_checked,
    } = error
    else {
        panic!("expected cache miss")
    };
    assert_eq!(candidates_checked, 2);
    assert_eq!(
        missing,
        [Reference {
            scope: vec![],
            name: "right".into()
        }]
    );
    flow.query(&[lo], &[so], &older).await?;
    let result = query.read(CacheRead::CachedOnly).await?;
    assert_eq!(result.candidate_index(), 1);
    assert_eq!(result.inputs().scalar_value(&scalar)?, &2_i64.into());
    assert_eq!(common::values(result.result().table(&lo)?), [3]);
    assert_eq!(common::values(result.result().table(&ro)?), [4]);
    assert_eq!(result.result().report().physical_plans, 0);
    Ok(())
}

#[tokio::test]
async fn expression_null_and_empty_table_overrides_are_explicit() -> Result<()> {
    let f = Fixture::new(Default::default()).await?;
    let inputs = f.inputs(common::snapshot(&[1, 2]), 2, 0)?;
    let empty = common::snapshot(&[]);
    for (fallback, exact) in [
        (
            f.flow
                .inputs()
                .expr(&f.measure, lit(1_i64))?
                .finish_overrides()?,
            inputs.edit().expr(&f.measure, lit(1_i64))?.finish()?,
        ),
        (
            f.flow
                .inputs()
                .scalar(&f.factor, ScalarValue::Int64(None))?
                .finish_overrides()?,
            inputs
                .edit()
                .scalar(&f.factor, ScalarValue::Int64(None))?
                .finish()?,
        ),
        (
            f.flow
                .inputs()
                .table(&f.source, empty.clone())?
                .finish_overrides()?,
            inputs.edit().table(&f.source, empty)?.finish()?,
        ),
    ] {
        f.flow.query(&[f.total], &[], &exact).await?;
        let query = f.flow.cache_aware_query(
            &[f.total],
            &[],
            QueryInputs::new(inputs.clone()).fallbacks([fallback])?,
            Default::default(),
        )?;
        let result = query.read(CacheRead::CachedOnly).await?;
        assert_eq!(result.candidate_index(), 1);
        assert_eq!(
            result.inputs().table_value(&f.source)?.id(),
            exact.table_value(&f.source)?.id()
        );
        assert_eq!(
            result.inputs().scalar_value(&f.factor)?,
            exact.scalar_value(&f.factor)?
        );
    }
    Ok(())
}

#[tokio::test]
async fn named_and_scalar_targets_validate_ownership_reachability_and_reuse() -> Result<()> {
    let f = Fixture::new(Default::default()).await?;
    let inputs = f.inputs(common::snapshot(&[1]), 1, 0)?;
    f.flow.query(&[f.total], &[], &inputs).await?;
    let named = |name: &str| {
        CacheNode::Named(Reference {
            scope: vec![],
            name: name.into(),
        })
    };
    let query = f.flow.cache_aware_query(
        &[f.output],
        &[],
        QueryInputs::new(inputs.clone()),
        CacheAwareOptions {
            targets: CacheTargets::Nodes(vec![named("total")]),
            start_latest: false,
        },
    )?;
    assert_eq!(
        query
            .read(CacheRead::FromCachedTargets)
            .await?
            .candidate_index(),
        0
    );
    for targets in [
        vec![],
        vec![named("total_rows")],
        vec![named("missing")],
        vec![named("view")],
    ] {
        assert!(f
            .flow
            .cache_aware_query(
                &[f.total],
                &[],
                QueryInputs::new(inputs.clone()),
                CacheAwareOptions {
                    targets: CacheTargets::Nodes(targets),
                    start_latest: false
                }
            )
            .is_err());
    }
    let foreign = Fixture::new(Default::default()).await?;
    assert!(matches!(
        QueryInputs::new(inputs.clone()).fallbacks([foreign.flow.inputs().finish_overrides()?]),
        Err(Error::ForeignHandle)
    ));
    assert!(matches!(
        f.flow.cache_aware_query(
            &[foreign.output],
            &[],
            QueryInputs::new(inputs.clone()),
            Default::default()
        ),
        Err(Error::ForeignHandle)
    ));
    assert!(matches!(
        f.flow.cache_aware_query(
            &[f.output],
            &[],
            QueryInputs::new(inputs),
            foreign.options(false)
        ),
        Err(Error::ForeignHandle)
    ));

    let mut b = DataflowBuilder::new();
    let scalar = b.add_scalar("constant", lit(7_i64))?;
    let out = b.scalar_output("out", &scalar)?;
    let p = Runtime::new(Default::default())?
        .prepare(&b.finish()?)
        .await?;
    let inputs = p.inputs().finish()?;
    p.query(&[], &[out], &inputs).await?;
    let query = p.cache_aware_query(
        &[],
        &[out],
        QueryInputs::new(inputs),
        CacheAwareOptions {
            targets: CacheTargets::Nodes(vec![CacheNode::Scalar(scalar)]),
            start_latest: false,
        },
    )?;
    assert_eq!(
        query
            .read(CacheRead::CachedOnly)
            .await?
            .result()
            .scalar(&out)?,
        &7_i64.into()
    );
    Ok(())
}

#[tokio::test]
async fn scalar_cache_keys_preserve_null_signed_zero_and_nan_bits() -> Result<()> {
    let mut b = DataflowBuilder::new();
    let x = b.scalar_input("x", DataType::Float64)?;
    let node = b.add_scalar("x", x.expr_ref())?;
    let out = b.scalar_output("x", &node)?;
    let p = Runtime::new(Default::default())?
        .prepare(&b.finish()?)
        .await?;
    for value in [
        Some(0.0),
        Some(-0.0),
        Some(f64::from_bits(0x7ff8000000000001)),
        Some(f64::from_bits(0x7ff8000000000002)),
        None,
    ] {
        let inputs = p
            .inputs()
            .scalar(&x, ScalarValue::Float64(value))?
            .finish()?;
        let q = p.cache_aware_query(
            &[],
            &[out],
            QueryInputs::new(inputs.clone()),
            Default::default(),
        )?;
        assert!(matches!(
            q.read(CacheRead::CachedOnly).await,
            Err(Error::CacheMiss { .. })
        ));
        p.query(&[], &[out], &inputs).await?;
        let result = q.read(CacheRead::CachedOnly).await?;
        let ScalarValue::Float64(actual) = result.result().scalar(&out)? else {
            panic!()
        };
        assert_eq!(actual.map(f64::to_bits), value.map(f64::to_bits));
    }
    Ok(())
}

#[test]
fn eager_warming_requires_tokio_but_cache_request_construction_does_not() -> Result<()> {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let f = rt.block_on(Fixture::new(Default::default()))?;
    let inputs = f.inputs(common::snapshot(&[1]), 1, 0)?;
    assert!(f
        .flow
        .cache_aware_query(
            &[f.output],
            &[],
            QueryInputs::new(inputs.clone()),
            f.options(false)
        )
        .is_ok());
    assert!(matches!(
        f.flow
            .cache_aware_query(&[f.output], &[], QueryInputs::new(inputs), f.options(true)),
        Err(Error::InvalidConfig(_))
    ));
    Ok(())
}

#[tokio::test]
async fn scoped_bindings_are_preserved_but_scoped_overrides_and_requests_are_rejected() -> Result<()>
{
    let mut b = DataflowBuilder::new();
    let source = b.table_input("source", common::schema())?;
    let root = b.add_plan("root", source.plan_ref())?;
    let root_out = b.table_output("rows", &root)?;
    let (scope, (local, local_out)) =
        b.partition_by("groups", root.plan_ref(), vec![col("value")], |builder| {
            let local = builder.scalar_input("local", DataType::Int64)?;
            let node = builder.add_scalar("local", local.expr_ref())?;
            Ok((local, builder.scalar_output("local", &node)?))
        })?;
    let flow = Runtime::new(Default::default())?
        .prepare(&b.finish()?)
        .await?;
    let values = common::snapshot(&[1]);
    let inputs = flow
        .inputs()
        .table(&source, values)?
        .scope_defaults(&scope, |b| b.scalar(&local, 9_i64.into()))?
        .finish()?;
    assert!(matches!(
        inputs.edit().finish_overrides(),
        Err(Error::OutOfScope(_))
    ));
    let instance = scope.instance([1_i64.into()])?;
    assert!(matches!(
        flow.inputs()
            .at(&instance, |b| b.scalar(&local, 7_i64.into()))?
            .finish_overrides(),
        Err(Error::OutOfScope(_))
    ));
    assert!(matches!(
        flow.cache_aware_query(
            &[],
            &[local_out],
            QueryInputs::new(inputs.clone()),
            Default::default()
        ),
        Err(Error::OutOfScope(_))
    ));
    flow.query(&[root_out], &[], &inputs).await?;
    let query = flow.cache_aware_query(
        &[root_out],
        &[],
        QueryInputs::new(inputs).fallbacks([flow.inputs().finish_overrides()?])?,
        Default::default(),
    )?;
    let selected = query.read(CacheRead::CachedOnly).await?;
    let result = flow.query(&[], &[local_out], selected.inputs()).await?;
    assert_eq!(
        result
            .scope(&scope)?
            .get(&scope.key([1_i64.into()])?)
            .unwrap()
            .scalar(&local_out)?,
        &9_i64.into()
    );
    Ok(())
}

#[tokio::test]
async fn fixed_assets_and_evaluation_local_nodes_are_not_cache_targets() -> Result<()> {
    let mut b = DataflowBuilder::new();
    let asset = b.table_snapshot("asset", common::snapshot(&[1]))?;
    let asset_out = b.table_output("asset", &asset)?;
    let now = b.add_scalar(
        "now",
        avenger_datafusion_dataflow::datafusion::functions::datetime::expr_fn::now(),
    )?;
    let now_out = b.scalar_output("now", &now)?;
    let flow = Runtime::new(Default::default())?
        .prepare(&b.finish()?)
        .await?;
    let inputs = flow.inputs().finish()?;
    assert!(matches!(
        flow.cache_aware_query(
            &[asset_out],
            &[],
            QueryInputs::new(inputs.clone()),
            Default::default()
        ),
        Err(Error::InvalidReference(_))
    ));
    assert!(matches!(
        flow.cache_aware_query(
            &[],
            &[now_out],
            QueryInputs::new(inputs),
            Default::default()
        ),
        Err(Error::InvalidReference(_))
    ));
    Ok(())
}

#[tokio::test]
async fn downstream_failure_does_not_retry_a_later_candidate() -> Result<()> {
    let mut b = DataflowBuilder::new();
    let input = b.scalar_input("divisor", DataType::Int64)?;
    let target = b.add_scalar("target", lit(6_i64))?;
    let target_out = b.scalar_output("target", &target)?;
    let view = b.add_scalar("view", target.expr_ref() / input.expr_ref())?;
    let output = b.scalar_output("view", &view)?;
    let flow = Runtime::new(Default::default())?
        .prepare(&b.finish()?)
        .await?;
    let good = flow.inputs().scalar(&input, 2_i64.into())?.finish()?;
    flow.query(&[], &[target_out, output], &good).await?;
    let bad = good.edit().scalar(&input, 0_i64.into())?.finish()?;
    let query = flow.cache_aware_query(
        &[],
        &[output],
        QueryInputs::new(bad).fallbacks([good.edit().finish_overrides()?])?,
        CacheAwareOptions {
            targets: CacheTargets::Nodes(vec![CacheNode::Scalar(target)]),
            start_latest: false,
        },
    )?;
    assert!(matches!(
        query.read(CacheRead::FromCachedTargets).await,
        Err(Error::Execution { .. }) | Err(Error::Shared(_))
    ));
    assert_eq!(
        query.read(CacheRead::CachedOnly).await?.candidate_index(),
        1
    );
    Ok(())
}

#[tokio::test]
async fn probing_a_partial_miss_does_not_change_cache_recency() -> Result<()> {
    use avenger_datafusion_dataflow::{CacheConfig, CachePolicy};
    let mut builder = DataflowBuilder::new();
    let left = builder.scalar_input("left", DataType::Int64)?;
    let right = builder.scalar_input("right", DataType::Int64)?;
    let left_node = builder.add_scalar("left", left.expr_ref())?;
    let right_node = builder.add_scalar("right", right.expr_ref())?;
    let left_out = builder.scalar_output("left", &left_node)?;
    let right_out = builder.scalar_output("right", &right_node)?;
    let flow = Runtime::new(RuntimeConfig {
        cache: CachePolicy::Lru(CacheConfig {
            max_entries: 3,
            max_bytes: 1024 * 1024,
        }),
        ..Default::default()
    })?
    .prepare(&builder.finish()?)
    .await?;
    let a = flow
        .inputs()
        .scalar(&left, 1_i64.into())?
        .scalar(&right, 1_i64.into())?
        .finish()?;
    let b = a.edit().scalar(&left, 2_i64.into())?.finish()?;
    flow.query(&[], &[left_out], &a).await?;
    flow.query(&[], &[left_out, right_out], &b).await?;
    let missing = a.edit().scalar(&right, 2_i64.into())?.finish()?;
    let probe = flow.cache_aware_query(
        &[],
        &[left_out, right_out],
        QueryInputs::new(missing),
        Default::default(),
    )?;
    assert!(matches!(
        probe.read(CacheRead::CachedOnly).await,
        Err(Error::CacheMiss { .. })
    ));
    let c = b.edit().scalar(&right, 3_i64.into())?.finish()?;
    flow.query(&[], &[right_out], &c).await?;
    let oldest =
        flow.cache_aware_query(&[], &[left_out], QueryInputs::new(a), Default::default())?;
    assert!(matches!(
        oldest.read(CacheRead::CachedOnly).await,
        Err(Error::CacheMiss { .. })
    ));
    let newer =
        flow.cache_aware_query(&[], &[left_out], QueryInputs::new(b), Default::default())?;
    assert!(newer.read(CacheRead::CachedOnly).await.is_ok());
    Ok(())
}
