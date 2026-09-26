//! Prepare portable chart definitions and evaluate immutable rendered frames.

mod app;
mod evaluate;
mod export;
mod marks;
mod scales;

pub use app::ChartAppState;
pub use avenger_chart_definition as definition;
pub use avenger_datafusion_dataflow as dataflow;
pub use avenger_text::TextEngine;
use dataflow::datafusion::{common::ScalarValue, execution::context::SessionContext};
use dataflow::{Inputs, InputsBuilder, PreparedDataflow, Runtime, RuntimeConfig};
use definition::ChartDefinition;
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
};

/// Chart preparation or rendering failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Definition(#[from] definition::Error),
    #[error(transparent)]
    Dataflow(#[from] dataflow::Error),
    #[error("chart: {0}")]
    Render(String),
}
/// Result of a chart operation.
pub type Result<T> = std::result::Result<T, Error>;
fn error(e: impl std::fmt::Display) -> Error {
    Error::Render(e.to_string())
}

/// Resources shared by measurement, rendering, and query execution.
#[derive(Default)]
pub struct ChartOptions {
    pub text_engine: Option<TextEngine>,
    pub scale_formatting: avenger_scales::formatter::ScaleFormatting,
    pub dataflow: Option<Runtime>,
}
impl ChartOptions {
    /// Share formatter settings between text markup and scale label preparation.
    pub fn with_formatting(
        mut self,
        formatting: avenger_scales::formatter::ScaleFormatting,
    ) -> Self {
        self.text_engine = Some(
            formatting.configure_text_engine(
                self.text_engine
                    .take()
                    .unwrap_or_else(avenger_text::default_text_engine),
            ),
        );
        self.scale_formatting = formatting;
        self
    }
}
/// One complete frame request. Native inputs support tables, expressions, and scoped bindings.
#[derive(Clone, Default)]
pub struct RenderOptions {
    pub inputs: Option<Inputs>,
    pub parameters: BTreeMap<String, ScalarValue>,
}
impl RenderOptions {
    /// Set a root chart parameter, replacing any earlier value in this request.
    pub fn parameter(mut self, name: impl Into<String>, value: ScalarValue) -> Self {
        self.parameters.insert(name.into(), value);
        self
    }
    /// Use a complete native dataflow binding snapshot.
    pub fn inputs(mut self, inputs: Inputs) -> Self {
        self.inputs = Some(inputs);
        self
    }
}
struct Inner {
    definition: ChartDefinition,
    prepared: PreparedDataflow,
    interface: dataflow::DataflowInterface,
    outputs: (Vec<dataflow::TableOutput>, Vec<dataflow::ScalarOutput>),
    text: TextEngine,
    scale_formatting: avenger_scales::formatter::ScaleFormatting,
    positions: Mutex<HashMap<String, Arc<marks::Positions>>>,
}
/// A prepared chart. Clones share query caches and retained immutable geometry.
#[derive(Clone)]
pub struct Chart(Arc<Inner>);
impl Chart {
    /// Validate and prepare a chart without evaluating its dataflow.
    pub async fn prepare(definition: ChartDefinition, options: ChartOptions) -> Result<Self> {
        let outputs = definition.outputs()?;
        let interface = definition.dataflow().interface();
        let runtime = match options.dataflow {
            Some(r) => r,
            None => Runtime::with_session_state_and_codec(
                SessionContext::new().state(),
                RuntimeConfig {
                    function_versions: avenger_transform::function_versions(),
                    ..Default::default()
                },
                Arc::new(avenger_transform::TransformExtensionCodec::default()),
            )?,
        };
        let prepared = runtime.prepare(definition.dataflow()).await?;
        Ok(Self(Arc::new(Inner {
            definition,
            prepared,
            interface,
            outputs,
            text: options
                .text_engine
                .unwrap_or_else(avenger_text::default_text_engine),
            scale_formatting: options.scale_formatting,
            positions: Mutex::new(HashMap::new()),
        })))
    }
    /// Read the native portable definition.
    pub fn definition(&self) -> &ChartDefinition {
        &self.0.definition
    }
    /// Start native bindings seeded with chart initial values, including scoped defaults.
    pub fn inputs(&self) -> Result<InputsBuilder> {
        let mut inputs = self.0.prepared.inputs();
        let interface = &self.0.interface;
        for p in self.definition().parameters() {
            if let Some(v) = &p.initial {
                let r = interface.scalar_input_reference(&p.input)?;
                inputs = if r.scope.is_empty() {
                    inputs.scalar(&p.input, v.clone())?
                } else {
                    inputs.scope_defaults(
                        interface.scope_at(&r.scope)?.handle().expect("child scope"),
                        |b| b.scalar(&p.input, v.clone()),
                    )?
                };
            }
        }
        Ok(inputs)
    }
    fn resolve_inputs(&self, options: &RenderOptions) -> Result<Inputs> {
        if let Some(inputs) = &options.inputs {
            self.0.interface.validate_inputs(inputs)?;
        }
        let mut builder = match &options.inputs {
            Some(i) => i.edit(),
            None => self.inputs()?,
        };
        let interface = &self.0.interface;
        for (name, value) in &options.parameters {
            let p = self
                .definition()
                .parameters()
                .iter()
                .find(|p| {
                    p.name == *name
                        && interface
                            .scalar_input_reference(&p.input)
                            .is_ok_and(|r| r.scope.is_empty())
                })
                .ok_or_else(|| error(format!("unknown root parameter {name}")))?;
            builder = builder.scalar(&p.input, value.clone())?;
        }
        Ok(builder.finish()?)
    }
    /// Evaluate one input snapshot and construct a complete immutable frame.
    pub async fn render(&self, options: RenderOptions) -> Result<RenderedChart> {
        let start = avenger_common::time::Instant::now();
        let inputs = self.resolve_inputs(&options)?;
        let (tables, scalars) = &self.0.outputs;
        let result = self.0.prepared.query(tables, scalars, &inputs).await?;
        let mut frame = evaluate::render(self, &result, inputs)?;
        frame.elapsed = start.elapsed();
        Ok(frame)
    }
}
/// Actual work performed while constructing one scene, in addition to the dataflow report.
#[derive(Clone, Debug, Default)]
pub struct GeometryReport {
    pub position_builds: usize,
    pub position_reuses: usize,
}
/// Geometry and scales used by one displayed plot instance.
#[derive(Clone, Debug)]
pub struct RenderedPlot {
    pub path: Vec<String>,
    pub instance: Option<dataflow::ScopeInstance>,
    pub panel: avenger_panels::PanelId,
    pub rect: avenger_layout::Rect,
    pub scales: BTreeMap<String, avenger_scales::scales::ConfiguredScale>,
}
/// An immutable scene and the effective input snapshot that produced it.
#[derive(Clone)]
pub struct RenderedChart {
    scene: Arc<avenger_scenegraph::scene_graph::SceneGraph>,
    text: TextEngine,
    inputs: Inputs,
    plots: Vec<RenderedPlot>,
    report: dataflow::EvaluationReport,
    geometry: GeometryReport,
    elapsed: std::time::Duration,
}
impl RenderedChart {
    /// Time spent evaluating and constructing this frame, excluding export and presentation.
    pub fn elapsed(&self) -> std::time::Duration {
        self.elapsed
    }
    /// Read the scene without evaluating the dataflow again.
    pub fn scenegraph(&self) -> &Arc<avenger_scenegraph::scene_graph::SceneGraph> {
        &self.scene
    }
    /// Read all expanded plot instances in display order.
    pub fn plots(&self) -> &[RenderedPlot] {
        &self.plots
    }
    /// Find a template path and optional typed facet instance.
    pub fn plot(
        &self,
        path: &[&str],
        instance: Option<&dataflow::ScopeInstance>,
    ) -> Option<&RenderedPlot> {
        self.plots.iter().find(|p| {
            p.path.iter().map(String::as_str).eq(path.iter().copied())
                && p.instance.as_ref() == instance
        })
    }
    /// Read the effective complete dataflow bindings.
    pub fn inputs(&self) -> &Inputs {
        &self.inputs
    }
    /// Read actual query, cache, and execution diagnostics.
    pub fn report(&self) -> &dataflow::EvaluationReport {
        &self.report
    }
    /// Read actual CPU position construction and reuse counts.
    pub fn geometry_report(&self) -> &GeometryReport {
        &self.geometry
    }
}
