//! Core Plot struct and its basic implementations

use super::specs::{AxisSpec, ScaleSpec};
use super::title::{PlotSubtitle, PlotTitle};

use crate::channel::value::strip_trailing_numbers;
use crate::coords::{CoordinateSystem, CoordinateSystemTransform};
use crate::error::AvengerChartError;
use crate::guide::{CoordinateGuideBuilder, CoordinateGuideRender};
use crate::layout::{CanvasConstraint, LayoutSpec, Margins, PlotConstraint};
use crate::legend::Legend;
use crate::marks::{Mark, MarkRenderer};
use crate::scales::Scale;
use crate::theme::{Theme, css::CssTheme};
use datafusion::dataframe::DataFrame;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Serialize, Deserialize)]
pub struct SerializablePlotRenderer {
    /// Coordinate system transform for position mapping
    pub(crate) coord_transform: Box<dyn CoordinateSystemTransform>,

    /// Guide renderer for axes/grids
    pub(crate) guide_renderer: Option<Box<dyn CoordinateGuideRender>>,

    /// Mark renderers
    pub(crate) marks: Vec<Arc<dyn MarkRenderer>>,

    /// Axis specifications
    pub(crate) axis_specs: HashMap<String, AxisSpec>,

    /// Legends
    pub(crate) legends: IndexMap<String, Legend>,

    /// Layout specification
    pub(crate) layout_spec: LayoutSpec,

    /// Plot title
    pub(crate) title: Option<PlotTitle>,

    /// Plot subtitle
    pub(crate) subtitle: Option<PlotSubtitle>,

    /// Theme
    pub(crate) theme: Option<Arc<dyn Theme>>,

    /// Mapping from scale names to coordinate channel
    pub(crate) scale_to_coord_channel: HashMap<String, String>,

    /// Scale specifications (temporarily kept for building scales)
    #[serde(skip)]
    pub(crate) scale_specs: HashMap<String, ScaleSpec>,

    /// Plot-level data (temporarily kept for mark inheritance)
    #[serde(skip)]
    pub(crate) data: Option<DataFrame>,
}

impl SerializablePlotRenderer {
    /// Get the theme or create default if not set
    pub fn get_theme(&self) -> Arc<dyn Theme> {
        self.theme.clone().unwrap_or_else(|| Arc::new(CssTheme::light()))
    }

    /// Get title if configured
    pub fn get_title(&self) -> Option<&PlotTitle> {
        self.title.as_ref()
    }

    /// Get subtitle if configured
    pub fn get_subtitle(&self) -> Option<&PlotSubtitle> {
        self.subtitle.as_ref()
    }

    /// Get layout spec
    pub fn get_layout_spec(&self) -> &LayoutSpec {
        &self.layout_spec
    }

    /// Render the plot to a scene graph
    pub async fn render(&self) -> Result<crate::render::RenderResult, AvengerChartError> {
        // This is a temporary implementation that shows we can't fully render
        // from just the SerializablePlotRenderer yet - we need the original Plot
        // to access methods like get_scale, collect_channels_needing_scales, etc.
        // This will be addressed in future refactoring.
        Err(AvengerChartError::InternalError(
            "Direct rendering from SerializablePlotRenderer not yet implemented".to_string()
        ))
    }
}

pub struct Plot<C: CoordinateSystem> {
    coord_system: C,
    pub(crate) axis_specs: HashMap<String, AxisSpec>,
    pub(crate) legends: IndexMap<String, Legend>,
    pub(crate) mark_renderers: Vec<Arc<dyn MarkRenderer>>,

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
    pub(crate) guide_renderer: Option<Box<dyn CoordinateGuideRender>>,
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
    /// Build a serializable plot renderer from this plot
    pub fn build(mut self) -> SerializablePlotRenderer {
        // Build guide if configured
        if self.guide_renderer.is_none() && self.guide_config.is_some() {
            self.guide_renderer = Some(self.guide_config.as_ref().unwrap().clone().build());
        }

        SerializablePlotRenderer {
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
            data: self.data,
        }
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
    pub fn mark_renderers(&self) -> &[Arc<dyn MarkRenderer>] {
        &self.mark_renderers
    }

    /// Get a reference to the legends
    pub fn legends(&self) -> &IndexMap<String, Legend> {
        &self.legends
    }

    /// Build a scale by name, applying any configured transformations
    /// Note: Default range will be applied during rendering when actual dimensions are known
    pub fn get_scale(&self, name: &str) -> Result<Scale, AvengerChartError> {
        // Strip trailing numbers to get the base scale name
        // e.g., "x2" -> "x", "y2" -> "y"
        let base_name = strip_trailing_numbers(name);

        // Build the default scale for the base name
        let mut base_scale = self.create_default_scale_for_channel_internal(base_name)?;

        // Gather domain expressions from marks
        if let Ok(domain_exprs) = self.gather_scale_domain_expressions(base_name) {
            if !domain_exprs.is_empty() {
                base_scale = base_scale.domain_data_fields(domain_exprs);
            }
        }

        match self.scale_specs.get(base_name) {
            Some(ScaleSpec::Local(scale_changes)) => {
                // Apply the plot-level scale configuration using update()
                let user_scale = base_scale.update(scale_changes.clone());
                Ok(user_scale)
            }
            None => Ok(base_scale),
        }
    }

    /// Get the default range for a coordinate channel based on plot area dimensions
    /// Returns None if the channel is not a coordinate channel
    pub fn get_coordinate_default_range(
        &self,
        name: &str,
        plot_area_width: f64,
        plot_area_height: f64,
    ) -> Option<(f64, f64)> {
        // Check if this scale is mapped to a coordinate channel
        let coord_channel = self
            .scale_to_coord_channel
            .get(name)
            .map(|s| s.as_str())
            .unwrap_or(name);

        self.coord_system
            .default_range(coord_channel, plot_area_width, plot_area_height)
    }

    pub fn mark<M: Mark<C> + 'static>(mut self, mark: M) -> Self {
        // Extract scale and legend configurations from the mark's channels
        self.extract_channel_configs(&mark);

        // Build the MarkRenderer from the Mark
        let renderer = mark.build();

        // Add the renderer
        self.mark_renderers.push(renderer);

        self
    }

    /// Set plot-level data that can be inherited by marks
    pub fn data(mut self, data: DataFrame) -> Self {
        self.data = Some(data);
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
