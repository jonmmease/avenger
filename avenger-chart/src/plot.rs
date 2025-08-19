use crate::axis::{AxisPosition, CartesianAxis};
use crate::coords::{Cartesian, CoordinateSystem, Polar};
use crate::legend::Legend;
use crate::marks::{Mark, RadiusExpression};
use crate::scales::Scale;
use datafusion::dataframe::DataFrame;
use datafusion::logical_expr::lit;
use std::collections::HashMap;
use std::sync::Arc;

/// Type alias for scale domain expressions with optional radius information
type ScaleDomainWithRadius = Vec<(
    Arc<DataFrame>,
    datafusion::logical_expr::Expr,
    Option<RadiusExpression>,
)>;

/// How a scale is defined for a channel
#[derive(Clone)]
pub enum ScaleSpec {
    /// Scale defined locally on this plot with a configuration function
    Local(Arc<dyn Fn(Scale) -> Scale + Send + Sync>),
    /// Reference to a scale defined in parent layout
    Reference(String),
}

/// Minimal plot title configuration
#[derive(Clone, Debug)]
pub struct PlotTitle {
    pub text: String,
    pub font_size: f32,
    pub font_family: String,
}

/// Minimal plot subtitle configuration
#[derive(Clone, Debug)]
pub struct PlotSubtitle {
    pub text: String,
    pub font_size: f32,
    pub font_family: String,
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

    /// Scale specifications (local or referenced)
    pub(crate) scale_specs: HashMap<String, ScaleSpec>,

    /// Mapping from scale names to their coordinate channel
    /// e.g., "y_squared" -> "y", "y_temperature" -> "y"
    scale_to_coord_channel: HashMap<String, String>,

    /// Preferred size for the plot canvas
    preferred_size: Option<(f32, f32)>,

    /// Optional plot title rendered by the layout system
    pub(crate) title: Option<PlotTitle>,

    /// Optional plot subtitle rendered by the layout system
    pub(crate) subtitle: Option<PlotSubtitle>,
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
    // Strip styling will be added when faceting is fully implemented
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
            scale_specs: HashMap::new(),
            scale_to_coord_channel: HashMap::new(),
            preferred_size: None,
            title: None,
            subtitle: None,
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
            if let Some(channel_value) = mark.data_context().channels().get(channel) {
                // Get the dataframe for this mark
                let df = match mark.data_source() {
                    crate::marks::DataSource::Explicit => mark.data_context().dataframe(),
                    crate::marks::DataSource::Inherited => {
                        if let Some(plot_df) = &self.data {
                            Some(plot_df)
                        } else {
                            continue;
                        }
                    }
                };

                // Try to get the data type of the expression
                if let Some(df) = df {
                    let schema = df.schema();
                    if let Ok(expr_type) = channel_value.expr().get_type(schema) {
                        data_type = Some(expr_type);
                        mark_type = Some(mark.mark_type());
                        break;
                    }
                }
            }
        }

        // Create a scale based on the inferred type
        let scale_type = if channel == "stroke_width" {
            // stroke_width MUST always use ordinal scale with discrete domain
            "ordinal"
        } else if let Some(dt) = &data_type {
            infer_scale_type_with_mark(channel, dt, mark_type)
        } else {
            // Fallback to channel-based defaults
            match channel {
                // Color and discrete visual channels default to ordinal
                "fill" | "stroke" | "color" | "shape" | "stroke_dash" => "ordinal",
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
            "stroke_width" => {
                // stroke_width ALWAYS uses discrete range
                scale = scale.range_discrete(vec![
                    lit(1.0),
                    lit(2.0),
                    lit(3.0),
                    lit(4.0),
                    lit(5.0),
                    lit(6.0),
                    lit(7.0),
                    lit(8.0),
                ])
            }
            "stroke_dash" => {
                // Use discrete dash patterns for stroke_dash
                if scale_type == "ordinal" {
                    use crate::scales::dash_defaults::DEFAULT_DASH_PATTERN_NAMES;
                    // Use the first 8 default pattern names
                    let patterns: Vec<_> = DEFAULT_DASH_PATTERN_NAMES
                        .iter()
                        .take(8)
                        .map(|name| lit(*name))
                        .collect();
                    scale = scale.range_discrete(patterns)
                }
            }
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

    /// Get the data to use for faceting operations
    /// Priority: explicit facet data > plot data > aggregated mark data
    pub fn get_faceting_data(&self) -> Option<DataFrame> {
        // Returns plot data if available
        // Full data resolution logic will be implemented with complete faceting support
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
                let mut base_scale = self.create_default_scale_for_channel_internal(name);

                // Gather and apply domain expressions to give the scale a data-driven domain
                // This happens BEFORE the user's lambda, so the user can override if desired
                if let Ok(domain_exprs) = self.gather_scale_domain_expressions(name) {
                    if !domain_exprs.is_empty() {
                        base_scale = base_scale.domain_data_fields_internal(domain_exprs);
                    }
                }

                // Apply user's transformation
                let mut user_scale = f(base_scale);

                // Enforce stroke_width must always be ordinal
                if name == "stroke_width" {
                    // Force ordinal scale type even if user tried to set it to linear
                    user_scale = user_scale.scale_type("ordinal");
                    // Ensure discrete range is used
                    if !user_scale.has_explicit_range() {
                        user_scale = user_scale.range_discrete(vec![
                            lit(1.0),
                            lit(2.0),
                            lit(3.0),
                            lit(4.0),
                            lit(5.0),
                            lit(6.0),
                            lit(7.0),
                            lit(8.0),
                        ]);
                    }
                }

                user_scale
            }
            Some(ScaleSpec::Reference(_)) => {
                todo!("Referenced scales are not yet implemented");
            }
            None => {
                let mut base_scale = self.create_default_scale_for_channel_internal(name);

                // For scales without user configuration, also apply data domain
                if let Ok(domain_exprs) = self.gather_scale_domain_expressions(name) {
                    if !domain_exprs.is_empty() {
                        base_scale = base_scale.domain_data_fields_internal(domain_exprs);
                    }
                }

                base_scale
            }
        }
    }

    /// Gather mark data and encoding expressions that use this scale
    pub fn gather_scale_domain_expressions(
        &self,
        scale_name: &str,
    ) -> Result<
        Vec<(Arc<DataFrame>, datafusion::logical_expr::Expr)>,
        crate::error::AvengerChartError,
    > {
        use crate::marks::DataSource;

        let mut data_expressions = Vec::new();

        for mark in &self.marks {
            // Get the appropriate DataFrame based on data source
            let df = match mark.data_source() {
                DataSource::Explicit => {
                    if let Some(df) = mark.data_context().dataframe() {
                        Arc::new(df.clone())
                    } else {
                        continue;
                    }
                }
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

            // Get channels and resolve channel references
            let channels = mark.data_context().channels();
            let resolved_channels = crate::channel_resolution::resolve_all_channel_refs(channels)?;

            // Check all encodings in the mark's data context
            for (channel, channel_value) in &resolved_channels {
                // Check if this channel uses our scale
                // Get the scale name this channel would use
                if let Some(channel_scale_name) = channel_value.scale_name(channel) {
                    if channel_scale_name == scale_name {
                        // Get the expression - we'll cast later if needed
                        let expr = channel_value.expr().clone();
                        data_expressions.push((df.clone(), expr));
                    }
                }
            }
        }

        Ok(data_expressions)
    }

    /// Create a channel resolver function for non-positional channels (which are already configured)
    /// This is used primarily for radius calculations that need size/stroke_width expressions
    fn create_channel_resolver<'a>(
        mark: &'a dyn Mark<C>,
        encodings: &'a indexmap::IndexMap<String, crate::marks::ChannelValue>,
        configured_scales: &'a HashMap<String, avenger_scales::scales::ConfiguredScale>,
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

                        // Use configured scales (for non-positional channels like size, stroke_width)
                        if let Some(configured) = configured_scales.get(&scale_key) {
                            use crate::scales::ConfiguredScaleDataFusionExt;
                            
                            // Use ConfiguredScale.to_expr() 
                            if let Some(band_value) = band {
                                configured
                                    .to_expr_with_band(channel_value.expr().clone(), *band_value)
                                    .unwrap_or_else(|_| channel_value.expr().clone())
                            } else {
                                configured
                                    .to_expr(channel_value.expr().clone())
                                    .unwrap_or_else(|_| channel_value.expr().clone())
                            }
                        } else {
                            // No scale configured for this channel - use raw expression
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
    /// Uses configured scales for non-positional channels (size, stroke_width) needed for radius
    pub fn gather_scale_domain_expressions_with_radius(
        &self,
        scale_name: &str,
        configured_scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
    ) -> Result<ScaleDomainWithRadius, crate::error::AvengerChartError> {
        use crate::marks::DataSource;

        let mut data_expressions = Vec::new();

        // Only gather radius for positional scales (including x2, y2 which map to x, y scales)
        let is_positional = matches!(scale_name, "x" | "y");
        if !is_positional {
            // For non-positional scales, return without radius
            for (df, expr) in self.gather_scale_domain_expressions(scale_name)? {
                data_expressions.push((df, expr, None));
            }
            return Ok(data_expressions);
        }

        for mark in &self.marks {
            // Get the appropriate DataFrame based on data source
            let df = match mark.data_source() {
                DataSource::Explicit => {
                    if let Some(df) = mark.data_context().dataframe() {
                        Arc::new(df.clone())
                    } else {
                        continue;
                    }
                }
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

            // Get channels and resolve channel references
            let encodings = mark.data_context().channels();
            let resolved_encodings =
                crate::channel_resolution::resolve_all_channel_refs(encodings)?;

            // Create channel resolver for this mark (uses configured non-positional scales)
            let resolve_channel = Self::create_channel_resolver(
                mark.as_ref(),
                &resolved_encodings,
                configured_scales,
            );

            for (channel, position_channel_value) in &resolved_encodings {
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

        Ok(data_expressions)
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

    /// Apply default dash range to a scale if no explicit range is set
    /// This is called during rendering for stroke_dash channels
    pub fn apply_default_dash_range(&self, scale: &mut Scale) {
        use crate::scales::dash_defaults::DEFAULT_DASH_PATTERN_NAMES;
        use datafusion::logical_expr::lit;

        if !scale.has_explicit_range() && scale.get_scale_type() == "ordinal" {
            // Get domain cardinality
            let domain_cardinality = scale.get_domain_cardinality();

            // Use the shared default dash pattern names
            let all_patterns: Vec<_> = DEFAULT_DASH_PATTERN_NAMES
                .iter()
                .map(|name| lit(*name))
                .collect();

            // Use only as many patterns as needed based on domain cardinality
            let dash_range = if let Some(n) = domain_cardinality {
                all_patterns.into_iter().take(n).collect()
            } else {
                // If cardinality unknown, use all patterns
                all_patterns
            };

            *scale = scale.clone().range_discrete(dash_range);
        }
    }

    /// Collect all channels that need scales
    pub fn collect_channels_needing_scales(&self) -> std::collections::HashSet<String> {
        use std::collections::HashSet;
        let mut used_channels = HashSet::new();
        for mark in &self.marks {
            for (channel_name, channel_value) in mark.data_context().channels() {
                if channel_value.scale_name(channel_name).is_some() {
                    used_channels.insert(channel_name.clone());
                }
            }
        }
        used_channels
    }

    /// Create a default scale for a channel
    pub async fn create_default_scale_for_channel(&self, channel: &str) -> Option<Scale> {
        Some(self.create_default_scale_for_channel_internal(channel))
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

    /// Set a simple plot title. For advanced styling, a richer API can be added later.
    pub fn title(mut self, text: impl Into<String>) -> Self {
        self.title = Some(PlotTitle {
            text: text.into(),
            font_size: 18.0,
            font_family: "Atkinson Hyperlegible Next".to_string(),
        });
        self
    }

    /// Set a simple plot subtitle. For advanced styling, a richer API can be added later.
    pub fn subtitle(mut self, text: impl Into<String>) -> Self {
        self.subtitle = Some(PlotSubtitle {
            text: text.into(),
            font_size: 14.0,
            font_family: "Atkinson Hyperlegible Next".to_string(),
        });
        self
    }

    /// Access the configured title
    pub fn get_title(&self) -> Option<&PlotTitle> {
        self.title.as_ref()
    }

    /// Access the configured subtitle
    pub fn get_subtitle(&self) -> Option<&PlotSubtitle> {
        self.subtitle.as_ref()
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
