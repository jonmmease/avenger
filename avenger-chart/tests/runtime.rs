use avenger_chart::{Chart, RenderOptions};
use avenger_chart_definition::{
    dataflow::{
        arrow::{
            array::{Float64Array, Int32Array, StringArray},
            datatypes::DataType,
            record_batch::RecordBatch,
        },
        datafusion::{
            common::ScalarValue,
            logical_expr::{col, lit},
        },
        *,
    },
    *,
};
use avenger_common::value::ScalarOrArrayValue;
use avenger_scenegraph::marks::{mark::SceneMark, symbol::SceneSymbolMark};
use avenger_wgpu::marks::instanced_mark::InstancedMarkFingerprint;
use std::sync::Arc;

fn snapshot(offset: f64) -> TableSnapshot {
    let batch = RecordBatch::try_from_iter(vec![
        (
            "x",
            Arc::new(Float64Array::from(
                (0..256)
                    .map(|i| i as f64 / 32. - 4. + offset)
                    .collect::<Vec<_>>(),
            )) as _,
        ),
        (
            "y",
            Arc::new(Float64Array::from(
                (0..256).map(|i| (i as f64).sin() * 3.).collect::<Vec<_>>(),
            )) as _,
        ),
    ])
    .unwrap();
    TableSnapshot::from_batches(batch.schema(), vec![batch]).unwrap()
}
fn definition(clamp: bool) -> anyhow::Result<(ChartDefinition, TableInput)> {
    let mut flow = DataflowBuilder::new();
    let rows = flow.table_input("points", snapshot(0.).schema().clone())?;
    let node = flow.add_plan("rows", rows.plan_ref())?;
    let points = flow.table_output("points", &node)?;
    let low = flow.scalar_input("low", DataType::Float64)?;
    let high = flow.scalar_input("high", DataType::Float64)?;
    let lo = flow.add_scalar("low_value", low.expr_ref())?;
    let hi = flow.add_scalar("high_value", high.expr_ref())?;
    let lo = flow.scalar_output("low", &lo)?;
    let hi = flow.scalar_output("high", &hi)?;
    let mut d = ChartDefinition::builder(flow.finish()?);
    d.parameter("low", &low, ScalarValue::Float64(Some(-4.)))?;
    d.parameter("high", &high, ScalarValue::Float64(Some(4.)))?;
    d.plot("points", |p| {
        p.content_size(320., 200.);
        p.guide_reservations(Edges::new(10., 10., 45., 50.));
        let x = p.scale(
            "x",
            Scale::linear(Domain::bounds(&lo, &hi), Range::PlotWidth).clamp(clamp),
        )?;
        let y = p.scale(
            "y",
            Scale::linear(Domain::numeric(-4., 4.), Range::PlotHeightReversed),
        )?;
        p.symbol(
            "points",
            &points,
            SymbolEncoding::new().x(x.field("x")).y(y.field("y")),
        )?;
        p.axis(Axis::bottom(&x))?;
        p.axis(Axis::left(&y))?;
        Ok(())
    })?;
    Ok((d.finish()?, rows))
}
fn symbol(marks: &[SceneMark]) -> &SceneSymbolMark {
    marks
        .iter()
        .find_map(|m| match m {
            SceneMark::Symbol(s) => Some(s),
            SceneMark::Group(g) => find_symbol(&g.marks),
            _ => None,
        })
        .unwrap()
}
fn find_symbol(marks: &[SceneMark]) -> Option<&SceneSymbolMark> {
    marks.iter().find_map(|m| match m {
        SceneMark::Symbol(s) => Some(s),
        SceneMark::Group(g) => find_symbol(&g.marks),
        _ => None,
    })
}

#[tokio::test]
async fn viewport_updates_retain_arrays_and_match_full_scaling() -> anyhow::Result<()> {
    let (d, rows) = definition(false)?;
    let fresh = Chart::prepare(d.clone(), chart_options()).await?;
    let chart = Chart::prepare(d, chart_options()).await?;
    let table = snapshot(0.);
    let inputs = chart.inputs()?.table(&rows, table)?.finish()?;
    let first = chart
        .render(RenderOptions::default().inputs(inputs.clone()))
        .await?;
    let request = RenderOptions::default()
        .inputs(inputs)
        .parameter("low", ScalarValue::Float64(Some(-2.)))
        .parameter("high", ScalarValue::Float64(Some(1.)));
    let second = chart.render(request.clone()).await?;
    let reference = fresh.render(request).await?;
    assert_eq!(first.geometry_report().position_builds, 1);
    assert_eq!(second.geometry_report().position_reuses, 1);
    let a = symbol(&first.scenegraph().marks);
    let b = symbol(&second.scenegraph().marks);
    let c = symbol(&reference.scenegraph().marks);
    let (ScalarOrArrayValue::Array(ax), ScalarOrArrayValue::Array(bx)) = (a.x.value(), b.x.value())
    else {
        panic!()
    };
    assert!(Arc::ptr_eq(ax, bx));
    assert!(a.x_adjustment.is_none());
    assert!(b.x_adjustment.is_some());
    assert_eq!(a.instanced_fingerprint(), b.instanced_fingerprint());
    for (x, y) in b.x_iter().zip(c.x_iter()) {
        assert!((x - y).abs() < 0.001, "{x} != {y}");
    }
    assert_eq!(b.len, 256);
    assert!(b.x_iter().any(|x| x < 0.));
    assert!(b.x_iter().any(|x| x > 320.));
    assert_eq!(first.plots()[0].rect, second.plots()[0].rect);
    assert_eq!(first.scenegraph().width, second.scenegraph().width);
    assert!(second.report().cache_hits > 0);
    assert_eq!(second.report().source_executions, 0);
    assert!(second.to_svg()?.contains("<svg"));
    assert!(second.to_pdf()?.starts_with(b"%PDF"));
    Ok(())
}
#[tokio::test]
async fn changed_snapshots_and_clamping_rebuild_positions() -> anyhow::Result<()> {
    for clamp in [false, true] {
        let (d, rows) = definition(clamp)?;
        let chart = Chart::prepare(d, chart_options()).await?;
        let input = chart.inputs()?.table(&rows, snapshot(0.))?.finish()?;
        let a = chart
            .render(RenderOptions::default().inputs(input.clone()))
            .await?;
        let b = chart
            .render(
                RenderOptions::default()
                    .inputs(input.clone())
                    .parameter("low", ScalarValue::Float64(Some(-2.))),
            )
            .await?;
        assert_eq!(b.geometry_report().position_reuses, usize::from(!clamp));
        let c = chart
            .render(
                RenderOptions::default().inputs(input.edit().table(&rows, snapshot(1.))?.finish()?),
            )
            .await?;
        assert_eq!(c.geometry_report().position_builds, 1);
        assert_eq!(c.geometry_report().position_reuses, 0);
        assert!(symbol(&a.scenegraph().marks).x_adjustment.is_none());
    }
    Ok(())
}
#[tokio::test]
async fn concurrent_frames_do_not_mutate_each_other() -> anyhow::Result<()> {
    let (d, rows) = definition(false)?;
    let chart = Chart::prepare(d, chart_options()).await?;
    let inputs = chart.inputs()?.table(&rows, snapshot(0.))?.finish()?;
    let first = chart
        .render(RenderOptions::default().inputs(inputs.clone()))
        .await?;
    let before = symbol(&first.scenegraph().marks).x_vec();
    let (a, b) = tokio::join!(
        chart.render(
            RenderOptions::default()
                .inputs(inputs.clone())
                .parameter("low", ScalarValue::Float64(Some(-1.)))
        ),
        chart.render(
            RenderOptions::default()
                .inputs(inputs)
                .parameter("high", ScalarValue::Float64(Some(2.)))
        )
    );
    let (a, b) = (a?, b?);
    assert_eq!(before, symbol(&first.scenegraph().marks).x_vec());
    assert_ne!(
        symbol(&a.scenegraph().marks).x_vec(),
        symbol(&b.scenegraph().marks).x_vec()
    );
    Ok(())
}
#[tokio::test]
async fn app_updates_validate_atomically_and_ignore_noops() -> anyhow::Result<()> {
    let (d, rows) = definition(false)?;
    let chart = Chart::prepare(d, chart_options()).await?;
    let inputs = chart.inputs()?.table(&rows, snapshot(0.))?.finish()?;
    let mut app = chart
        .into_app(RenderOptions::default().inputs(inputs))
        .await?;
    let state = app.app_state_mut();
    assert!(!state.set_parameter("low", ScalarValue::Float64(Some(-4.)))?);
    assert!(state
        .set_parameters([
            ("low".into(), ScalarValue::Float64(Some(-2.))),
            ("high".into(), ScalarValue::Utf8(Some("bad".into())))
        ])
        .is_err());
    assert_eq!(state.parameter("low")?, &ScalarValue::Float64(Some(-4.)));
    assert!(!state.is_pending());
    assert!(state.set_parameters([
        ("low".into(), ScalarValue::Float64(Some(-2.))),
        ("high".into(), ScalarValue::Float64(Some(2.)))
    ])?);
    assert!(state.is_pending());
    assert_eq!(
        state.rendered().plots()[0].scales["x"].numeric_interval_domain_f64()?,
        (-4., 4.)
    );
    Ok(())
}

#[tokio::test]
async fn nested_facets_sort_observed_keys_and_bind_ancestor_outputs() -> anyhow::Result<()> {
    let batch = RecordBatch::try_from_iter(vec![
        (
            "region",
            Arc::new(StringArray::from(vec!["West", "East", "West"])) as _,
        ),
        (
            "year",
            Arc::new(Int32Array::from(vec![2025, 2023, 2023])) as _,
        ),
        (
            "category",
            Arc::new(StringArray::from(vec!["A", "B", "A"])) as _,
        ),
        (
            "value",
            Arc::new(Float64Array::from(vec![10., 20., 30.])) as _,
        ),
    ])?;
    let mut flow = DataflowBuilder::new();
    let source = flow.table_snapshot(
        "source",
        TableSnapshot::from_batches(batch.schema(), vec![batch])?,
    )?;
    let categories = flow.table_output("categories", &source)?;
    let (regions, (years, rows, anchor)) =
        flow.partition_by("regions", source.plan_ref(), vec![col("region")], |r| {
            let (years, (rows, anchor)) =
                r.partition_by("years", r.rows().plan_ref(), vec![col("year")], |y| {
                    let anchor = y.add_scalar("anchor", lit(1))?;
                    Ok((
                        y.table_output("rows", &y.rows())?,
                        y.scalar_output("anchor", &anchor)?,
                    ))
                })?;
            Ok((years, rows, anchor))
        })?;
    let mut d = ChartDefinition::builder(flow.finish()?);
    d.facet("regions", &regions, Arrangement::column(), |r| {
        r.title(Text::key("region"));
        r.facet(
            "years",
            &years,
            Arrangement::row().uniform_columns().share("columns"),
            |y| {
                y.title(Text::key("year"));
                y.discover_with(&anchor);
                y.plot("bars", |p| {
                    let x = p.scale(
                        "x",
                        Scale::band(Domain::column(&categories, "category"), Range::PlotWidth),
                    )?;
                    let y = p.scale(
                        "y",
                        Scale::linear(Domain::numeric(0., 40.), Range::PlotHeightReversed)
                            .share_domain("v", PanelScope::Root),
                    )?;
                    p.rect(
                        "bars",
                        &rows,
                        RectEncoding::new()
                            .x(x.field("category"))
                            .width(x.bandwidth())
                            .y(y.field("value"))
                            .y2(y.constant(0.)),
                    )?;
                    p.axis(Axis::bottom(&x).labels(LabelVisibility::Outer))?;
                    p.axis(
                        Axis::left(&y)
                            .labels(LabelVisibility::Outer)
                            .shared_title(true)
                            .title("Value"),
                    )?;
                    Ok(())
                })
            },
        )
    })?;
    let definition = d.finish()?;
    let runtime = Runtime::new(RuntimeConfig::default())?;
    let definition = ChartDefinition::from_bytes(&definition.to_bytes()?, &runtime)?;
    let chart = Chart::prepare(definition, chart_options()).await?;
    let frame = chart.render(RenderOptions::default()).await?;
    assert_eq!(frame.plots().len(), 3);
    assert!(frame.plots()[0].rect.y < frame.plots()[1].rect.y);
    assert_eq!(frame.plots()[1].rect.y, frame.plots()[2].rect.y);
    let next = chart.render(RenderOptions::default()).await?;
    assert_eq!(
        frame.plots().iter().map(|p| &p.panel).collect::<Vec<_>>(),
        next.plots().iter().map(|p| &p.panel).collect::<Vec<_>>()
    );
    let svg = frame.to_svg()?;
    assert!(svg.contains("West"));
    assert!(svg.contains("East"));
    // Only the bottom row supplies category labels; represented axes keep their ticks.
    assert_eq!(svg.matches(">A</text>").count(), 2);
    assert_eq!(svg.matches(">B</text>").count(), 2);
    Ok(())
}

#[tokio::test]
async fn fixed_grids_and_wrapped_facets_compose_multiple_plots() -> anyhow::Result<()> {
    let batch = RecordBatch::try_from_iter(vec![(
        "key",
        Arc::new(Int32Array::from(vec![3, 1, 2])) as _,
    )])?;
    let mut flow = DataflowBuilder::new();
    let source = flow.table_snapshot(
        "source",
        TableSnapshot::from_batches(batch.schema(), vec![batch])?,
    )?;
    let all = flow.table_output("all", &source)?;
    let (scope, rows) = flow.partition_by("keys", source.plan_ref(), vec![col("key")], |s| {
        s.table_output("rows", &s.rows())
    })?;
    let add_plot = |p: &mut PlotBuilder, rows: &TableOutput| {
        p.content_size(100., 60.);
        p.rect(
            "rect",
            rows,
            RectEncoding::new().x(0.).y(0.).width(10.).height(10.),
        )
    };
    let mut d = ChartDefinition::builder(flow.finish()?);
    d.group(
        "fixed",
        Arrangement::grid(
            1,
            2,
            vec![
                (
                    "left".into(),
                    GridSlot {
                        row: 0,
                        column: 0,
                        row_span: 1,
                        column_span: 1,
                    },
                ),
                (
                    "right".into(),
                    GridSlot {
                        row: 0,
                        column: 1,
                        row_span: 1,
                        column_span: 1,
                    },
                ),
            ],
        ),
        |g| {
            g.plot("left", |p| add_plot(p, &all))?;
            g.plot("right", |p| add_plot(p, &all))
        },
    )?;
    d.facet("wrapped", &scope, Arrangement::wrap(2), |g| {
        g.arrange(Arrangement::row());
        g.plot("left", |p| add_plot(p, &rows))?;
        g.plot("right", |p| add_plot(p, &rows))
    })?;
    let definition = d.finish()?;
    let runtime = Runtime::new(RuntimeConfig::default())?;
    let definition = ChartDefinition::from_bytes(&definition.to_bytes()?, &runtime)?;
    let frame = Chart::prepare(definition, chart_options())
        .await?
        .render(RenderOptions::default())
        .await?;
    let p = frame.plots();
    assert_eq!(p.len(), 8);
    assert_eq!(p[0].rect.y, p[1].rect.y);
    assert!(p[0].rect.x < p[1].rect.x);
    assert_eq!(p[2].rect.y, p[5].rect.y);
    assert!(p[6].rect.y > p[2].rect.y);
    assert_eq!(p[6].rect.y, p[7].rect.y);
    assert_eq!(
        p.iter()
            .map(|p| &p.panel)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        8
    );
    Ok(())
}

#[tokio::test]
async fn scoped_defaults_overrides_and_empty_discovery() -> anyhow::Result<()> {
    let batch = RecordBatch::try_from_iter(vec![
        (
            "region",
            Arc::new(StringArray::from(vec!["East", "East", "West"])) as _,
        ),
        ("value", Arc::new(Float64Array::from(vec![1., 3., 2.])) as _),
    ])?;
    let schema = batch.schema();
    let mut f = DataflowBuilder::new();
    let source = f.table_input("source", schema.clone())?;
    let (regions, (threshold, rows)) =
        f.partition_by("regions", source.plan_ref(), vec![col("region")], |scope| {
            let threshold = scope.scalar_input("threshold", DataType::Float64)?;
            let visible = scope.add_plan(
                "visible",
                dataflow::datafusion::logical_expr::LogicalPlanBuilder::from(
                    scope.rows().plan_ref(),
                )
                .filter(col("value").gt(threshold.expr_ref()))?
                .build()?,
            )?;
            Ok((threshold, scope.table_output("rows", &visible)?))
        })?;
    let mut d = ChartDefinition::builder(f.finish()?);
    d.facet("regions", &regions, Arrangement::row(), |scope| {
        scope.parameter("threshold", &threshold, ScalarValue::Float64(Some(0.)))?;
        scope.plot("points", |p| {
            p.symbol(
                "points",
                &rows,
                SymbolEncoding::new().x(Value::field("value")).y(10.),
            )
        })
    })?;
    let chart = Chart::prepare(d.finish()?, chart_options()).await?;
    let east = regions.instance([ScalarValue::Utf8(Some("East".into()))])?;
    let inputs = chart
        .inputs()?
        .table(
            &source,
            TableSnapshot::from_batches(schema.clone(), vec![batch])?,
        )?
        .at(&east, |b| {
            b.scalar(&threshold, ScalarValue::Float64(Some(2.)))
        })?
        .finish()?;
    let frame = chart
        .render(RenderOptions::default().inputs(inputs.clone()))
        .await?;
    assert_eq!(frame.plots().len(), 2);
    assert_eq!(symbol(&frame.scenegraph().marks).len, 1);
    let empty = chart
        .render(
            RenderOptions::default().inputs(
                inputs
                    .edit()
                    .table(&source, TableSnapshot::from_batches(schema, vec![])?)?
                    .finish()?,
            ),
        )
        .await?;
    assert!(empty.plots().is_empty());
    assert!(empty.scenegraph().width.is_finite());
    assert!(empty.to_svg()?.contains("<svg"));
    Ok(())
}

type Job = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;
#[derive(Default)]
struct ManualHost {
    jobs: std::sync::Mutex<Vec<Job>>,
    wakes: std::sync::Mutex<Vec<avenger_eventstream::runtime::RuntimeWakeEvent>>,
}
impl avenger_app::background::host::Executor for ManualHost {
    fn spawn(&self, future: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>) {
        self.jobs.lock().unwrap().push(future);
    }
    fn wake(&self, event: avenger_eventstream::runtime::RuntimeWakeEvent) {
        self.wakes.lock().unwrap().push(event);
    }
}
#[tokio::test]
async fn superseded_completion_cannot_replace_the_displayed_frame() -> anyhow::Result<()> {
    let (d, rows) = definition(false)?;
    let chart = Chart::prepare(d, chart_options()).await?;
    let inputs = chart.inputs()?.table(&rows, snapshot(0.))?.finish()?;
    let mut app = chart
        .into_app(RenderOptions::default().inputs(inputs))
        .await?;
    let host = Arc::new(ManualHost::default());
    let attachment = avenger_app::background::host::Attachment::new(
        app.background_tasks().unwrap(),
        host.clone(),
    )?;
    attachment.activate();
    app.app_state_mut()
        .set_parameter("low", ScalarValue::Float64(Some(-1.)))?;
    let job = host.jobs.lock().unwrap().pop().unwrap();
    job.await;
    let stale = host.wakes.lock().unwrap().pop().unwrap();
    app.app_state_mut()
        .set_parameter("low", ScalarValue::Float64(Some(-2.)))?;
    app.update(
        &avenger_eventstream::window::WindowEvent::RuntimeWake(stale),
        avenger_common::time::Instant::now(),
    )
    .await?;
    assert_eq!(
        app.app_state_mut().rendered().plots()[0].scales["x"].numeric_interval_domain_f64()?,
        (-4., 4.)
    );
    let job = host.jobs.lock().unwrap().pop().unwrap();
    job.await;
    let wake = host.wakes.lock().unwrap().pop().unwrap();
    app.update(
        &avenger_eventstream::window::WindowEvent::RuntimeWake(wake),
        avenger_common::time::Instant::now(),
    )
    .await?;
    assert_eq!(
        app.app_state_mut().rendered().plots()[0].scales["x"].numeric_interval_domain_f64()?,
        (-2., 4.)
    );
    assert!(!app.app_state_mut().is_pending());
    Ok(())
}

fn d3_formatting() -> avenger_scales::formatter::ScaleFormatting {
    avenger_scales::formatter::ScaleFormatting::d3(Default::default(), Default::default())
}

fn chart_options() -> avenger_chart::ChartOptions {
    avenger_chart::ChartOptions::default().with_formatting(d3_formatting())
}
