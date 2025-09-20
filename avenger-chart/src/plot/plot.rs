//! Core Plot struct and its basic implementations

use crate::channel::value::strip_trailing_numbers;
use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::guide::CoordinateGuide;
use crate::legend::Legend;
use crate::marks::Mark;
use crate::scales::Scale;
use crate::theme::{Theme, css::CssTheme};
use datafusion::dataframe::DataFrame;
use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::Arc;

use super::specs::{AxisSpec, ScaleSpec};
use super::title::{PlotSubtitle, PlotTitle, TitleAlign};

pub struct Plot<C: CoordinateSystem> {
    coord_system: C,
    pub(crate) axis_specs: HashMap<String, AxisSpec<<C::Guide as CoordinateGuide>::Axis>>,
    pub(crate) legends: IndexMap<String, Legend>,
    pub(crate) marks: Vec<Box<dyn Mark<C>>>,

    /// Plot-level data for mark inheritance
    pub(crate) data: Option<DataFrame>,

    /// Scale specifications (local or referenced)
    pub(crate) scale_specs: HashMap<String, ScaleSpec>,

    /// Mapping from scale names to their coordinate channel
    /// e.g., "y2" -> "y", "x2" -> "x"
    pub(crate) scale_to_coord_channel: HashMap<String, String>,

    /// Preferred size for the plot canvas
    preferred_size: Option<(f32, f32)>,

    /// Optional plot title rendered by the layout system
    pub(crate) title: Option<PlotTitle>,

    /// Optional plot subtitle rendered by the layout system
    pub(crate) subtitle: Option<PlotSubtitle>,

    /// Theme for visual styling
    pub(crate) theme: Option<Arc<dyn Theme>>,

    /// Guide configuration function
    pub(crate) configure_guide_fn: Option<Box<dyn Fn(C::Guide) -> C::Guide + Send + Sync>>,
}

impl<C: CoordinateSystem> Plot<C> {
    pub fn with_coord(coord_system: C) -> Self {
        Plot {
            coord_system,
            axis_specs: HashMap::new(),
            legends: IndexMap::new(),
            marks: Vec::new(),
            data: None,
            scale_specs: HashMap::new(),
            scale_to_coord_channel: HashMap::new(),
            preferred_size: None,
            title: None,
            subtitle: None,
            theme: None,
            configure_guide_fn: None,
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
    /// Get a reference to the coordinate system
    pub fn coord_system(&self) -> &C {
        &self.coord_system
    }

    /// Get a reference to the scale specifications
    pub fn scale_specs(&self) -> &HashMap<String, ScaleSpec> {
        &self.scale_specs
    }

    /// Get a reference to the axis specifications
    pub fn axis_specs(&self) -> &HashMap<String, AxisSpec<<C::Guide as CoordinateGuide>::Axis>> {
        &self.axis_specs
    }

    /// Get a reference to the scale to coordinate channel mapping
    pub fn scale_to_coord_channel(&self) -> &HashMap<String, String> {
        &self.scale_to_coord_channel
    }

    pub fn marks(&self) -> &[Box<dyn Mark<C>>] {
        &self.marks
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
            Some(ScaleSpec::Local(f)) => {
                let user_scale = f(base_scale);
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

        // Add the mark
        self.marks.push(Box::new(mark));
        self
    }

    /// Set plot-level data that can be inherited by marks
    pub fn data(mut self, data: DataFrame) -> Self {
        self.data = Some(data);
        self
    }

    /// Get preferred size for the plot
    pub fn get_preferred_size(&self) -> Option<(f32, f32)> {
        self.preferred_size
    }

    /// Set a preferred size for the plot canvas
    pub fn with_size(mut self, width: f32, height: f32) -> Self {
        self.preferred_size = Some((width, height));
        self
    }

    /// Set the theme for the plot
    pub fn theme(mut self, theme: impl Theme + 'static) -> Self {
        self.theme = Some(Arc::new(theme));
        self
    }

    /// Configure the guide (coordinate system visual elements like axes and background)
    pub fn configure_guide<F>(mut self, f: F) -> Self
    where
        F: Fn(C::Guide) -> C::Guide + Send + Sync + 'static,
    {
        self.configure_guide_fn = Some(Box::new(f));
        self
    }

    /// Access the configured theme (or default if not set)
    pub fn get_theme(&self) -> Arc<dyn Theme> {
        self.theme
            .clone()
            .unwrap_or_else(|| Arc::new(CssTheme::light()))
    }
}
