//! A native TextInput filters a scatter plot as its committed value changes.
//!
//! The pale layer retains the full team for context. The blue foreground
//! contains names that include the case-insensitive search text. This example
//! also demonstrates the runtime registry/store injection required by native
//! widgets.
//!
//! Run with:
//! ```bash
//! cargo run --release -p avenger-chart-app --example widget_text_input_filter --features winit-wgpu
//! ```

use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_chart_app::{
    ChartAppBundle, ChartAppOptions, ChartResizeBinding, ChartRuntimeResources,
    WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions, chart_avenger_app_with_runtime_resources,
};
use avenger_chart_widgets::{TextInput, register_native_widgets};
use avenger_image::ImageResourceCache;
use avenger_resource::{RenderInvalidationHub, RenderInvalidationSink};
use datafusion::{
    functions::string::expr_fn::{contains, lower},
    prelude::{SessionContext, col},
};
use winit::{dpi::LogicalSize, window::WindowAttributes};

const SIZE: [f32; 2] = [760.0, 500.0];

fn main() {
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    let bundle = tokio_runtime.block_on(build_app());
    let options = bundle.configure_winit_options(
        WinitWgpuAvengerAppOptions::new(2.0).window_attributes(
            WindowAttributes::default()
                .with_title("avenger-chart native text input filter")
                .with_inner_size(LogicalSize::new(f64::from(SIZE[0]), f64::from(SIZE[1])))
                .with_resizable(false),
        ),
    );
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop_with_options(bundle.app, options, tokio_runtime);
    event_loop.run_app(&mut app).expect("run app");
}

async fn build_app() -> ChartAppBundle {
    let ctx = Arc::new(SessionContext::new());
    let data = ctx
        .sql(
            "SELECT * FROM (VALUES
                ('Ada',     1.0, 8.8), ('Amir',    2.2, 7.1),
                ('Bea',     3.1, 8.1), ('Carlos',  4.0, 6.4),
                ('Daria',   4.8, 9.2), ('Eli',     5.5, 7.8),
                ('Fatima',  6.2, 8.5), ('Grace',   7.1, 6.9),
                ('Hiro',    7.8, 8.9), ('Inez',    8.6, 7.4),
                ('Jonah',   9.1, 8.0), ('Kavita', 10.0, 9.4)
            ) AS t(name, tenure, score)",
        )
        .await
        .expect("build searchable scatter data");
    let search = TextInput::new("search")
        .placeholder("Filter by name…")
        .debounce(120);
    let matches = contains(lower(col("name")), lower(search.value()));

    let chart = Chart::<Cartesian>::new()
        .title("Search the team")
        .subtitle("Type part of a name; the blue layer updates after a short debounce")
        .canvas_size(SIZE[0], SIZE[1])
        .plot_size(520.0, 320.0)
        .data(data)
        .mark(
            Symbol::new()
                .x_with(col("tenure"), |x| {
                    x.axis(|axis| axis.title("Tenure (years)"))
                })
                .y_with(col("score"), |y| {
                    y.axis(|axis| axis.title("Performance score").grid(true))
                })
                .size(170.0)
                .fill("#D7DADD")
                .stroke("#FFFFFF")
                .stroke_width(1.0),
        )
        .mark(
            Symbol::new()
                .transform_no_output(Filter::new(matches), |mark| mark)
                .x(col("tenure"))
                .y(col("score"))
                .size(170.0)
                .fill("#0072B2")
                .stroke("#FFFFFF")
                .stroke_width(1.0),
        )
        .native_widget(search.position(ChromePosition::Top));

    let compiled = chart.compile(&ctx).await.expect("compile plot");
    let invalidations = RenderInvalidationHub::default();
    let image_resources = Arc::new(ImageResourceCache::new().with_render_invalidation_sink(
        Arc::new(invalidations.clone()) as Arc<dyn RenderInvalidationSink>,
    ));
    let mut registry = NativeWidgetRegistry::new();
    register_native_widgets(&mut registry).expect("register built-in native widgets");
    let native_runtime = NativeWidgetRuntimeResources::new(
        Arc::new(registry),
        Arc::new(InMemoryNativeWidgetInstanceStore::new()),
        NativeWidgetDocumentId::new(),
    );
    let runtime_resources = ChartRuntimeResources::new(image_resources, invalidations)
        .with_native_widget_runtime(native_runtime);
    let app = chart_avenger_app_with_runtime_resources(
        compiled,
        ctx,
        ChartAppOptions {
            resize_binding: ChartResizeBinding::none(),
            resize_throttle_ms: None,
            exact_on_resize_settle: true,
            log_metrics: true,
        },
        runtime_resources.clone(),
    )
    .await
    .expect("build chart app");
    ChartAppBundle {
        app,
        runtime_resources,
    }
}

fn init_diagnostics() {
    if std::env::var_os("RUST_LOG").is_some() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
            .try_init();
    } else {
        let _ = env_logger::try_init();
    }
}
