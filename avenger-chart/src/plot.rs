use crate::coords::CoordinateSystem;
use crate::error::AvengerChartError;
use crate::legend::Legend;
use crate::marks::{ChannelValue, Mark, RadiusExpression};
use crate::scales::{Auto, Scale};
use datafusion::dataframe::DataFrame;
use indexmap::IndexMap;
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

/// How an axis is customized for a channel
#[derive(Clone)]
pub enum AxisSpec<A> {
    /// Axis customized locally with a configuration function
    Local(Arc<dyn Fn(A) -> A + Send + Sync>),
    /// Reference to an axis defined in parent layout (for future use)
    Reference(String),
}

/// Alignment options for title and subtitle
#[derive(Clone, Debug, Copy, PartialEq, Default)]
pub enum TitleAlign {
    /// Title/subtitle spans entire width minus padding columns
    #[default]
    FullWidth,
    /// Title/subtitle only spans the plot area column
    PlotAreaOnly,
}

/// Minimal plot title configuration
#[derive(Clone, Debug)]
pub struct PlotTitle {
    pub text: String,
    pub font_size: f32,
    pub font_family: String,
    pub align: TitleAlign,
}

/// Minimal plot subtitle configuration
#[derive(Clone, Debug)]
pub struct PlotSubtitle {
    pub text: String,
    pub font_size: f32,
    pub font_family: String,
    pub align: TitleAlign,
}

pub struct Plot<C: CoordinateSystem> {
    coord_system: C,
    pub(crate) axis_specs: HashMap<String, AxisSpec<C::Axis>>,
    pub(crate) legends: IndexMap<String, Legend>,
    pub(crate) marks: Vec<Box<dyn Mark<C>>>,

    /// Plot-level data for faceting and mark inheritance
    pub(crate) data: Option<DataFrame>,

    /// Faceting configuration
    facet_spec: Option<FacetSpec>,

    /// Scale specifications (local or referenced)
    pub(crate) scale_specs: HashMap<String, ScaleSpec>,

    /// Mapping from scale names to their coordinate channel
    /// e.g., "y_squared" -> "y", "y_temperature" -> "y"
    pub(crate) scale_to_coord_channel: HashMap<String, String>,

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

    /// Configure axis resolution for a positional channel
    pub fn axis<C: Into<String>>(mut self, channel: C, resolution: Resolution) -> Self {
        self.axes.insert(channel.into(), resolution);
        self
    }

    /// Configure legend resolution for a non-positional channel
    pub fn legend<C: Into<String>>(mut self, channel: C, resolution: Resolution) -> Self {
        self.legends.insert(channel.into(), resolution);
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
    pub fn with_coord(coord_system: C) -> Self {
        Plot {
            coord_system,
            axis_specs: HashMap::new(),
            legends: IndexMap::new(),
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
    pub fn axis_specs(&self) -> &HashMap<String, AxisSpec<C::Axis>> {
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

    /// Internal helper to create a default scale for a channel
    fn create_default_scale_for_channel_internal(
        &self,
        channel: &str,
    ) -> Result<Scale, AvengerChartError> {
        // Try to infer the data type and use mark-based scale preferences
        let mut scale_impl = None;
        let mut data_type = None;
        let mut found_mark = None;

        // Look through marks to find the expression for this channel
        for mark in &self.marks {
            let channels = mark.data_context().channels();

            // Try to resolve channel references first
            let resolved_channels =
                match crate::channel_resolution::resolve_all_channel_refs(channels) {
                    Ok(resolved) => resolved,
                    Err(e) => {
                        // If resolution failed (e.g., due to cycles), return an error
                        if channels.contains_key(channel) {
                            return Err(AvengerChartError::InternalError(format!(
                                "Cannot create scale for channel '{}': {}",
                                channel, e
                            )));
                        }
                        // Channel doesn't exist in this mark, continue to next
                        continue;
                    }
                };

            if let Some(channel_value) = resolved_channels.get(channel) {
                // Get the dataframe for this mark
                // Use mark's explicit data if available, otherwise inherit from plot
                let df = mark.data_context().dataframe().or(self.data.as_ref());

                // Try to get the data type of the channel
                if let Some(df) = df {
                    let schema = df.schema();
                    if let Some(dt) = channel_value.get_data_type(schema) {
                        data_type = Some(dt.clone());

                        // Try scale type preference in order:
                        // 1. Mark's preference for this specific channel
                        // 2. Coordinate system's preference (typically for position channels)
                        scale_impl = mark.preferred_scale_type(channel, &dt)
                            .or_else(|| self.coord_system.preferred_scale_type(channel, &dt));
                        
                        found_mark = Some(mark);
                        break;
                    }
                }
            }
        }

        let scale_impl = scale_impl.ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Failed to infer scale implementation for channel '{}'",
                channel
            ))
        })?;
        let scale_type = scale_impl.scale_type();
        let mut scale = Scale::<Auto>::from_impl(scale_impl.clone());

        // Apply default options based on channel and scale type
        if let (Some(dt), Some(mark)) = (&data_type, found_mark) {
            let mut default_options = HashMap::new();

            // First get coordinate system defaults (for all channels, not just position)
            let coord_options = self.coord_system.default_scale_options(channel, scale_type);
            default_options.extend(coord_options);

            // Then get mark-specific scale option preferences
            // We already know this mark has the channel since we found it above
            let mark_options = mark.default_scale_options(channel, scale_type, dt);
            // Mark preferences override coordinate system defaults
            default_options.extend(mark_options);

            for (key, value) in default_options {
                scale = scale.option(&key, value);
            }
        }

        Ok(scale)
    }

    pub fn mark<M: Mark<C> + 'static>(mut self, mark: M) -> Self {
        // Extract scale and legend configurations from the mark's channels
        self.extract_channel_configs(&mark);

        // Add the mark
        self.marks.push(Box::new(mark));
        self
    }

    /// Extract scale and legend configurations from a mark's channels
    fn extract_channel_configs(&mut self, mark: &impl Mark<C>) {
        use crate::marks::ChannelValue;

        // Get all channel encodings from the mark
        let encodings = mark.data_context().channels();

        // Try to resolve channel references, but if it fails (e.g., due to conditional references),
        // we still want to extract configs from non-reference channels
        let resolved_encodings =
            match crate::channel_resolution::resolve_all_channel_refs(encodings) {
                Ok(resolved) => resolved,
                Err(_) => {
                    // Resolution failed (probably due to conditional references)
                    // Use original encodings - we'll handle the error later during rendering
                    encodings.clone()
                }
            };

        for (channel_name, channel_value) in resolved_encodings {
            // Extract scale and legend configs
            let (scale_config, legend_config) = match channel_value.clone() {
                ChannelValue::Scaled {
                    scale_config,
                    legend_config,
                    ..
                }
                | ChannelValue::Conditional {
                    scale_config,
                    legend_config,
                    ..
                } => (scale_config, legend_config),
                ChannelValue::Value { .. } => {
                    // No scale or legend for identity mappings
                    continue;
                }
            };

            // Determine scale name
            let scale_key = match channel_value {
                ChannelValue::Scaled { scale_name, .. } => {
                    scale_name.as_deref().unwrap_or(&channel_name).to_string()
                }
                ChannelValue::Conditional { .. } => {
                    // Conditional always uses channel name
                    channel_name.to_string()
                }
                _ => unreachable!(),
            };

            // Extract scale config if present
            if let Some(config) = scale_config {
                // Only insert if not already configured at plot level
                self.scale_specs
                    .entry(scale_key.clone())
                    .or_insert_with(|| ScaleSpec::Local(config.clone()));
            }

            // Extract legend config if present
            if let Some(config) = legend_config {
                // Apply config to existing or new legend
                let legend = self
                    .legends
                    .shift_remove(channel_name.as_str())
                    .unwrap_or_default();
                let configured = config(legend);
                self.legends.insert(channel_name.clone(), configured);
            }
        }
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
    pub fn get_scale(&self, name: &str) -> Result<Scale, AvengerChartError> {
        use crate::marks::channel::strip_trailing_numbers;

        // Strip trailing numbers to get the base scale name
        // e.g., "x2" -> "x", "y2" -> "y"
        let base_name = strip_trailing_numbers(name);

        match self.scale_specs.get(base_name) {
            Some(ScaleSpec::Local(f)) => {
                let mut base_scale = self.create_default_scale_for_channel_internal(base_name)?;

                // Gather and apply domain expressions to give the scale a data-driven domain
                // This happens BEFORE the user's lambda, so the user can override if desired
                if let Ok(domain_exprs) = self.gather_scale_domain_expressions(base_name) {
                    if !domain_exprs.is_empty() {
                        base_scale = base_scale.domain_data_fields(domain_exprs);
                    }
                }

                // Apply user's transformation
                let user_scale = f(base_scale);
                Ok(user_scale)
            }
            Some(ScaleSpec::Reference(_)) => {
                todo!("Referenced scales are not yet implemented");
            }
            None => {
                let mut base_scale = self.create_default_scale_for_channel_internal(base_name)?;

                // For scales without user configuration, also apply data domain
                if let Ok(domain_exprs) = self.gather_scale_domain_expressions(base_name) {
                    if !domain_exprs.is_empty() {
                        base_scale = base_scale.domain_data_fields(domain_exprs);
                    }
                }

                Ok(base_scale)
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
        let mut data_expressions = Vec::new();

        for mark in &self.marks {
            // Get channels and resolve references first to check if columns are referenced
            let channels = mark.data_context().channels();
            let resolved_channels = crate::channel_resolution::resolve_all_channel_refs(channels)?;

            // Check if any expressions reference columns
            let references_columns =
                resolved_channels
                    .values()
                    .any(|channel_value| match channel_value {
                        ChannelValue::Scaled { expr, .. } | ChannelValue::Value { expr } => {
                            !expr.column_refs().is_empty()
                        }
                        ChannelValue::Conditional {
                            conditions,
                            otherwise,
                            ..
                        } => {
                            conditions.iter().any(|(condition, value)| {
                                !condition.column_refs().is_empty()
                                    || !value.expr().column_refs().is_empty()
                            }) || !otherwise.expr().column_refs().is_empty()
                        }
                    });

            // Determine DataFrame for this mark
            let df = if let Some(mark_df) = mark.data_context().dataframe() {
                // Mark has explicit data
                Arc::new(mark_df.clone())
            } else if !references_columns {
                // No column references - skip domain inference for unit marks
                continue;
            } else if let Some(plot_data) = &self.data {
                // Inherit from plot
                Arc::new(plot_data.clone())
            } else {
                // No data available - skip this mark
                continue;
            };

            // Already resolved channels above

            // Check all encodings in the mark's data context
            for (channel, channel_value) in &resolved_channels {
                // Check if this channel uses our scale
                // Get the scale name this channel would use
                if let Some(channel_scale_name) = channel_value.get_scale_name(channel) {
                    if channel_scale_name == scale_name {
                        // Get the expressions - handle conditional values properly
                        match channel_value {
                            crate::marks::ChannelValue::Scaled { expr, .. }
                            | crate::marks::ChannelValue::Value { expr } => {
                                data_expressions.push((df.clone(), expr.clone()));
                            }
                            crate::marks::ChannelValue::Conditional {
                                conditions,
                                otherwise,
                                ..
                            } => {
                                // For conditional values, we need to extract field expressions
                                // (not literal values) for domain inference
                                use crate::marks::channel::ConditionalValue;

                                // Add field expressions from conditions
                                for (_, value) in conditions {
                                    if let ConditionalValue::Scaled { expr } = value {
                                        data_expressions.push((df.clone(), expr.clone()));
                                    }
                                    // Skip ConditionalValue::Value as those are literals
                                }

                                // Add field expression from otherwise branch if it's a field
                                if let ConditionalValue::Scaled { expr } = otherwise {
                                    data_expressions.push((df.clone(), expr.clone()));
                                }
                            }
                        }
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
                    ChannelValue::Value { expr } => {
                        // No scaling requested
                        expr.clone()
                    }
                    ChannelValue::Scaled {
                        expr,
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
                                    .to_expr_with_band(expr.clone(), *band_value)
                                    .unwrap_or_else(|_| expr.clone())
                            } else {
                                configured
                                    .to_expr(expr.clone())
                                    .unwrap_or_else(|_| expr.clone())
                            }
                        } else {
                            // No scale configured for this channel - use raw expression
                            expr.clone()
                        }
                    }
                    ChannelValue::Conditional { .. } => {
                        // TODO: Conditional encoding resolution will be handled in a later phase
                        // For now, return a placeholder
                        lit(datafusion::scalar::ScalarValue::Null)
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
        let mut data_expressions = Vec::new();

        // Only gather radius for Cartesian x/y scales (including x2, y2 which map to x, y scales)
        // Polar coordinates (r, theta) don't use radius-based padding
        let needs_radius = matches!(scale_name, "x" | "y");
        if !needs_radius {
            // For non-positional scales, return without radius
            for (df, expr) in self.gather_scale_domain_expressions(scale_name)? {
                data_expressions.push((df, expr, None));
            }
            return Ok(data_expressions);
        }

        for mark in &self.marks {
            // Get channels and resolve references first
            let encodings = mark.data_context().channels();
            let resolved_encodings =
                crate::channel_resolution::resolve_all_channel_refs(encodings)?;

            // Check if any expressions reference columns
            let references_columns =
                resolved_encodings
                    .values()
                    .any(|channel_value| match channel_value {
                        ChannelValue::Scaled { expr, .. } | ChannelValue::Value { expr } => {
                            !expr.column_refs().is_empty()
                        }
                        ChannelValue::Conditional {
                            conditions,
                            otherwise,
                            ..
                        } => {
                            conditions.iter().any(|(condition, value)| {
                                !condition.column_refs().is_empty()
                                    || !value.expr().column_refs().is_empty()
                            }) || !otherwise.expr().column_refs().is_empty()
                        }
                    });

            // Determine DataFrame for this mark
            let df = if let Some(mark_df) = mark.data_context().dataframe() {
                // Mark has explicit data
                Arc::new(mark_df.clone())
            } else if !references_columns {
                // No column references - skip domain inference for unit marks
                continue;
            } else if let Some(plot_data) = &self.data {
                // Inherit from plot
                Arc::new(plot_data.clone())
            } else {
                // No data available - skip this mark
                continue;
            };

            // Create channel resolver for this mark (uses configured non-positional scales)
            let resolve_channel = Self::create_channel_resolver(
                mark.as_ref(),
                &resolved_encodings,
                configured_scales,
            );

            for (channel, position_channel_value) in &resolved_encodings {
                // Check if this channel uses our scale
                if let Some(channel_scale_name) = position_channel_value.get_scale_name(channel) {
                    if channel_scale_name == scale_name {
                        // Get the position expressions - handle conditional values properly
                        match position_channel_value {
                            crate::marks::ChannelValue::Scaled { expr, .. }
                            | crate::marks::ChannelValue::Value { expr } => {
                                // Get radius expression from the mark
                                let radius_expr =
                                    mark.radius_expression(scale_name, &resolve_channel);
                                data_expressions.push((df.clone(), expr.clone(), radius_expr));
                            }
                            crate::marks::ChannelValue::Conditional {
                                conditions,
                                otherwise,
                                ..
                            } => {
                                // For conditional values, we need to extract field expressions
                                use crate::marks::channel::ConditionalValue;

                                // For radius expressions, we use the mark's radius for all branches
                                // since radius doesn't vary by condition
                                let radius_expr =
                                    mark.radius_expression(scale_name, &resolve_channel);

                                // Add field expressions from conditions
                                for (_, value) in conditions {
                                    if let ConditionalValue::Scaled { expr } = value {
                                        data_expressions.push((
                                            df.clone(),
                                            expr.clone(),
                                            radius_expr.clone(),
                                        ));
                                    }
                                }

                                // Add field expression from otherwise branch if it's a field
                                if let ConditionalValue::Scaled { expr } = otherwise {
                                    data_expressions.push((df.clone(), expr.clone(), radius_expr));
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(data_expressions)
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

    /// Collect all channels that need scales
    pub fn collect_channels_needing_scales(&self) -> std::collections::HashSet<String> {
        use std::collections::HashSet;
        let mut used_channels = HashSet::new();
        for mark in &self.marks {
            // Get channels and resolve references first
            let encodings = mark.data_context().channels();
            // Try to resolve, but use original channels if resolution fails
            let resolved_encodings = crate::channel_resolution::resolve_all_channel_refs(encodings)
                .unwrap_or_else(|_| encodings.clone());
            for (channel_name, channel_value) in resolved_encodings {
                if channel_value.get_scale_name(&channel_name).is_some() {
                    used_channels.insert(channel_name.clone());
                }
            }
        }
        used_channels
    }

    /// Create a default scale for a channel
    pub async fn create_default_scale_for_channel(&self, channel: &str) -> Option<Scale> {
        self.create_default_scale_for_channel_internal(channel).ok()
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
            align: TitleAlign::default(),
        });
        self
    }

    /// Configure the title with a closure for advanced options
    pub fn configure_title<F>(mut self, text: impl Into<String>, f: F) -> Self
    where
        F: FnOnce(PlotTitle) -> PlotTitle,
    {
        let title = PlotTitle {
            text: text.into(),
            font_size: 18.0,
            font_family: "Atkinson Hyperlegible Next".to_string(),
            align: TitleAlign::default(),
        };
        self.title = Some(f(title));
        self
    }

    /// Set a simple plot subtitle. For advanced styling, a richer API can be added later.
    pub fn subtitle(mut self, text: impl Into<String>) -> Self {
        self.subtitle = Some(PlotSubtitle {
            text: text.into(),
            font_size: 14.0,
            font_family: "Atkinson Hyperlegible Next".to_string(),
            align: TitleAlign::default(),
        });
        self
    }

    /// Configure the subtitle with a closure for advanced options
    pub fn configure_subtitle<F>(mut self, text: impl Into<String>, f: F) -> Self
    where
        F: FnOnce(PlotSubtitle) -> PlotSubtitle,
    {
        let subtitle = PlotSubtitle {
            text: text.into(),
            font_size: 14.0,
            font_family: "Atkinson Hyperlegible Next".to_string(),
            align: TitleAlign::default(),
        };
        self.subtitle = Some(f(subtitle));
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
}
