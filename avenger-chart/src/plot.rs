use crate::axis::{AxisPosition, CartesianAxis};
use crate::controllers::Controller;
use crate::coords::{Cartesian, CoordinateSystem, Polar};
use crate::legend::Legend;
use crate::marks::{Mark, RadiusExpression};
use crate::scales::Scale;
use datafusion::dataframe::DataFrame;
use datafusion::logical_expr::lit;
use std::collections::HashMap;
use std::sync::Arc;

/// How a scale is defined for a channel
#[derive(Clone)]
pub enum ScaleSpec {
    /// Scale defined locally on this plot with a configuration function
    Local(Arc<dyn Fn(Scale) -> Scale + Send + Sync>),
    /// Reference to a scale defined in parent layout
    Reference(String),
}

pub struct Plot<C: CoordinateSystem> {
    coord_system: C,
    pub(crate) axes: HashMap<String, C::Axis>,
    pub(crate) legends: HashMap<String, Legend>,
    pub(crate) marks: Vec<Box<dyn Mark<C>>>,

    /// Plot-level data for faceting and mark inheritance
    pub(crate) data: Option<DataFrame>,

    /// Faceting configuration
    facet_spec: Option<FacetSpec>,

    /// Controllers for interactivity (using type erasure)
    controllers: Vec<Box<dyn std::any::Any + Send + Sync>>,

    /// Scale specifications (local or referenced)
    pub(crate) scale_specs: HashMap<String, ScaleSpec>,

    /// Mapping from scale names to their coordinate channel
    /// e.g., "y_squared" -> "y", "y_temperature" -> "y"
    scale_to_coord_channel: HashMap<String, String>,
}

/// Enhanced resolution options with row/column specificity
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Resolution {
    /// Completely shared across all facets (same domain/range)
    #[default]
    Shared,

    /// Independent per facet (each facet has its own domain/range)
    Independent,

    /// Shared within rows, independent across rows
    /// (facet_grid: each row has consistent domain, different rows can differ)
    SharedRows,

    /// Shared within columns, independent across columns  
    /// (facet_grid: each column has consistent domain, different columns can differ)
    SharedCols,
}

/// Fine-grained resolution control for faceted plots
#[derive(Debug, Clone)]
pub struct FacetResolve {
    /// Scale resolution per channel (data mapping)
    scales: HashMap<String, Resolution>,

    /// Axis resolution per positional channel (visual layout)
    axes: HashMap<String, Resolution>,

    /// Legend resolution per non-positional channel (visual layout)
    legends: HashMap<String, Resolution>,
}

impl FacetResolve {
    pub fn new() -> Self {
        Self {
            scales: HashMap::new(),
            axes: HashMap::new(),
            legends: HashMap::new(),
        }
    }

    /// Configure scale resolution for a channel
    pub fn scale<C: Into<String>>(mut self, channel: C, resolution: Resolution) -> Self {
        self.scales.insert(channel.into(), resolution);
        self
    }

    /// Configure axis resolution for a positional channel (x, y, r, theta)
    pub fn axis<C: Into<String>>(mut self, channel: C, resolution: Resolution) -> Self {
        let channel = channel.into();
        if Self::is_positional_channel(&channel) {
            self.axes.insert(channel, resolution);
        }
        // Silently ignore non-positional channels (or we could warn/error)
        self
    }

    /// Configure legend resolution for a non-positional channel (color, size, shape, etc.)
    pub fn legend<C: Into<String>>(mut self, channel: C, resolution: Resolution) -> Self {
        let channel = channel.into();
        if !Self::is_positional_channel(&channel) {
            self.legends.insert(channel, resolution);
        }
        // Silently ignore positional channels (or we could warn/error)
        self
    }

    /// Get effective resolution for a channel type
    pub fn get_scale_resolution(&self, channel: &str) -> Resolution {
        self.scales
            .get(channel)
            .copied()
            .unwrap_or(Resolution::Shared)
    }

    pub fn get_axis_resolution(&self, channel: &str) -> Resolution {
        self.axes.get(channel).copied().unwrap_or_else(|| {
            // Default: axes follow scales unless explicitly overridden
            self.get_scale_resolution(channel)
        })
    }

    pub fn get_legend_resolution(&self, channel: &str) -> Resolution {
        self.legends.get(channel).copied().unwrap_or_else(|| {
            // Default: legends follow scales unless explicitly overridden
            self.get_scale_resolution(channel)
        })
    }

    fn is_positional_channel(channel: &str) -> bool {
        matches!(channel, "x" | "y" | "r" | "theta")
    }
}

impl Default for FacetResolve {
    fn default() -> Self {
        Self::new()
    }
}

/// Strip configuration for facet labels
#[derive(Debug, Clone)]
pub struct StripConfig {
    // TODO: Add strip styling options (background, text, positioning)
}

/// Enhanced faceting specification with full resolution control
#[derive(Debug, Clone)]
pub enum FacetSpec {
    /// Wrap facets in a grid, flowing to new rows
    Wrap {
        column: String,
        columns: Option<usize>,

        // Resolution system
        resolve: FacetResolve,

        // Layout configuration
        spacing: Option<f64>,
        strip: Option<StripConfig>,
    },
    /// Arrange facets in explicit grid
    Grid {
        row: Option<String>,
        column: Option<String>,

        // Resolution system
        resolve: FacetResolve,

        // Layout configuration
        spacing: Option<(f64, f64)>, // (row_spacing, col_spacing)
        strip: Option<StripConfig>,
    },
}

/// Builder for facet specifications
pub struct Facet;

impl Facet {
    pub fn wrap<S: Into<String>>(column: S) -> FacetWrapBuilder {
        FacetWrapBuilder {
            column: column.into(),
            columns: None,
            resolve: FacetResolve::new(),
            spacing: None,
            strip: None,
        }
    }

    pub fn grid() -> FacetGridBuilder {
        FacetGridBuilder {
            row: None,
            column: None,
            resolve: FacetResolve::new(),
            spacing: None,
            strip: None,
        }
    }
}

/// Enhanced facet wrap builder with resolution control
pub struct FacetWrapBuilder {
    column: String,
    columns: Option<usize>,
    resolve: FacetResolve,
    spacing: Option<f64>,
    strip: Option<StripConfig>,
}

impl FacetWrapBuilder {
    pub fn columns(mut self, columns: usize) -> Self {
        self.columns = Some(columns);
        self
    }

    /// Set complete resolution configuration
    pub fn resolve(mut self, resolve: FacetResolve) -> Self {
        self.resolve = resolve;
        self
    }

    /// Quick scale resolution for a channel
    pub fn resolve_scale<C: Into<String>>(mut self, channel: C, resolution: Resolution) -> Self {
        self.resolve = self.resolve.scale(channel, resolution);
        self
    }

    /// Quick axis resolution for a positional channel
    pub fn resolve_axis<C: Into<String>>(mut self, channel: C, resolution: Resolution) -> Self {
        self.resolve = self.resolve.axis(channel, resolution);
        self
    }

    /// Quick legend resolution for a non-positional channel
    pub fn resolve_legend<C: Into<String>>(mut self, channel: C, resolution: Resolution) -> Self {
        self.resolve = self.resolve.legend(channel, resolution);
        self
    }

    pub fn spacing(mut self, spacing: f64) -> Self {
        self.spacing = Some(spacing);
        self
    }

    pub fn build(self) -> FacetSpec {
        FacetSpec::Wrap {
            column: self.column,
            columns: self.columns,
            resolve: self.resolve,
            spacing: self.spacing,
            strip: self.strip,
        }
    }
}

impl From<FacetWrapBuilder> for FacetSpec {
    fn from(builder: FacetWrapBuilder) -> Self {
        builder.build()
    }
}

/// Enhanced facet grid builder with resolution control
pub struct FacetGridBuilder {
    row: Option<String>,
    column: Option<String>,
    resolve: FacetResolve,
    spacing: Option<(f64, f64)>,
    strip: Option<StripConfig>,
}

impl FacetGridBuilder {
    pub fn row<S: Into<String>>(mut self, variable: S) -> Self {
        self.row = Some(variable.into());
        self
    }

    pub fn column<S: Into<String>>(mut self, variable: S) -> Self {
        self.column = Some(variable.into());
        self
    }

    /// Set complete resolution configuration
    pub fn resolve(mut self, resolve: FacetResolve) -> Self {
        self.resolve = resolve;
        self
    }

    /// Quick scale resolution for a channel
    pub fn resolve_scale<C: Into<String>>(mut self, channel: C, resolution: Resolution) -> Self {
        self.resolve = self.resolve.scale(channel, resolution);
        self
    }

    /// Quick axis resolution for a positional channel
    pub fn resolve_axis<C: Into<String>>(mut self, channel: C, resolution: Resolution) -> Self {
        self.resolve = self.resolve.axis(channel, resolution);
        self
    }

    /// Quick legend resolution for a non-positional channel
    pub fn resolve_legend<C: Into<String>>(mut self, channel: C, resolution: Resolution) -> Self {
        self.resolve = self.resolve.legend(channel, resolution);
        self
    }

    pub fn spacing(mut self, row_spacing: f64, col_spacing: f64) -> Self {
        self.spacing = Some((row_spacing, col_spacing));
        self
    }

    pub fn build(self) -> FacetSpec {
        FacetSpec::Grid {
            row: self.row,
            column: self.column,
            resolve: self.resolve,
            spacing: self.spacing,
            strip: self.strip,
        }
    }
}

impl From<FacetGridBuilder> for FacetSpec {
    fn from(builder: FacetGridBuilder) -> Self {
        builder.build()
    }
}

impl<C: CoordinateSystem> Plot<C> {
    pub fn new(coord_system: C) -> Self {
        Plot {
            coord_system,
            axes: HashMap::new(),
            legends: HashMap::new(),
            marks: Vec::new(),
            data: None,
            facet_spec: None,
            controllers: Vec::new(),
            scale_specs: HashMap::new(),
            scale_to_coord_channel: HashMap::new(),
        }
    }

    /// Get a reference to the coordinate system
    pub fn coord_system(&self) -> &C {
        &self.coord_system
    }

    /// Internal helper to create a default scale for a channel
    fn create_default_scale_for_channel_internal(&self, channel: &str) -> Scale {
        use crate::scales::inference::{get_default_scale_options, infer_scale_type_with_mark};
        use datafusion::logical_expr::ExprSchemable;

        // Try to infer the data type and mark type for this channel
        let mut data_type = None;
        let mut mark_type = None;

        // Look through marks to find the expression for this channel
        for mark in &self.marks {
            if let Some(channel_value) = mark.data_context().encodings().get(channel) {
                // Get the dataframe for this mark
                let df = match mark.data_source() {
                    crate::marks::DataSource::Explicit => mark.data_context().dataframe(),
                    crate::marks::DataSource::Inherited => {
                        if let Some(plot_df) = &self.data {
                            plot_df
                        } else {
                            continue;
                        }
                    }
                };

                // Try to get the data type of the expression
                let schema = df.schema();
                if let Ok(expr_type) = channel_value.expr().get_type(schema) {
                    data_type = Some(expr_type);
                    mark_type = Some(mark.mark_type());
                    break;
                }
            }
        }

        // Create a scale based on the inferred type
        let scale_type = if let Some(dt) = &data_type {
            infer_scale_type_with_mark(channel, dt, mark_type)
        } else {
            // Fallback to channel-based defaults
            match channel {
                // Color channels default to ordinal
                "fill" | "stroke" | "color" | "shape" => "ordinal",
                // Everything else defaults to linear
                _ => "linear",
            }
        };

        let mut scale = Scale::with_type(scale_type);

        // Apply default options based on channel and scale type
        if let Some(dt) = &data_type {
            let default_options = get_default_scale_options(channel, scale_type, dt);
            for (key, value) in default_options {
                scale = scale.option(&key, value);
            }
        }

        // Apply channel-specific ranges
        match channel {
            "size" => scale = scale.range_interval(lit(16.0), lit(64.0)),
            "stroke_width" => scale = scale.range_interval(lit(0.0), lit(10.0)),
            "font_size" => scale = scale.range_interval(lit(0.0), lit(10.0)),
            "corner_radius" => scale = scale.range_interval(lit(0.0), lit(10.0)),
            "opacity" => scale = scale.range_interval(lit(0.0), lit(1.0)),
            "angle" => scale = scale.range_interval(lit(0.0), lit(360.0)),
            _ => {}
        }

        scale
    }

    pub fn mark<M: Mark<C> + 'static>(mut self, mark: M) -> Self {
        self.marks.push(Box::new(mark));
        self
    }

    /// Set plot-level data that can be inherited by marks and used for faceting
    pub fn data(mut self, data: DataFrame) -> Self {
        self.data = Some(data);
        self
    }

    /// Add faceting specification to the plot
    pub fn facet(mut self, facet_spec: impl Into<FacetSpec>) -> Self {
        self.facet_spec = Some(facet_spec.into());
        self
    }

    /// Convenient method to create wrap faceting
    pub fn facet_wrap(mut self, column: impl Into<String>) -> Self {
        self.facet_spec = Some(FacetSpec::Wrap {
            column: column.into(),
            columns: None,
            resolve: FacetResolve::new(),
            spacing: None,
            strip: None,
        });
        self
    }

    /// Convenient method to create grid faceting
    pub fn facet_grid(mut self, row: Option<String>, column: Option<String>) -> Self {
        self.facet_spec = Some(FacetSpec::Grid {
            row,
            column,
            resolve: FacetResolve::new(),
            spacing: None,
            strip: None,
        });
        self
    }

    // TODO: Data resolution methods
    /// Get the data to use for faceting operations
    /// Priority: explicit facet data > plot data > aggregated mark data
    pub fn get_faceting_data(&self) -> Option<DataFrame> {
        // TODO: Implement data resolution logic
        // 1. Check if facet has explicit data
        // 2. Use plot-level data if available
        // 3. Aggregate from marks as fallback
        self.data.clone()
    }

    /// Check if the plot has faceting configured
    pub fn is_faceted(&self) -> bool {
        self.facet_spec.is_some()
    }

    /// Build a scale by name, applying any configured transformations
    /// Note: Default range will be applied during rendering when actual dimensions are known
    pub fn get_scale(&self, name: &str) -> Scale {
        match self.scale_specs.get(name) {
            Some(ScaleSpec::Local(f)) => {
                let base_scale = self.create_default_scale_for_channel_internal(name);
                f(base_scale)
            }
            Some(ScaleSpec::Reference(_)) => {
                todo!("Referenced scales are not yet implemented");
            }
            None => self.create_default_scale_for_channel_internal(name),
        }
    }

    /// Gather mark data and encoding expressions that use this scale
    pub fn gather_scale_domain_expressions(
        &self,
        scale_name: &str,
    ) -> Vec<(Arc<DataFrame>, datafusion::logical_expr::Expr)> {
        use crate::marks::DataSource;

        let mut data_expressions = Vec::new();

        for mark in &self.marks {
            // Get the appropriate DataFrame based on data source
            let df = match mark.data_source() {
                DataSource::Explicit => Arc::new(mark.data_context().dataframe().clone()),
                DataSource::Inherited => {
                    // Use plot-level data if available
                    if let Some(plot_data) = &self.data {
                        Arc::new(plot_data.clone())
                    } else {
                        // Skip this mark if no plot data is available
                        continue;
                    }
                }
            };

            // Check all encodings in the mark's data context
            for (channel, channel_value) in mark.data_context().encodings() {
                // Check if this channel uses our scale
                // Get the scale name this channel would use
                if let Some(channel_scale_name) = channel_value.scale_name(channel) {
                    if channel_scale_name == scale_name {
                        // Add the expression directly
                        data_expressions.push((df.clone(), channel_value.expr().clone()));
                    }
                }
            }
        }

        data_expressions
    }

    /// Create a channel resolver function for a mark that handles both explicit mappings and defaults
    fn create_channel_resolver<'a>(
        mark: &'a dyn Mark<C>,
        encodings: &'a indexmap::IndexMap<String, crate::marks::ChannelValue>,
        scales: &'a HashMap<String, Scale>,
    ) -> impl Fn(&str) -> datafusion::logical_expr::Expr + 'a {
        use crate::marks::ChannelValue;
        use crate::marks::channel::strip_trailing_numbers;
        use datafusion::prelude::lit;

        move |channel_name: &str| -> datafusion::logical_expr::Expr {
            // First check explicit mapping
            if let Some(channel_value) = encodings.get(channel_name) {
                // Apply scaling if needed
                match channel_value {
                    ChannelValue::Identity { .. } => {
                        // No scaling requested
                        channel_value.expr().clone()
                    }
                    ChannelValue::Scaled {
                        scale_name: custom_scale_name,
                        band,
                        ..
                    } => {
                        // Determine scale name
                        let scale_key = custom_scale_name
                            .as_ref()
                            .cloned()
                            .unwrap_or_else(|| strip_trailing_numbers(channel_name).to_string());

                        // Apply scale if it exists
                        if let Some(scale) = scales.get(&scale_key) {
                            let scale = if let Some(band_value) = band {
                                let scale_type = scale.get_scale_impl().scale_type();
                                if scale_type == "band" || scale_type == "point" {
                                    scale.clone().option("band", lit(*band_value))
                                } else {
                                    scale.clone()
                                }
                            } else {
                                scale.clone()
                            };

                            scale
                                .to_expr(channel_value.expr().clone())
                                .unwrap_or_else(|_| channel_value.expr().clone())
                        } else {
                            channel_value.expr().clone()
                        }
                    }
                }
            } else if let Some(default_scalar) = mark.default_channel_value(channel_name) {
                // Use mark-provided default
                lit(default_scalar)
            } else {
                // No mapping and no default
                lit(datafusion::scalar::ScalarValue::Null)
            }
        }
    }

    /// Gather mark data and encoding expressions with radius information for positional scales
    pub fn gather_scale_domain_expressions_with_radius(
        &self,
        scale_name: &str,
        scales: &HashMap<String, Scale>,
    ) -> Vec<(
        Arc<DataFrame>,
        datafusion::logical_expr::Expr,
        Option<RadiusExpression>,
    )> {
        use crate::marks::DataSource;

        let mut data_expressions = Vec::new();

        // Only gather radius for positional scales (including x2, y2 which map to x, y scales)
        let is_positional = matches!(scale_name, "x" | "y");
        if !is_positional {
            // For non-positional scales, return without radius
            for (df, expr) in self.gather_scale_domain_expressions(scale_name) {
                data_expressions.push((df, expr, None));
            }
            return data_expressions;
        }

        for mark in &self.marks {
            // Get the appropriate DataFrame based on data source
            let df = match mark.data_source() {
                DataSource::Explicit => Arc::new(mark.data_context().dataframe().clone()),
                DataSource::Inherited => {
                    // Use plot-level data if available
                    if let Some(plot_data) = &self.data {
                        Arc::new(plot_data.clone())
                    } else {
                        // Skip this mark if no plot data is available
                        continue;
                    }
                }
            };

            // Check all encodings in the mark's data context
            let encodings = mark.data_context().encodings();

            // Create channel resolver for this mark
            let resolve_channel = Self::create_channel_resolver(mark.as_ref(), encodings, scales);

            for (channel, position_channel_value) in encodings {
                // Check if this channel uses our scale
                if let Some(channel_scale_name) = position_channel_value.scale_name(channel) {
                    if channel_scale_name == scale_name {
                        // Get the position expression
                        let position_expr = position_channel_value.expr().clone();

                        // Get radius expression from the mark
                        let radius_expr = mark.radius_expression(scale_name, &resolve_channel);

                        // Add to expressions with radius info
                        data_expressions.push((df.clone(), position_expr, radius_expr));
                    }
                }
            }
        }

        data_expressions
    }

    /// Apply default range to a scale based on plot area dimensions
    /// This is called during rendering when actual plot area dimensions are known
    /// (i.e., after padding has been subtracted by the layout/rendering system)
    pub fn apply_default_range(
        &self,
        scale: &mut Scale,
        name: &str,
        plot_area_width: f64,
        plot_area_height: f64,
    ) {
        if !scale.has_explicit_range() {
            // Check if this scale is mapped to a coordinate channel
            let coord_channel = self
                .scale_to_coord_channel
                .get(name)
                .map(|s| s.as_str())
                .unwrap_or(name);

            if let Some(default_range) =
                self.coord_system
                    .default_range(coord_channel, plot_area_width, plot_area_height)
            {
                *scale = scale.clone().range(default_range);
            }
        }
    }

    /// Apply default color range to a scale if no explicit range is set
    /// This is called during rendering for color channels
    pub fn apply_default_color_range(&self, scale: &mut Scale, name: &str) {
        use crate::scales::color_defaults::get_default_color_range_for_channel;

        if !scale.has_explicit_range() {
            // Get domain cardinality for discrete scales
            let domain_cardinality = scale.get_domain_cardinality();

            if let Some(default_range) = get_default_color_range_for_channel(
                name,
                scale.get_scale_type(),
                domain_cardinality,
            ) {
                *scale = scale.clone().range(default_range);
            }
        }
    }

    /// Apply default shape range to a scale if no explicit range is set
    /// This is called during rendering for shape channels
    pub fn apply_default_shape_range(&self, scale: &mut Scale) {
        use crate::scales::shape_defaults::DEFAULT_SHAPES;
        use datafusion::logical_expr::lit;

        if !scale.has_explicit_range() && scale.get_scale_type() == "ordinal" {
            // Get domain cardinality
            let domain_cardinality = scale.get_domain_cardinality();

            // Use the shared default shapes
            let all_shapes: Vec<_> = DEFAULT_SHAPES.iter().map(|&s| lit(s)).collect();

            // Use only as many shapes as needed based on domain cardinality
            let shape_range = if let Some(n) = domain_cardinality {
                all_shapes.into_iter().take(n).collect()
            } else {
                // If cardinality unknown, use all shapes
                all_shapes
            };

            *scale = scale.clone().range_discrete(shape_range);
        }
    }

    /// Add a controller for interactivity
    pub fn controller<T: Controller + 'static>(mut self, controller: T) -> Self {
        self.controllers.push(Box::new(controller));
        self
    }


    /// Collect all channels that need scales
    pub fn collect_channels_needing_scales(&self) -> std::collections::HashSet<String> {
        use std::collections::HashSet;
        let mut used_channels = HashSet::new();
        for mark in &self.marks {
            for (channel, channel_value) in mark.data_context().encodings() {
                if channel_value.scale_name(channel).is_some() {
                    used_channels.insert(channel.clone());
                }
            }
        }
        used_channels
    }

    /// Create a default scale for a channel
    pub async fn create_default_scale_for_channel(&self, channel: &str) -> Option<Scale> {
        Some(self.create_default_scale_for_channel_internal(channel))
    }

    /// Get preferred size for the plot (temporary placeholder)
    pub fn get_preferred_size(&self) -> Option<(f32, f32)> {
        // Return None to use defaults in renderer
        None
    }

    /// Measure padding required for axes, legends, etc. (temporary placeholder)
    pub fn measure_padding(&self, _width: f32, _height: f32) -> crate::render::Padding {
        // Return default padding for now
        crate::render::Padding {
            left: 60.0,
            right: 60.0,
            top: 30.0,
            bottom: 50.0,
        }
    }
}

impl Plot<Cartesian> {
    pub fn scale_x<F>(mut self, f: F) -> Self
    where
        F: Fn(Scale) -> Scale + Send + Sync + 'static,
    {
        self.scale_specs
            .insert("x".to_string(), ScaleSpec::Local(Arc::new(f)));
        self
    }

    /// Reference a named scale from parent layout for x channel
    pub fn scale_x_ref<S: Into<String>>(mut self, name: S) -> Self {
        self.scale_specs
            .insert("x".to_string(), ScaleSpec::Reference(name.into()));
        self
    }

    pub fn scale_y<F>(mut self, f: F) -> Self
    where
        F: Fn(Scale) -> Scale + Send + Sync + 'static,
    {
        self.scale_specs
            .insert("y".to_string(), ScaleSpec::Local(Arc::new(f)));
        self
    }

    /// Reference a named scale from parent layout for y channel
    pub fn scale_y_ref<S: Into<String>>(mut self, name: S) -> Self {
        self.scale_specs
            .insert("y".to_string(), ScaleSpec::Reference(name.into()));
        self
    }

    pub fn axis_x<F>(mut self, f: F) -> Self
    where
        F: FnOnce(<Cartesian as CoordinateSystem>::Axis) -> <Cartesian as CoordinateSystem>::Axis,
    {
        // Get count for x axes (this is the primary one, so index 0)
        let index = 0;

        // Get existing axis or create default
        let current = self
            .axes
            .remove("x")
            .unwrap_or_else(|| Cartesian::default_axis("x", index).unwrap());

        let axis = f(current);
        self.axes.insert("x".to_string(), axis);
        self
    }

    pub fn axis_y<F>(mut self, f: F) -> Self
    where
        F: FnOnce(<Cartesian as CoordinateSystem>::Axis) -> <Cartesian as CoordinateSystem>::Axis,
    {
        // Get count for y axes (this is the primary one, so index 0)
        let index = 0;

        // Get existing axis or create default
        let current = self
            .axes
            .remove("y")
            .unwrap_or_else(|| Cartesian::default_axis("y", index).unwrap());

        let axis = f(current);
        self.axes.insert("y".to_string(), axis);
        self
    }

    /// Add an alternative y-axis scale with a custom name
    pub fn scale_y_alt<S: Into<String>, F>(mut self, name: S, f: F) -> Self
    where
        F: Fn(Scale) -> Scale + Send + Sync + 'static,
    {
        let name = name.into();
        self.scale_specs
            .insert(name.clone(), ScaleSpec::Local(Arc::new(f)));
        // Map this scale to the y coordinate channel
        self.scale_to_coord_channel.insert(name, "y".to_string());
        self
    }

    /// Add an alternative x-axis scale with a custom name
    pub fn scale_x_alt<S: Into<String>, F>(mut self, name: S, f: F) -> Self
    where
        F: Fn(Scale) -> Scale + Send + Sync + 'static,
    {
        let name = name.into();
        self.scale_specs
            .insert(name.clone(), ScaleSpec::Local(Arc::new(f)));
        // Map this scale to the x coordinate channel
        self.scale_to_coord_channel.insert(name, "x".to_string());
        self
    }

    /// Configure an axis for a named y scale
    pub fn axis_y_alt<S: Into<String>, F>(mut self, scale_name: S, f: F) -> Self
    where
        F: FnOnce(<Cartesian as CoordinateSystem>::Axis) -> <Cartesian as CoordinateSystem>::Axis,
    {
        let scale_name = scale_name.into();
        // Get existing axis or create default with right position for alt axes
        let current = self.axes.remove(&scale_name).unwrap_or_else(|| {
            CartesianAxis::new()
                .position(AxisPosition::Right)
                .label_angle(0.0)
        });

        let axis = f(current);
        self.axes.insert(scale_name, axis);
        self
    }

    /// Configure an axis for a named x scale
    pub fn axis_x_alt<S: Into<String>, F>(mut self, scale_name: S, f: F) -> Self
    where
        F: FnOnce(<Cartesian as CoordinateSystem>::Axis) -> <Cartesian as CoordinateSystem>::Axis,
    {
        let scale_name = scale_name.into();
        // Get existing axis or create default with top position for alt axes
        let current = self.axes.remove(&scale_name).unwrap_or_else(|| {
            CartesianAxis::new()
                .position(AxisPosition::Top)
                .label_angle(0.0)
        });

        let axis = f(current);
        self.axes.insert(scale_name, axis);
        self
    }
}

impl Plot<Polar> {
    pub fn scale_r<F>(mut self, f: F) -> Self
    where
        F: Fn(Scale) -> Scale + Send + Sync + 'static,
    {
        self.scale_specs
            .insert("r".to_string(), ScaleSpec::Local(Arc::new(f)));
        self
    }

    /// Reference a named scale from parent layout for r channel
    pub fn scale_r_ref<S: Into<String>>(mut self, name: S) -> Self {
        self.scale_specs
            .insert("r".to_string(), ScaleSpec::Reference(name.into()));
        self
    }

    pub fn scale_theta<F>(mut self, f: F) -> Self
    where
        F: Fn(Scale) -> Scale + Send + Sync + 'static,
    {
        self.scale_specs
            .insert("theta".to_string(), ScaleSpec::Local(Arc::new(f)));
        self
    }

    /// Reference a named scale from parent layout for theta channel
    pub fn scale_theta_ref<S: Into<String>>(mut self, name: S) -> Self {
        self.scale_specs
            .insert("theta".to_string(), ScaleSpec::Reference(name.into()));
        self
    }
}

#[cfg(test)]
mod examples {
    use super::*;
    use crate::adjust::Jitter;
    use crate::legend::LegendPosition;
    use crate::marks::line::Line;
    use crate::transforms::{Bin, BinNd, Group, Stack};
    use datafusion::functions_aggregate::expr_fn::{count, sum};
    use datafusion::logical_expr::test::function_stub::avg;
    use datafusion::logical_expr::{ident, lit};
    use datafusion::prelude::{CsvReadOptions, SessionContext};

    #[allow(dead_code)]
    async fn example_basic_plot() -> Result<(), Box<dyn std::error::Error>> {
        // Create DataFrame using DataFusion
        let ctx = SessionContext::new();
        let df = ctx
            .read_csv("data.csv", CsvReadOptions::new())
            .await
            .unwrap();

        // Create plot with explicit axis configuration
        let _plot = Plot::new(Cartesian)
            .scale_x(|scale| scale.domain((0.0, 100.0)))
            .scale_y(|scale| scale.domain((0.0, 100.0)))
            .axis_x(|axis| axis.title("Date").grid(true))
            .mark(
                Line::new()
                    .data(df.clone())
                    .transform(
                        Bin::x("date")
                            .aggregate(avg(ident("temperature")))
                            .width(10.0),
                    )?
                    .stroke("station"),
            );

        // Using lambda to change defaults
        let _plot2 = Plot::new(Cartesian)
            .scale_x(|scale| scale) // Use default linear scale
            .axis_x(|axis| axis.title("Modified Title"));

        // Disable axis using visible(false)
        let _plot3 = Plot::new(Cartesian)
            .scale_x(|scale| scale) // Use default linear scale
            .axis_x(|axis| axis.visible(false));

        // Example with legends
        let _plot4 = Plot::new(Cartesian)
            .scale_y(|scale| scale.domain((0.0, 50.0)))
            .scale_fill(|scale| scale.domain((0.0, 100.0)))
            .scale_size(|scale| scale.domain((0.0, 1000.0)))
            .legend_fill(|legend| {
                legend
                    .title("Temperature (°C)")
                    .position(LegendPosition::Right)
            })
            .legend_size(|legend| {
                legend
                    .title("Population")
                    .position(LegendPosition::Bottom)
                    .symbol_size(15.0)
            })
            .mark(Line::new().x("date").y("value").stroke("category"));

        // Create plot
        let _plot = Plot::new(Polar)
            .scale_r(|scale| scale.domain((0.0, 100.0)))
            .scale_theta(|scale| scale.domain((0.0, 360.0)))
            .mark(
                Line::new()
                    .r("temperature")
                    .theta("other")
                    .stroke("station"),
            );

        Ok(())
    }

    #[allow(dead_code)]
    async fn example_transform_pipeline() -> Result<(), Box<dyn std::error::Error>> {
        use crate::marks::rect::Rect;
        use datafusion::logical_expr::col;

        // Create DataFrame using DataFusion
        let ctx = SessionContext::new();
        let df = ctx
            .read_csv("sales_data.csv", CsvReadOptions::new())
            .await
            .unwrap();

        // Example 1: Simple histogram with binning
        // The Bin transform sets x/x2 encodings in DataContext
        let _histogram = Plot::new(Cartesian).mark(
            Rect::new()
                .data(df.clone())
                .transform(Bin::x("price").width(10.0).aggregate(count(col("*"))))?
                // The transform automatically sets encodings in DataContext
                // x → price_bin_start
                // x2 → price_bin_end
                // y → count
                // .x("price_bin_start")
                // .x2("price_bin_end")
                // .y("count")
                .fill(lit("red"))
                .adjust(Jitter::new().x(5.0)),
        );

        // Example 2: Grouped bar chart with stacking
        // Demonstrates how transforms build on each other via DataContext
        let _stacked_bars = Plot::new(Cartesian).mark(
            Rect::new()
                .data(df.clone())
                // First transform: group and aggregate
                .transform(Group::xfill("month", "product").aggregate(sum(col("sales"))))?
                // The Group transform sets encodings:
                // x → month
                // fill → product
                // y → sum_sales
                .x("month")
                .fill("product")
                // Now apply stacking - it can reference :x, :y, :fill
                .transform(
                    Stack::y() // Looks for :x and :y channels
                        .order(crate::transforms::StackOrder::Sum),
                )?
                // Stack transform produces y1/y2 for rectangles
                .y("y_stack_start")
                .y2("y_stack_end"),
        );

        // Example 3: 2D histogram (heatmap)
        let _heatmap = Plot::new(Cartesian).mark(
            Rect::new()
                .data(df.clone())
                .transform(
                    BinNd::xy("price", "quantity")
                        .width_x(10.0)
                        .bins_y(20)
                        .aggregate(count(col("*")))
                        .extra_aggregate("fill", avg(col("profit"))),
                )?
                // BinNd sets multiple channel encodings:
                // x → price_bin_start, x2 → price_bin_end
                // y → quantity_bin_start, y2 → quantity_bin_end
                // value → count
                // And extra_aggregate adds:
                // fill → avg_profit
                .x("price_bin_start")
                .x2("price_bin_end")
                .y("quantity_bin_start")
                .y2("quantity_bin_end")
                .fill("avg_profit")
                .opacity("count"),
        );

        Ok(())
    }

    #[allow(dead_code)]
    async fn example_plot_level_data_and_faceting() -> Result<(), Box<dyn std::error::Error>> {
        use crate::marks::symbol::Symbol;
        use datafusion::prelude::*;

        // Create DataFrame using DataFusion
        let ctx = SessionContext::new();
        let df = ctx
            .sql(
                "
            SELECT 
                'USA' as country,
                'North America' as continent,
                80000.0 as gdp_per_capita,
                78.5 as life_expectancy,
                300.0 as population
            UNION ALL
            SELECT 'China', 'Asia', 10000.0, 76.0, 1400.0
            UNION ALL  
            SELECT 'Germany', 'Europe', 45000.0, 81.0, 83.0
        ",
            )
            .await?;

        // Plot-level data with faceting API
        let _faceted_plot = Plot::new(Cartesian)
            .data(df.clone()) // Plot-level data
            .facet_wrap("continent") // Simple wrap faceting
            .mark(
                Symbol::new() // Mark inherits plot data
                    .x("gdp_per_capita")
                    .y("life_expectancy")
                    .size("population"),
            );

        // Builder pattern for faceting
        let _builder_faceted = Plot::new(Cartesian)
            .data(df.clone())
            .facet(Facet::wrap("continent").columns(2)) // Using builder
            .mark(Symbol::new().x("gdp_per_capita").y("life_expectancy"));

        // Grid faceting example (placeholder for future)
        let _grid_faceted = Plot::new(Cartesian)
            .data(df.clone())
            .facet(
                Facet::grid() // TODO: Implement
                    .row("continent")
                    .column("year"),
            ) // TODO: Add year data
            .mark(Symbol::new().x("gdp_per_capita").y("life_expectancy"));

        // Backward compatibility: existing mark-level data continues to work
        let _existing_pattern = Plot::new(Cartesian).mark(
            Symbol::new()
                .data(df.clone()) // Mark-level data (existing)
                .x("gdp_per_capita")
                .y("life_expectancy")
                .size("population"),
        );

        // Advanced data inheritance patterns
        let _mixed_data_sources = Plot::new(Cartesian)
            .data(df.clone()) // Default data for faceting
            .facet_wrap("continent") // Facet on main data
            .mark(
                Symbol::new() // Uses plot data (inherited)
                    .x("gdp_per_capita")
                    .y("life_expectancy")
                    .size("population"),
            )
            .mark(
                Symbol::new() // Override with different data
                    .data(df.clone()) // Explicit data (must have "continent" column)
                    .x("gdp_per_capita")
                    .y("life_expectancy"),
            ) // Filter by facet values
            .mark(
                Symbol::new() // Reference mark in all facets
                    .data(df.clone())
                    .x("gdp_per_capita")
                    .y("life_expectancy")
                    .broadcast_to_facets(),
            ); // Show in every facet

        // Enhanced resolution system examples
        let _enhanced_resolution = Plot::new(Cartesian)
            .data(df.clone())
            .facet(
                Facet::wrap("continent")
                    .columns(2)
                    .resolve_scale("x", Resolution::Independent) // Each facet has own x-scale
                    .resolve_scale("y", Resolution::SharedCols) // Columns share y-scale
                    .resolve_axis("x", Resolution::Independent) // Independent x-axes
                    .resolve_legend("color", Resolution::Shared),
            ) // Single color legend
            .mark(
                Symbol::new()
                    .x("gdp_per_capita")
                    .y("life_expectancy")
                    .fill("development_status"),
            );

        // Grid faceting with advanced resolution
        let _grid_resolution = Plot::new(Cartesian)
            .data(df.clone())
            .facet(
                Facet::grid().row("continent").column("year").resolve(
                    FacetResolve::new()
                        .scale("x", Resolution::Shared) // Same time axis
                        .scale("y", Resolution::SharedCols) // Same metric per column
                        .scale("color", Resolution::SharedRows) // Same colors per region
                        .axis("x", Resolution::Shared) // Single x-axis
                        .axis("y", Resolution::SharedCols) // Column-specific y-axes
                        .legend("color", Resolution::SharedRows),
                ),
            ) // Row-specific legends
            .mark(Line::new().x("year").y("gdp_per_capita").stroke("country"));

        println!("✅ Enhanced facet resolution system implemented!");
        println!("🔧 TODO: Implement full rendering pipeline and domain calculation");

        Ok(())
    }

    #[allow(dead_code)]
    async fn example_interactive_plot() -> Result<(), Box<dyn std::error::Error>> {
        use crate::controllers::{BoxSelect, PanZoom};
        use crate::marks::symbol::Symbol;
        use datafusion::prelude::*;

        // Create DataFrame using DataFusion
        let ctx = SessionContext::new();
        let df = ctx
            .sql(
                "
            SELECT 
                rand() * 100.0 as x,
                rand() * 100.0 as y,
                rand() * 10.0 as size
            FROM generate_series(1, 1000)
        ",
            )
            .await?;

        // Example 1: Simple pan/zoom controller
        let _interactive_scatter = Plot::new(Cartesian)
            .data(df.clone())
            .controller(PanZoom::new()) // Declarative controller!
            .mark(Symbol::new().x("x").y("y").size("size"));

        // Example 2: Box selection controller
        let _box_select_plot = Plot::new(Cartesian)
            .data(df.clone())
            .controller(BoxSelect::new())
            .mark(Symbol::new().x("x").y("y").fill(lit("#4682b4")));

        // Example 3: Multiple controllers
        let _multi_controller = Plot::new(Cartesian)
            .data(df.clone())
            .controller(PanZoom::new())
            .controller(BoxSelect::new())
            .mark(Symbol::new().x("x").y("y"));

        // Example 4: Controllers with faceting
        let _faceted_interactive = Plot::new(Cartesian)
            .data(df.clone())
            .facet(
                Facet::wrap("category")
                    .columns(2)
                    .resolve_scale("x", Resolution::Independent) // Independent x scales
                    .resolve_scale("y", Resolution::Shared),
            ) // Shared y scale
            .controller(PanZoom::new()) // Controller respects resolve config
            .mark(Symbol::new().x("x").y("y"));

        println!("✅ Declarative controller API demonstrated!");
        println!("🔧 TODO: Implement controller runtime and event handling");

        Ok(())
    }
}
