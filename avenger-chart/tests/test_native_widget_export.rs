#![cfg(all(feature = "wgpu", feature = "pdf"))]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use avenger_chart::{
    plot::CompiledPlot,
    prelude::*,
    render::{PdfRenderer, WgpuRenderer},
};
use avenger_color::ColorOrGradient;
use avenger_scenegraph::marks::{mark::SceneMark, text::SceneTextMark};
use avenger_text::{
    empty_label_params,
    types::{FontStyle, FontWeight, TextAlign, TextBaseline, TextSyntaxMode},
};
use datafusion::prelude::SessionContext;

#[derive(Clone, Copy)]
struct ExportToy {
    fail: bool,
}

impl NativeWidget for ExportToy {
    fn id(&self) -> &str {
        if self.fail {
            "failing-export-toy"
        } else {
            "export-toy"
        }
    }

    fn kind(&self) -> &'static str {
        "export-toy"
    }

    fn schema_version(&self) -> u32 {
        1
    }

    fn payload(&self) -> serde_json::Value {
        serde_json::json!({ "fail": self.fail })
    }

    fn measure(&self) -> NativeWidgetMeasureSpec {
        NativeWidgetMeasureSpec::Registry
    }

    fn state(&self) -> NativeWidgetStateSpec {
        NativeWidgetStateSpec::try_new(Vec::new()).unwrap()
    }
}

#[derive(Clone)]
struct ExportToyFactory {
    detach_calls: Arc<AtomicUsize>,
    unmount_calls: Arc<AtomicUsize>,
}

impl NativeWidgetFactory for ExportToyFactory {
    fn kind(&self) -> &'static str {
        "export-toy"
    }

    fn supported_schema_versions(&self) -> std::ops::RangeInclusive<u32> {
        1..=1
    }

    fn measure(
        &self,
        _spec: &CompiledNativeWidgetSpec,
        _payload: &serde_json::Value,
        _ctx: &NativeWidgetFactoryContext<'_>,
    ) -> Result<NativeWidgetMeasurement, AvengerChartError> {
        Ok(NativeWidgetMeasurement::fixed(180.0, 32.0))
    }

    fn create(
        &self,
        _spec: &CompiledNativeWidgetSpec,
        payload: &serde_json::Value,
    ) -> Result<Box<dyn NativeWidgetInstance>, AvengerChartError> {
        Ok(Box::new(ExportToyInstance {
            fail: payload["fail"].as_bool().unwrap_or(false),
            detach_calls: self.detach_calls.clone(),
            unmount_calls: self.unmount_calls.clone(),
        }))
    }
}

struct ExportToyInstance {
    fail: bool,
    detach_calls: Arc<AtomicUsize>,
    unmount_calls: Arc<AtomicUsize>,
}

impl NativeWidgetInstance for ExportToyInstance {
    fn scene(
        &mut self,
        _environment: &NativeWidgetEnvironment,
        _ctx: &mut NativeWidgetCtx,
    ) -> Result<NativeWidgetScene, AvengerChartError> {
        if self.fail {
            return Err(AvengerChartError::InvalidArgument(
                "intentional export-toy scene failure".to_string(),
            ));
        }
        NativeWidgetScene::try_from_iter([(
            "label".to_string(),
            SceneMark::Text(Arc::new(SceneTextMark {
                text: "Native export stays vector".to_string().into(),
                text_syntax: TextSyntaxMode::Plain,
                text_params: empty_label_params().clone(),
                x: 10.0.into(),
                y: 21.0.into(),
                color: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]).into(),
                font: "Lato".to_string().into(),
                font_size: 14.0.into(),
                font_weight: FontWeight::default().into(),
                font_style: FontStyle::default().into(),
                align: TextAlign::Left.into(),
                baseline: TextBaseline::Alphabetic.into(),
                limit: 160.0.into(),
                ..Default::default()
            })),
        )])
    }

    fn on_session_detach(&mut self, _ctx: &mut NativeWidgetCtx) -> Result<(), AvengerChartError> {
        self.detach_calls.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn on_unmount(&mut self, _ctx: &mut NativeWidgetCtx) -> Result<(), AvengerChartError> {
        self.unmount_calls.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

fn runtime(
    detach_calls: Arc<AtomicUsize>,
    unmount_calls: Arc<AtomicUsize>,
) -> NativeWidgetRuntimeResources {
    let registry = NativeWidgetRegistry::new()
        .with_factory(ExportToyFactory {
            detach_calls,
            unmount_calls,
        })
        .unwrap();
    NativeWidgetRuntimeResources::new(
        Arc::new(registry),
        Arc::new(InMemoryNativeWidgetInstanceStore::new()),
        NativeWidgetDocumentId::new(),
    )
}

async fn compile_toy(ctx: &SessionContext, fail: bool) -> CompiledPlot {
    let compiled = Chart::<ZeroDCoord>::new()
        .canvas_size(320.0, 160.0)
        .plot_size(180.0, 64.0)
        .mark(Symbol::new().size(100.0).fill("#0072B2"))
        .native_widget(ExportToy { fail }.position(ChromePosition::Top))
        .compile(ctx)
        .await
        .unwrap();
    bincode::deserialize(&bincode::serialize(&compiled).unwrap()).unwrap()
}

#[tokio::test]
async fn native_widget_exports_require_registry_and_cleanup_success_and_failure() {
    let ctx = SessionContext::new();
    let compiled = compile_toy(&ctx, false).await;

    assert!(matches!(
        WgpuRenderer::new().render(&compiled, &ctx, None).await,
        Err(AvengerChartError::UnknownNativeWidgetKind { widget_id, kind })
            if widget_id == "export-toy" && kind == "export-toy"
    ));

    let detach_calls = Arc::new(AtomicUsize::new(0));
    let unmount_calls = Arc::new(AtomicUsize::new(0));
    let resources = runtime(detach_calls.clone(), unmount_calls.clone());
    let png = WgpuRenderer::new()
        .with_native_widget_runtime(
            resources.clone(),
            NativeWidgetPlotId::from_member_path("export/png"),
        )
        .render(&compiled, &ctx, None)
        .await
        .unwrap();
    assert_eq!(png.dimensions(), (320, 160));
    assert_eq!(detach_calls.load(Ordering::Relaxed), 1);
    assert_eq!(unmount_calls.load(Ordering::Relaxed), 1);

    let pdf = PdfRenderer::new()
        .with_options(avenger_pdf::PdfRenderOptions {
            compress: false,
            ..Default::default()
        })
        .with_native_widget_runtime(
            resources.clone(),
            NativeWidgetPlotId::from_member_path("export/pdf"),
        )
        .render(&compiled, &ctx, None)
        .await
        .unwrap();
    assert!(pdf.starts_with(b"%PDF-"));
    assert!(
        pdf.windows(2).any(|bytes| bytes == b"BT")
            && (pdf.windows(2).any(|bytes| bytes == b"Tj")
                || pdf.windows(2).any(|bytes| bytes == b"TJ"))
    );
    assert!(
        !pdf.windows(b"/Subtype /Image".len())
            .any(|bytes| bytes == b"/Subtype /Image")
    );
    assert_eq!(detach_calls.load(Ordering::Relaxed), 2);
    assert_eq!(unmount_calls.load(Ordering::Relaxed), 2);

    let failing = compile_toy(&ctx, true).await;
    assert!(matches!(
        WgpuRenderer::new()
            .with_native_widget_runtime(
                resources,
                NativeWidgetPlotId::from_member_path("export/failure"),
            )
            .render(&failing, &ctx, None)
            .await,
        Err(AvengerChartError::InvalidArgument(message))
            if message == "intentional export-toy scene failure"
    ));
    assert_eq!(detach_calls.load(Ordering::Relaxed), 3);
    assert_eq!(unmount_calls.load(Ordering::Relaxed), 3);
}
