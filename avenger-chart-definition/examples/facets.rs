//! Region partitions contain year partitions; the runtime discovers both levels.
use avenger_chart::{Chart, RenderOptions};
use avenger_chart_definition::{
    dataflow::{
        arrow::{
            array::{Float64Array, Int32Array, StringArray},
            record_batch::RecordBatch,
        },
        datafusion::{
            functions_aggregate::expr_fn::sum,
            logical_expr::{col, scalar_subquery, LogicalPlanBuilder},
        },
        *,
    },
    *,
};
use std::sync::Arc;

fn main() -> anyhow::Result<()> {
    let ragged = std::env::args().any(|arg| arg == "--ragged");
    let mut regions = vec![];
    let mut years = vec![];
    let mut categories = vec![];
    let mut amounts = vec![];
    for (ri, region) in ["East", "North", "West"].iter().enumerate() {
        for year in 2023..=2025 {
            if ragged && *region == "North" && year == 2024 {
                continue;
            }
            for (ci, category) in ["A", "B", "C"].iter().enumerate() {
                for record in 0..3 {
                    regions.push(*region);
                    years.push(year);
                    categories.push(*category);
                    amounts.push(
                        10.0 + (ri * 7 + ci * 4 + record * 3) as f64 + (year - 2023) as f64 * 5.0,
                    );
                }
            }
        }
    }
    if std::env::args().any(|arg| arg == "--reverse") {
        regions.reverse();
        years.reverse();
        categories.reverse();
        amounts.reverse();
    }
    let batch = RecordBatch::try_from_iter(vec![
        ("region", Arc::new(StringArray::from(regions)) as _),
        ("year", Arc::new(Int32Array::from(years)) as _),
        ("category", Arc::new(StringArray::from(categories)) as _),
        ("amount", Arc::new(Float64Array::from(amounts)) as _),
    ])?;
    let mut flow = DataflowBuilder::with_semantics(SemanticConfig {
        function_versions: avenger_transform::function_versions(),
        ..Default::default()
    });
    let source = flow.table_snapshot(
        "sales",
        TableSnapshot::from_batches(batch.schema(), vec![batch])?,
    )?;
    let categories = flow.add_plan(
        "categories",
        LogicalPlanBuilder::from(source.plan_ref())
            .project(vec![col("category")])?
            .distinct()?
            .sort(vec![col("category").sort(true, false)])?
            .build()?,
    )?;
    let categories = flow.table_output("categories", &categories)?;
    let (regions, (years, bars, extent)) = flow.partition_by(
        "regions",
        source.plan_ref(),
        vec![col("region")],
        |region| {
            let (years, (bars, extent)) = region.partition_by(
                "years",
                region.rows().plan_ref(),
                vec![col("year")],
                |year| {
                    let totals = year.add_plan(
                        "totals",
                        avenger_transform::aggregate(
                            year.rows().plan_ref(),
                            vec![col("category")],
                            vec![sum(col("amount")).alias("total")],
                        )?,
                    )?;
                    let extent = year.add_scalar(
                        "extent",
                        scalar_subquery(Arc::new(avenger_transform::extent(
                            totals.plan_ref(),
                            col("total"),
                        )?)),
                    )?;
                    Ok((
                        year.table_output("bars", &totals)?,
                        year.scalar_output("extent", &extent)?,
                    ))
                },
            )?;
            Ok((years, bars, extent))
        },
    )?;
    let mut definition = ChartDefinition::builder(flow.finish()?);
    definition.title("Sales by region and year");
    definition.facet(
        "regions",
        &regions,
        Arrangement::column().gap(22.),
        |region| {
            region.title(Text::key("region"));
            region.facet(
                "years",
                &years,
                Arrangement::row()
                    .gap(14.)
                    .uniform_columns()
                    .share("year-columns"),
                |year| {
                    year.title(Text::key("year"));
                    year.plot("sales", |plot| {
                        plot.content_size(210., 130.);
                        let x = plot.scale(
                            "x",
                            Scale::band(Domain::column(&categories, "category"), Range::PlotWidth)
                                .padding_inner(0.15),
                        )?;
                        let y = plot.scale(
                            "y",
                            Scale::linear(Domain::extent(&extent), Range::PlotHeightReversed)
                                .zero(true)
                                .nice(true)
                                .share_domain("sales", PanelScope::Root),
                        )?;
                        plot.rect(
                            "bars",
                            &bars,
                            RectEncoding::new()
                                .x(x.field("category"))
                                .width(x.bandwidth())
                                .y(y.field("total"))
                                .y2(y.constant(0.)),
                        )?;
                        plot.axis(
                            Axis::bottom(&x)
                                .title("Category")
                                .labels(LabelVisibility::Outer)
                                .shared_title(true),
                        )?;
                        plot.axis(
                            Axis::left(&y)
                                .title("Sales")
                                .labels(LabelVisibility::Outer)
                                .shared_title(true),
                        )?;
                        Ok(())
                    })
                },
            )
        },
    )?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let chart = runtime.block_on(Chart::prepare(definition.finish()?, chart_options()))?;
    if std::env::args().any(|arg| arg == "--export") {
        let frame = runtime.block_on(chart.render(RenderOptions::default()))?;
        let name = if ragged {
            "chart-facets-ragged"
        } else {
            "chart-facets"
        };
        std::fs::write(format!("{name}.svg"), frame.to_svg()?)?;
        std::fs::write(format!("{name}.png"), runtime.block_on(frame.to_png(2.))?)?;
        println!(
            "Rendered {} observed panels; wrote {name}.svg and {name}.png.\n{:?}",
            frame.plots().len(),
            frame.report()
        );
        return Ok(());
    }
    let mut app = runtime.block_on(chart.into_app(RenderOptions::default()))?;
    println!("{:?}", app.app_state_mut().rendered().report());
    let options =
        avenger_winit_wgpu::WinitWgpuAvengerAppOptions::new(if cfg!(target_os = "macos") {
            2.
        } else {
            1.
        })
        .window_attributes(
            winit::window::WindowAttributes::default()
                .with_title("Chart definition · nested facets")
                .with_resizable(false),
        );
    let (mut host, event_loop) =
        avenger_winit_wgpu::WinitWgpuAvengerApp::try_new_and_event_loop_with_options(
            app, options, runtime,
        )?;
    event_loop.run_app(&mut host)?;
    if let Some(error) = host.take_fatal_error() {
        anyhow::bail!(error);
    }
    Ok(())
}

fn d3_text_engine() -> avenger_text::TextEngine {
    let mut registry = avenger_text::NumberFormatRegistry::default();
    registry.register(
        "d3",
        std::sync::Arc::new(avenger_format_number_d3::D3NumberFormatProvider),
    );
    avenger_text::default_text_engine().with_number_formatting(
        avenger_text::NumberFormatConfig::new("d3"),
        std::sync::Arc::new(registry),
    )
}

fn chart_options() -> avenger_chart::ChartOptions {
    avenger_chart::ChartOptions {
        text_engine: Some(d3_text_engine()),
        ..Default::default()
    }
}
