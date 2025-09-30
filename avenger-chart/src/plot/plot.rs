//! Plot builder for creating visualizations

use super::compiled_plot::CompiledPlot;
use super::specs::{AxisSpec, ScaleSpec};
use super::title::{PlotSubtitle, PlotTitle};

use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::guide::{CompiledGuide, CoordinateGuideBuilder};
use crate::layout::{CanvasConstraint, LayoutSpec, Margins, PlotConstraint};
use crate::legend::Legend;
use crate::marks::{CompiledMark, Mark};
use crate::serialization::{LogicalPlanNodeExt, SerializableDataFrame};
use crate::theme::{Theme, css::CssTheme};
use datafusion::dataframe::DataFrame;
use datafusion_proto::protobuf::LogicalPlanNode;
use indexmap::IndexMap;
use serde_with::{FromInto, serde_as};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
pub struct Plot<C: CoordinateSystem> {
    coord_system: C,
    pub(crate) axis_specs: HashMap<String, AxisSpec>,
    pub(crate) legends: IndexMap<String, Legend>,
    pub(crate) mark_renderers: Vec<Arc<dyn CompiledMark>>,

    /// Plot-level data for mark inheritance
    pub(crate) data: Option<DataFrame>,

    /// Scale specifications (local or referenced)
    pub(crate) scale_specs: HashMap<String, ScaleSpec>,

    /// Mapping from scale names to their coordinate channel
    /// e.g., "y2" -> "y", "x2" -> "x"
    pub(crate) scale_to_coord_channel: HashMap<String, String>,

    /// Layout specification for sizing and margins
    pub(crate) layout_spec: LayoutSpec,

    /// Optional plot title rendered by the layout system
    pub(crate) title: Option<PlotTitle>,

    /// Optional plot subtitle rendered by the layout system
    pub(crate) subtitle: Option<PlotSubtitle>,

    /// Theme for visual styling
    pub(crate) theme: Option<Arc<dyn Theme>>,

    /// Guide configuration
    pub(crate) guide_config: Option<C::Guide>,

    /// Built guide renderer (for serialization)
    pub(crate) guide_renderer: Option<Arc<dyn CompiledGuide>>,

    /// Parameters that can be used in expressions
    pub(crate) params: Vec<crate::param::Param>,
}

impl<C: CoordinateSystem> Plot<C> {
    pub fn with_coord(coord_system: C) -> Self {
        Plot {
            coord_system,
            axis_specs: HashMap::new(),
            legends: IndexMap::new(),
            mark_renderers: Vec::new(),
            data: None,
            scale_specs: HashMap::new(),
            scale_to_coord_channel: HashMap::new(),
            layout_spec: LayoutSpec::default(),
            title: None,
            subtitle: None,
            theme: None,
            guide_config: None,
            guide_renderer: None,
            params: Vec::new(),
        }
    }
}

impl<C: CoordinateSystem + Default> Default for Plot<C> {
    fn default() -> Self {
        Self::with_coord(C::default())
    }
}

impl<C: CoordinateSystem + Default> Plot<C> {
    pub fn new() -> Self {
        Self::default()
    }
}

impl<C: CoordinateSystem> Plot<C> {
    /// Compile this plot into a renderable form (consuming self)
    pub async fn compile(
        mut self,
        session_context: &datafusion::prelude::SessionContext,
    ) -> Result<CompiledPlot, AvengerChartError> {
        // Always build guide renderer - either from config or default
        if self.guide_renderer.is_none() {
            let mut guide = if let Some(config) = &self.guide_config {
                config.clone()
            } else {
                // Create default guide for the coordinate system
                C::Guide::default()
            };

            // We need to populate axes from axis_specs before building
            // This mirrors what PlotRenderer does in render/guide.rs
            use crate::guide::CoordinateGuideBuilder;

            // Create axes map for the guide
            let mut guide_axes = HashMap::new();

            // Add user-specified axes from axis_specs
            for (channel, axis_spec) in &self.axis_specs {
                let crate::plot::AxisSpec::Local(axis_config) = axis_spec;
                // The axis_config is already the correct type for this coordinate system
                // We need to downcast it to the specific axis type for the guide
                // This is safe because the axis type matches the coordinate system
                if let Some(typed_axis) = axis_config
                    .as_any()
                    .downcast_ref::<<C::Guide as CoordinateGuideBuilder>::Axis>(
                ) {
                    guide_axes.insert(channel.clone(), typed_axis.clone());
                }
            }

            // Set the axes on the guide
            guide.set_axes(guide_axes);

            // Pass mark renderers to the guide so it can extract titles at render time
            guide.set_mark_renderers(self.mark_renderers.clone(), session_context);

            self.guide_renderer = Some(Arc::from(guide.build()));
        }

        Ok(CompiledPlot {
            coord_transform: self.coord_system.create_transform(),
            guide_renderer: self.guide_renderer,
            marks: self.mark_renderers,
            axis_specs: self.axis_specs,
            legends: self.legends,
            layout_spec: self.layout_spec,
            title: self.title,
            subtitle: self.subtitle,
            theme: self.theme,
            scale_to_coord_channel: self.scale_to_coord_channel,
            scale_specs: self.scale_specs,
            data: match self.data {
                Some(df) => {
                    let plan = df.logical_plan().clone();
                    Some(LogicalPlanNode::from_logical_plan(&plan).map_err(|e| {
                        AvengerChartError::InternalError(format!(
                            "Failed to serialize logical plan: {}. \
                            This typically happens when using a DataFrame created with a different SessionContext. \
                            Make sure to use the same SessionContext for creating data and compiling the plot.",
                            e
                        ))
                    })?)
                }
                None => None,
            },
            default_params: self
                .params
                .iter()
                .map(|p| (p.name.clone(), p.default.clone()))
                .collect(),
        })
    }

    /// Get a reference to the coordinate system
    pub fn coord_system(&self) -> &C {
        &self.coord_system
    }

    /// Get a reference to the scale specifications
    pub fn scale_specs(&self) -> &HashMap<String, ScaleSpec> {
        &self.scale_specs
    }

    /// Get a reference to the axis specifications
    pub fn axis_specs(&self) -> &HashMap<String, AxisSpec> {
        &self.axis_specs
    }

    /// Get a reference to the scale to coordinate channel mapping
    pub fn scale_to_coord_channel(&self) -> &HashMap<String, String> {
        &self.scale_to_coord_channel
    }

    /// Get a reference to the mark renderers
    pub fn mark_renderers(&self) -> &[Arc<dyn CompiledMark>] {
        &self.mark_renderers
    }

    /// Get a reference to the legends
    pub fn legends(&self) -> &IndexMap<String, Legend> {
        &self.legends
    }

    pub fn mark<M: Mark<C> + 'static>(mut self, mark: M) -> Self {
        // Extract scale and legend configurations from the mark's channels
        self.extract_channel_configs(&mark);

        // Build the CompiledMark from the Mark
        let renderer = mark.compile();

        // Add the renderer
        self.mark_renderers.push(renderer);

        self
    }

    /// Set plot-level data that can be inherited by marks
    pub fn data(mut self, data: DataFrame) -> Self {
        self.data = Some(data);
        self
    }

    /// Add a parameter that can be used in plot expressions
    pub fn add_param(mut self, param: crate::param::Param) -> Self {
        self.params.push(param);
        self
    }

    /// Add multiple parameters at once
    pub fn add_params(mut self, params: impl IntoIterator<Item = crate::param::Param>) -> Self {
        self.params.extend(params);
        self
    }

    /// Get the layout specification
    pub fn get_layout_spec(&self) -> &LayoutSpec {
        &self.layout_spec
    }

    // ====== Layout API ======

    /// Set fixed canvas dimensions (traditional mode)
    /// The plot area will fill the available space within the canvas
    pub fn canvas_size(mut self, width: f32, height: f32) -> Self {
        self.layout_spec =
            LayoutSpec::fixed_canvas(width, height, self.layout_spec.margins.clone());
        self
    }

    /// Set canvas sizing constraint for responsive layouts
    pub fn canvas_constraint(mut self, constraint: CanvasConstraint) -> Self {
        // If setting aspect ratio, clear plot aspect ratio to avoid conflicts
        if matches!(constraint, CanvasConstraint::PreferredAspectRatio(_))
            && matches!(
                self.layout_spec.plot_area,
                crate::layout::SizeMode::AspectRatio(_)
            )
        {
            self.layout_spec.plot_area = crate::layout::SizeMode::Auto;
        }
        self.layout_spec.canvas = constraint.into();
        self
    }

    /// Set fixed plot area dimensions (data-first mode)
    /// The canvas will expand to accommodate the plot area plus margins, axes, and legends
    pub fn plot_size(mut self, width: f32, height: f32) -> Self {
        self.layout_spec =
            LayoutSpec::fixed_plot_area(width, height, self.layout_spec.margins.clone());
        self
    }

    /// Set plot area sizing constraint for responsive layouts
    pub fn plot_constraint(mut self, constraint: PlotConstraint) -> Self {
        self.layout_spec.plot_area = match constraint {
            PlotConstraint::Auto => crate::layout::SizeMode::Auto,
            PlotConstraint::AspectRatio(r) => crate::layout::SizeMode::AspectRatio(r),
            PlotConstraint::Width(w) => crate::layout::SizeMode::Width(w),
            PlotConstraint::Height(h) => crate::layout::SizeMode::Height(h),
        };
        // If setting aspect ratio, clear canvas aspect ratio to avoid conflicts
        if matches!(constraint, PlotConstraint::AspectRatio(_))
            && matches!(
                self.layout_spec.canvas,
                crate::layout::SizeMode::AspectRatio(_)
            )
        {
            self.layout_spec.canvas = crate::layout::SizeMode::Auto;
        }
        self
    }

    /// Set margins
    pub fn margins(mut self, margins: Margins) -> Self {
        self.layout_spec.margins = margins;
        self
    }

    /// Set the theme for the plot
    pub fn theme(mut self, theme: impl Theme + 'static) -> Self {
        self.theme = Some(Arc::new(theme));
        self
    }

    /// Configure the guide (coordinate system visual elements like axes and background)
    pub fn configure_guide(mut self, guide: C::Guide) -> Self {
        self.guide_config = match self.guide_config {
            Some(existing) => {
                let mut updated = existing.clone();
                updated.update(guide);
                Some(updated)
            }
            None => Some(guide),
        };
        self
    }

    /// Access the configured theme (or default if not set)
    pub fn get_theme(&self) -> Arc<dyn Theme> {
        self.theme
            .clone()
            .unwrap_or_else(|| Arc::new(CssTheme::light()))
    }
}
