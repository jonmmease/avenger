//! Scale builder pattern for caching expensive data queries
//!
//! This module implements a pattern that separates expensive async data queries
//! from cheap sync scale construction. The key insight is:
//!
//! - **Data queries are expensive** (milliseconds to seconds for large datasets)
//! - **Scale construction is cheap** (microseconds for mathematical operations)
//!
//! By caching query results and rebuilding scales on-demand with current dimensions,
//! we achieve both performance (no duplicate queries) and correctness (scales always
//! match current plot dimensions).
//!
//! ## Design Patterns
//!
//! ### Standard Scales
//! Cache final data extents (min/max or unique values). These don't depend on
//! plot dimensions, so they can be computed once and reused.
//!
//! ### Radius-Aware Scales
//! Cache raw position and radius data vectors. The domain padding calculation
//! depends on plot dimensions (via range_width), so we recompute the domain
//! on each build using cached data. This trades cheap math (microseconds) for
//! avoiding duplicate queries (milliseconds to seconds).

use std::collections::HashMap;

use avenger_scales::scales::{
    RangeKind, ScaleImpl, domain_solver::compute_domain_from_data_with_padding_linear,
};
use datafusion::{arrow::datatypes::DataType, logical_expr::lit, prelude::SessionContext};
use datafusion_common::ScalarValue;
use datafusion_proto::protobuf::LogicalExprNode;
use indexmap::IndexMap;
use tracing::{debug, trace};

use avenger_chart_core::{
    AvengerChartError, Maybe, ResolvedDomain, ScaleRange, ScaleRangeBinding, Theme,
};

use crate::{
    Auto, ConfiguredScaleWithSpec, DomainBounds, DomainExtent, PlotScaleSpec, RadiusPadding, Scale,
    ScaleRuntimeExt, ScaleSpec,
    domain::{ScaleDefaultDomain, ScaleDomain},
    domain_extent::SerializableDomainValue,
    serialization::LogicalExprNodeExt,
};

pub type DefaultScaleRangeResolver<'a> = dyn Fn(
        &str,
        &dyn ScaleImpl,
        &ResolvedDomain,
        &DataType,
        &Theme,
        &IndexMap<String, ScalarValue>,
    ) -> Option<ScaleRange>
    + Send
    + Sync
    + 'a;

/// Stores cached query results for building scales on-demand
///
/// The ScaleBuilder caches expensive DataFusion query results and provides
/// a `build_scales()` method that constructs fresh ConfiguredScale objects
/// with current plot dimensions.
///
/// This pattern prevents two classes of bugs:
/// 1. **Stale scales**: Scales with incorrect ranges when dimensions change
/// 2. **Duplicate queries**: Re-executing expensive queries with identical results
#[derive(Debug, Clone)]
pub struct ScaleBuilder {
    /// Per-channel cached data for scale construction
    pub(crate) channel_scale_data: HashMap<String, ChannelScaleData>,
    /// Data type for each channel (needed for default_channel_range())
    pub(crate) channel_data_types: HashMap<String, DataType>,
}

/// Cached data for a single channel's scale
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum ChannelScaleData {
    /// Standard scale: cache final data extents
    ///
    /// For scales where domain doesn't depend on plot dimensions,
    /// we cache the computed extents (min/max or unique values).
    Standard {
        /// The scale specification (linear, log, ordinal, etc.)
        scale_spec: Box<dyn ScaleSpec>,
        /// Cached data extents from query
        data_extents: DataExtents,
        /// Scale options (e.g., nice, zero, padding)
        options: HashMap<String, LogicalExprNode>,
        /// Runtime raw-domain override expression, if configured.
        raw_domain: Option<LogicalExprNode>,
    },

    /// Radius-aware scale: cache raw data, recompute domain on each build
    ///
    /// For scales where domain padding depends on plot dimensions,
    /// we cache the raw position and radius vectors and recompute
    /// the domain on each `build_scales()` call.
    RadiusAware {
        /// The scale specification (typically linear)
        scale_spec: Box<dyn ScaleSpec>,
        /// Cached position values from query
        position_data: Vec<f64>,
        /// Cached lower radius values from query
        radius_lower_data: Vec<f64>,
        /// Cached upper radius values from query
        radius_upper_data: Vec<f64>,
        /// Scale options (e.g., nice, zero, padding)
        options: HashMap<String, LogicalExprNode>,
        /// Runtime raw-domain override expression, if configured.
        raw_domain: Option<LogicalExprNode>,
    },

    /// Explicit domain scale: no data caching, domain set explicitly
    ///
    /// For scales with explicitly set domains (e.g., `.domain((0.0, 100.0))`),
    /// we don't cache any data - the domain is already known.
    ExplicitDomain {
        /// The scale specification (linear, log, ordinal, etc.)
        scale_spec: Box<dyn ScaleSpec>,
        /// Scale options (e.g., nice, zero, padding)
        options: HashMap<String, LogicalExprNode>,
        /// The explicit domain that was set by the user
        domain: ScaleDomain,
    },
}

/// Cached data extents for standard scales
#[derive(Debug, Clone)]
pub enum DataExtents {
    /// Numeric interval: (min, max)
    Interval(f64, f64),
    /// Categorical: unique values
    Discrete(Vec<ScalarValue>),
    /// Categorical: intentionally ordered unique values
    OrderedDiscrete(Vec<ScalarValue>),
    /// Temporal interval: (min, max) as Unix timestamps
    Temporal(i64, i64),
}

#[derive(Debug, Clone, Copy)]
struct RadiusDomainBoundarySamplesTrace {
    shared_min: f64,
    shared_max: f64,
    max_radius_lower: f64,
    max_radius_upper: f64,
    len_before: usize,
    len_after: usize,
}

fn append_radius_domain_boundary_samples(
    position_data: &mut Vec<f64>,
    radius_lower_data: &mut Vec<f64>,
    radius_upper_data: &mut Vec<f64>,
    shared_extent: &DomainExtent,
) -> Option<RadiusDomainBoundarySamplesTrace> {
    let DomainBounds::Numeric {
        min: shared_min,
        max: shared_max,
    } = &shared_extent.bounds
    else {
        return None;
    };

    let len_before = position_data.len();
    let (max_radius_lower, max_radius_upper) = if let Some(radius) = &shared_extent.radius {
        (radius.max_lower, radius.max_upper)
    } else {
        let lower = radius_lower_data.iter().copied().fold(0.0_f64, f64::max);
        let upper = radius_upper_data.iter().copied().fold(0.0_f64, f64::max);
        (lower, upper)
    };

    // Add boundary samples so the range-dependent radius padding solver sees
    // the coordinated numeric extent without requiring a second data query.
    position_data.push(*shared_min);
    radius_lower_data.push(max_radius_lower);
    radius_upper_data.push(0.0);

    position_data.push(*shared_max);
    radius_lower_data.push(0.0);
    radius_upper_data.push(max_radius_upper);

    Some(RadiusDomainBoundarySamplesTrace {
        shared_min: *shared_min,
        shared_max: *shared_max,
        max_radius_lower,
        max_radius_upper,
        len_before,
        len_after: position_data.len(),
    })
}

fn should_use_cached_domain(scale: &Scale<Auto>) -> bool {
    match &scale.config().domain {
        Maybe::Set(domain) => {
            matches!(domain.default_domain, ScaleDefaultDomain::DomainExprs(_))
                || domain.is_raw_only()
        }
        Maybe::Unset => true,
    }
}

fn current_raw_domain(
    scale: &Scale<Auto>,
    fallback: Option<&LogicalExprNode>,
) -> Option<LogicalExprNode> {
    scale
        .get_domain()
        .and_then(|domain| domain.raw_domain.clone())
        .or_else(|| fallback.cloned())
}

fn attach_raw_domain(mut domain: ScaleDomain, raw_domain: Option<LogicalExprNode>) -> ScaleDomain {
    domain.raw_domain = raw_domain;
    domain
}

impl ScaleBuilder {
    /// Create a new empty scale builder
    pub fn new() -> Self {
        Self {
            channel_scale_data: HashMap::new(),
            channel_data_types: HashMap::new(),
        }
    }

    /// Set the data type for a channel
    pub fn set_channel_data_type(&mut self, channel_name: String, data_type: DataType) {
        self.channel_data_types.insert(channel_name, data_type);
    }

    /// Add a standard scale channel
    pub fn add_standard(
        &mut self,
        channel_name: String,
        scale_spec: Box<dyn ScaleSpec>,
        data_extents: DataExtents,
        options: HashMap<String, LogicalExprNode>,
    ) {
        self.channel_scale_data.insert(
            channel_name,
            ChannelScaleData::Standard {
                scale_spec,
                data_extents,
                options,
                raw_domain: None,
            },
        );
    }

    /// Add a radius-aware scale channel
    pub fn add_radius_aware(
        &mut self,
        channel_name: String,
        scale_spec: Box<dyn ScaleSpec>,
        position_data: Vec<f64>,
        radius_lower_data: Vec<f64>,
        radius_upper_data: Vec<f64>,
        options: HashMap<String, LogicalExprNode>,
    ) {
        self.channel_scale_data.insert(
            channel_name,
            ChannelScaleData::RadiusAware {
                scale_spec,
                position_data,
                radius_lower_data,
                radius_upper_data,
                options,
                raw_domain: None,
            },
        );
    }

    /// Add a scale with explicit domain (no data caching needed)
    pub fn add_explicit_domain(
        &mut self,
        channel_name: String,
        scale_spec: Box<dyn ScaleSpec>,
        options: HashMap<String, LogicalExprNode>,
        domain: ScaleDomain,
    ) {
        self.channel_scale_data.insert(
            channel_name,
            ChannelScaleData::ExplicitDomain {
                scale_spec,
                options,
                domain,
            },
        );
    }

    /// Attach a raw-domain override to a cached scale channel.
    pub fn apply_raw_domain(
        &mut self,
        channel_name: &str,
        raw_domain_override: Option<LogicalExprNode>,
    ) {
        let Some(raw_domain_override) = raw_domain_override else {
            return;
        };

        if let Some(channel_builder) = self.channel_scale_data.get_mut(channel_name) {
            match channel_builder {
                ChannelScaleData::Standard { raw_domain, .. }
                | ChannelScaleData::RadiusAware { raw_domain, .. } => {
                    *raw_domain = Some(raw_domain_override);
                }
                ChannelScaleData::ExplicitDomain { domain, .. } => {
                    domain.raw_domain = Some(raw_domain_override);
                }
            }
        }
    }

    /// Get the channel builders
    pub fn channel_builders(&self) -> &HashMap<String, ChannelScaleData> {
        &self.channel_scale_data
    }

    /// Extract data extents for specified channels as DomainExtent
    ///
    /// This is the unified method for extracting extents from a ScaleBuilder.
    /// It returns a HashMap of channel name to DomainExtent, preserving radius
    /// information when present. This is used for Level(N) and Shared domain
    /// computation to ensure consistent domain representation.
    ///
    /// # Arguments
    /// * `channels` - List of channel names to extract extents for
    ///
    /// # Returns
    /// HashMap mapping channel names to their DomainExtent values
    pub fn extract_domain_extents(&self, channels: &[&str]) -> HashMap<String, DomainExtent> {
        let mut result = HashMap::new();

        for channel in channels {
            if let Some(builder) = self.channel_scale_data.get(*channel) {
                match builder {
                    ChannelScaleData::Standard { data_extents, .. } => {
                        let extent = match data_extents {
                            DataExtents::Interval(min, max) => DomainExtent::numeric(*min, *max),
                            DataExtents::Temporal(min, max) => DomainExtent::temporal(*min, *max),
                            DataExtents::Discrete(values)
                            | DataExtents::OrderedDiscrete(values) => {
                                let serializable_values: Vec<SerializableDomainValue> = values
                                    .iter()
                                    .map(SerializableDomainValue::from_scalar)
                                    .collect();
                                if matches!(data_extents, DataExtents::OrderedDiscrete(_)) {
                                    DomainExtent::ordered_discrete(serializable_values)
                                } else {
                                    DomainExtent::discrete(serializable_values)
                                }
                            }
                        };
                        result.insert(channel.to_string(), extent);
                    }
                    ChannelScaleData::RadiusAware {
                        position_data,
                        radius_lower_data,
                        radius_upper_data,
                        ..
                    } => {
                        // For radius-aware scales, compute min/max from position data
                        // and include max radius values for proper domain expansion
                        if !position_data.is_empty() {
                            let min = position_data.iter().cloned().fold(f64::INFINITY, f64::min);
                            let max = position_data
                                .iter()
                                .cloned()
                                .fold(f64::NEG_INFINITY, f64::max);
                            let max_radius_lower =
                                radius_lower_data.iter().cloned().fold(0.0_f64, f64::max);
                            let max_radius_upper =
                                radius_upper_data.iter().cloned().fold(0.0_f64, f64::max);
                            result.insert(
                                channel.to_string(),
                                DomainExtent {
                                    bounds: DomainBounds::Numeric { min, max },
                                    radius: Some(RadiusPadding {
                                        max_lower: max_radius_lower,
                                        max_upper: max_radius_upper,
                                    }),
                                    ordered_discrete: false,
                                },
                            );
                        }
                    }
                    ChannelScaleData::ExplicitDomain { .. } => {
                        // Explicit domain doesn't have extractable extents
                    }
                }
            }
        }

        result
    }

    /// Extend data extents using DomainExtent values
    ///
    /// This is the unified method for extending scale domains with shared extents.
    /// It properly handles the `DomainExtent.radius` field to ensure radius-aware
    /// domain padding is preserved through the pipeline.
    ///
    /// # Arguments
    /// * `shared_extents` - HashMap mapping channel names to their shared DomainExtent values
    pub fn extend_with_domain_extents(&mut self, shared_extents: &HashMap<String, DomainExtent>) {
        debug!(
            shared_channel_count = shared_extents.len(),
            available_channels = ?self.channel_scale_data.keys().collect::<Vec<_>>(),
            "extend_with_domain_extents"
        );
        for (ch, ext) in shared_extents {
            trace!(channel = ch, extent = ?ext, "shared extent");
        }

        for (channel, shared_extent) in shared_extents {
            if let Some(channel_builder) = self.channel_scale_data.get_mut(channel) {
                let variant = match channel_builder {
                    ChannelScaleData::Standard { data_extents, .. } => {
                        format!("Standard({:?})", data_extents)
                    }
                    ChannelScaleData::RadiusAware { .. } => "RadiusAware".to_string(),
                    ChannelScaleData::ExplicitDomain { .. } => "ExplicitDomain".to_string(),
                };
                trace!(channel, builder = %variant, "processing shared extent");

                match channel_builder {
                    ChannelScaleData::Standard { data_extents, .. } => {
                        // Extend the local extents with shared extents
                        match (&mut *data_extents, &shared_extent.bounds) {
                            (
                                DataExtents::Interval(local_min, local_max),
                                DomainBounds::Numeric {
                                    min: shared_min,
                                    max: shared_max,
                                },
                            ) => {
                                trace!(
                                    channel,
                                    local_min = *local_min,
                                    local_max = *local_max,
                                    shared_min = *shared_min,
                                    shared_max = *shared_max,
                                    "Extending numeric extents"
                                );
                                // Take union: min of mins, max of maxes
                                *local_min = local_min.min(*shared_min);
                                *local_max = local_max.max(*shared_max);
                                trace!(
                                    channel,
                                    new_min = *local_min,
                                    new_max = *local_max,
                                    "Updated numeric extents"
                                );
                            }
                            (
                                DataExtents::Temporal(local_min, local_max),
                                DomainBounds::Temporal {
                                    min: shared_min,
                                    max: shared_max,
                                },
                            ) => {
                                *local_min = (*local_min).min(*shared_min);
                                *local_max = (*local_max).max(*shared_max);
                            }
                            (
                                DataExtents::Discrete(local_values)
                                | DataExtents::OrderedDiscrete(local_values),
                                DomainBounds::Discrete(shared_values),
                            ) => {
                                // For categorical plot scale sharing, replace local values with shared values.
                                trace!(
                                    channel,
                                    local_values = local_values.len(),
                                    shared_values = shared_values.len(),
                                    "Replacing discrete extents with shared values"
                                );
                                let replacement_values =
                                    shared_values.iter().map(|v| v.to_scalar()).collect();
                                *data_extents = if shared_extent.ordered_discrete {
                                    DataExtents::OrderedDiscrete(replacement_values)
                                } else {
                                    DataExtents::Discrete(replacement_values)
                                };
                            }
                            _ => {
                                trace!(channel, "No matching extent variant for shared extent");
                            }
                        }
                    }
                    ChannelScaleData::RadiusAware {
                        position_data,
                        radius_lower_data,
                        radius_upper_data,
                        ..
                    } => {
                        if let Some(trace_data) = append_radius_domain_boundary_samples(
                            position_data,
                            radius_lower_data,
                            radius_upper_data,
                            shared_extent,
                        ) {
                            trace!(
                                channel,
                                shared_min = trace_data.shared_min,
                                shared_max = trace_data.shared_max,
                                max_radius_lower = trace_data.max_radius_lower,
                                max_radius_upper = trace_data.max_radius_upper,
                                len_before = trace_data.len_before,
                                len_after = trace_data.len_after,
                                "Extended RadiusAware extents with shared range"
                            );
                        }
                    }
                    ChannelScaleData::ExplicitDomain { .. } => {
                        // Explicit domains should not be modified by shared extents
                    }
                }
            }
        }
    }

    fn apply_default_range_if_needed(
        &self,
        mut scale: Scale<Auto>,
        channel_name: &str,
        default_range_resolver: &DefaultScaleRangeResolver<'_>,
        theme: &Theme,
        params: &IndexMap<String, ScalarValue>,
    ) -> Scale<Auto> {
        if scale.get_range().is_some() {
            return scale;
        }

        if let Some(data_type) = self.channel_data_types.get(channel_name)
            && let (Some(scale_impl), Some(domain)) = (scale.get_scale_impl(), scale.get_domain())
            && let Ok(resolved_domain) = domain.to_resolved()
            && let Some(mark_range) = default_range_resolver(
                channel_name,
                scale_impl.as_ref(),
                &resolved_domain,
                data_type,
                theme,
                params,
            )
        {
            scale = scale.range(mark_range);
        }

        if scale.get_range().is_none() {
            let range_kind = scale
                .get_scale_impl()
                .map(|impl_arc| impl_arc.range_kind())
                .unwrap_or(RangeKind::Continuous);

            let range = theme
                .get_range_for_channel("mark", channel_name, range_kind, None, params)
                .unwrap_or_else(|| crate::default_range_for_channel(channel_name, range_kind));

            scale = scale.range(range);
        }

        scale
    }

    /// Build scales on-demand with current dimensions and padding
    ///
    /// This is the core method that constructs ConfiguredScale objects
    /// using cached data. For standard scales, it uses cached extents directly.
    /// For radius-aware scales, it recomputes domain padding with the new range.
    ///
    /// # Arguments
    /// * `width` - Plot area width
    /// * `height` - Plot area height
    /// * `coord_system_range_bindings` - Map of coordinate channel name to range binding metadata
    /// * `scale_specs` - Scale specifications for overrides
    /// * `default_range_resolver` - Callback for mark-specific default ranges
    /// * `theme` - Theme for default ranges
    /// * `ctx` - DataFusion session context
    /// * `params` - Parameters for expression evaluation
    ///
    /// # Returns
    /// HashMap of channel name to ConfiguredScaleWithSpec
    #[allow(clippy::too_many_arguments)]
    pub async fn build_scales(
        &self,
        width: f32,
        height: f32,
        coord_system_range_bindings: &HashMap<String, ScaleRangeBinding>,
        scale_specs: &HashMap<String, PlotScaleSpec>,
        default_range_resolver: &DefaultScaleRangeResolver<'_>,
        theme: &Theme,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<HashMap<String, ConfiguredScaleWithSpec>, AvengerChartError> {
        let mut result = HashMap::new();

        // Iterate channel builders in a deterministic order
        let mut builder_entries: Vec<(&String, &ChannelScaleData)> =
            self.channel_scale_data.iter().collect();
        builder_entries.sort_by(|a, b| a.0.cmp(b.0));

        for (channel_name, channel_builder) in builder_entries.into_iter() {
            if channel_name == "y"
                && let ChannelScaleData::RadiusAware { position_data, .. } = channel_builder
            {
                trace!(
                    channel = channel_name,
                    len = position_data.len(),
                    "build_scales starting RadiusAware channel"
                );
            }
            match channel_builder {
                ChannelScaleData::Standard {
                    scale_spec,
                    data_extents,
                    options,
                    raw_domain,
                } => {
                    // For standard scales: use cached extents directly (no query!)
                    let mut scale = Scale::<Auto>::from_spec(scale_spec.as_ref().clone_box());

                    // Apply cached options
                    for (key, value_node) in options {
                        let expr = value_node.to_expr(ctx)?;
                        scale = scale.option(key, expr);
                    }

                    // Check if there's an explicit scale spec that overrides the domain
                    // If so, we should NOT use the cached data_extents
                    let use_cached_domain = if let Some(spec) = scale_specs.get(channel_name) {
                        // Apply the spec to see if it has an explicit domain
                        match spec {
                            PlotScaleSpec::Local(scale_changes) => {
                                scale = scale.update(Scale::from_config(scale_changes.clone()));
                            }
                        }

                        // Check if domain is still DomainExprs (needs data) or explicit
                        should_use_cached_domain(&scale)
                    } else {
                        true // No override, use cached data
                    };

                    // Set domain from cached extents only if needed
                    if use_cached_domain {
                        let domain = attach_raw_domain(
                            data_extents.to_scale_domain()?,
                            current_raw_domain(&scale, raw_domain.as_ref()),
                        );
                        scale = scale.domain(domain);
                    }

                    let range_binding = coord_system_range_bindings
                        .get(channel_name)
                        .copied()
                        .unwrap_or(ScaleRangeBinding::Independent);

                    // Set coordinate-owned range if available
                    if let Some((range_min, range_max)) =
                        range_binding.resolve(width as f64, height as f64)
                    {
                        scale = scale.range_interval(lit(range_min), lit(range_max));
                    }

                    // Normalize domain (apply zero, nice, padding)
                    scale = Box::pin(scale.normalize_domain(width, height, ctx, params)).await?;

                    scale = self.apply_default_range_if_needed(
                        scale,
                        channel_name,
                        default_range_resolver,
                        theme,
                        params,
                    );

                    // Create configured scale
                    let configured = Box::pin(
                        scale
                            .clone()
                            .create_configured_scale(width, height, ctx, params),
                    )
                    .await?;

                    result.insert(
                        channel_name.clone(),
                        ConfiguredScaleWithSpec::with_range_binding(
                            scale,
                            configured,
                            range_binding,
                        ),
                    );
                }
                ChannelScaleData::RadiusAware {
                    scale_spec,
                    position_data,
                    radius_lower_data,
                    radius_upper_data,
                    options,
                    raw_domain,
                } => {
                    // For radius-aware scales: recompute domain with new range (cheap math, no query!)
                    let mut scale = Scale::<Auto>::from_spec(scale_spec.as_ref().clone_box());

                    // Apply cached options
                    for (key, value_node) in options {
                        let expr = value_node.to_expr(ctx)?;
                        scale = scale.option(key, expr);
                    }

                    let range_binding = coord_system_range_bindings
                        .get(channel_name)
                        .copied()
                        .unwrap_or(ScaleRangeBinding::Independent);

                    // Get coordinate-owned range for this channel
                    let (range_min, range_max) = range_binding
                        .resolve(width as f64, height as f64)
                        .ok_or_else(|| {
                            AvengerChartError::InternalError(format!(
                                "No range found for radius-aware channel '{}'",
                                channel_name
                            ))
                        })?;

                    let range_width = (range_max - range_min).abs();

                    // Recompute domain with new range_width using cached data
                    let (d_min, d_max) = compute_domain_from_data_with_padding_linear(
                        position_data,
                        radius_lower_data,
                        radius_upper_data,
                        range_width,
                    )?;

                    if channel_name == "y" {
                        let data_preview: Vec<f64> =
                            position_data.iter().take(12).cloned().collect();
                        trace!(
                            channel = channel_name,
                            d_min,
                            d_max,
                            position_len = position_data.len(),
                            range_width,
                            data_preview = ?data_preview,
                            "RadiusAware domain after compute_domain_from_data"
                        );
                    }

                    // Set domain and range
                    let domain = attach_raw_domain(
                        ScaleDomain::new_interval(lit(d_min), lit(d_max)),
                        raw_domain.clone(),
                    );
                    scale = scale.domain(domain);
                    scale = scale.range_interval(lit(range_min), lit(range_max));

                    // Normalize domain (apply zero, nice, padding)
                    scale = Box::pin(scale.normalize_domain(width, height, ctx, params)).await?;

                    if channel_name == "y"
                        && let Ok((norm_min, norm_max)) = Box::pin(
                            scale
                                .clone()
                                .create_configured_scale(width, height, ctx, params),
                        )
                        .await
                        .and_then(|c| Ok(c.numeric_interval_domain()?))
                    {
                        trace!(
                            channel = channel_name,
                            norm_min, norm_max, "RadiusAware domain after normalize"
                        );
                    }

                    scale = self.apply_default_range_if_needed(
                        scale,
                        channel_name,
                        default_range_resolver,
                        theme,
                        params,
                    );

                    // Create configured scale
                    let configured = Box::pin(
                        scale
                            .clone()
                            .create_configured_scale(width, height, ctx, params),
                    )
                    .await?;

                    result.insert(
                        channel_name.clone(),
                        ConfiguredScaleWithSpec::with_range_binding(
                            scale,
                            configured,
                            range_binding,
                        ),
                    );
                }
                ChannelScaleData::ExplicitDomain {
                    scale_spec,
                    options,
                    domain,
                } => {
                    // For explicit domain scales: build scale from spec without cached data
                    let mut scale = Scale::<Auto>::from_spec(scale_spec.as_ref().clone_box());

                    // Apply the stored explicit domain FIRST
                    scale = scale.domain(domain.clone());

                    // Apply cached options
                    for (key, value_node) in options {
                        let expr = value_node.to_expr(ctx)?;
                        scale = scale.option(key, expr);
                    }

                    // Apply scale spec overrides (which might override the explicit domain)
                    if let Some(spec) = scale_specs.get(channel_name) {
                        match spec {
                            PlotScaleSpec::Local(scale_changes) => {
                                scale = scale.update(Scale::from_config(scale_changes.clone()));
                            }
                        }
                    }

                    let range_binding = coord_system_range_bindings
                        .get(channel_name)
                        .copied()
                        .unwrap_or(ScaleRangeBinding::Independent);

                    // Set coordinate-owned range if available
                    if let Some((range_min, range_max)) =
                        range_binding.resolve(width as f64, height as f64)
                    {
                        scale = scale.range_interval(lit(range_min), lit(range_max));
                    }

                    // Normalize domain (apply zero, nice, padding)
                    scale = Box::pin(scale.normalize_domain(width, height, ctx, params)).await?;

                    scale = self.apply_default_range_if_needed(
                        scale,
                        channel_name,
                        default_range_resolver,
                        theme,
                        params,
                    );

                    // Create configured scale
                    let configured = Box::pin(
                        scale
                            .clone()
                            .create_configured_scale(width, height, ctx, params),
                    )
                    .await?;

                    result.insert(
                        channel_name.clone(),
                        ConfiguredScaleWithSpec::with_range_binding(
                            scale,
                            configured,
                            range_binding,
                        ),
                    );
                }
            }
        }

        Ok(result)
    }
}

impl Default for ScaleBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl DataExtents {
    /// Convert data extents to a scale domain
    pub fn to_scale_domain(&self) -> Result<ScaleDomain, AvengerChartError> {
        match self {
            DataExtents::Interval(min, max) => Ok(ScaleDomain::new_interval(lit(*min), lit(*max))),
            DataExtents::Discrete(values) | DataExtents::OrderedDiscrete(values) => {
                let exprs: Vec<_> = values.iter().map(|v| lit(v.clone())).collect();
                Ok(ScaleDomain::new_discrete(exprs))
            }
            DataExtents::Temporal(min, max) => Ok(ScaleDomain::new_interval(lit(*min), lit(*max))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Linear;
    use datafusion::functions_array::expr_fn::make_array;

    fn no_default_range(
        _channel: &str,
        _scale_impl: &dyn ScaleImpl,
        _domain: &ResolvedDomain,
        _data_type: &DataType,
        _theme: &Theme,
        _params: &IndexMap<String, ScalarValue>,
    ) -> Option<ScaleRange> {
        None
    }

    #[test]
    fn test_scale_builder_new() {
        let builder = ScaleBuilder::new();
        assert!(builder.channel_scale_data.is_empty());
    }

    #[test]
    fn test_add_standard_scale() {
        let mut builder = ScaleBuilder::new();
        let scale_spec = Box::new(Linear) as Box<dyn ScaleSpec>;
        let data_extents = DataExtents::Interval(0.0, 100.0);
        let options = HashMap::new();

        builder.add_standard("x".to_string(), scale_spec, data_extents, options);

        assert_eq!(builder.channel_scale_data.len(), 1);
        assert!(builder.channel_scale_data.contains_key("x"));
    }

    #[test]
    fn test_add_radius_aware_scale() {
        let mut builder = ScaleBuilder::new();
        let scale_spec = Box::new(Linear) as Box<dyn ScaleSpec>;
        let position_data = vec![0.0, 1.0, 2.0];
        let radius_lower = vec![0.5, 0.5, 0.5];
        let radius_upper = vec![0.5, 0.5, 0.5];
        let options = HashMap::new();

        builder.add_radius_aware(
            "x".to_string(),
            scale_spec,
            position_data,
            radius_lower,
            radius_upper,
            options,
        );

        assert_eq!(builder.channel_scale_data.len(), 1);
        assert!(builder.channel_scale_data.contains_key("x"));
    }

    #[test]
    fn test_data_extents_to_domain_interval() {
        let extents = DataExtents::Interval(0.0, 100.0);
        let domain = extents.to_scale_domain().unwrap();

        // Verify it created an interval domain
        match domain.default_domain {
            ScaleDefaultDomain::Interval(_, _) => {
                // Success
            }
            _ => panic!("Expected interval domain"),
        }
    }

    #[test]
    fn test_data_extents_to_domain_discrete() {
        let extents = DataExtents::Discrete(vec![
            ScalarValue::Utf8(Some("a".to_string())),
            ScalarValue::Utf8(Some("b".to_string())),
        ]);
        let domain = extents.to_scale_domain().unwrap();

        // Verify it created a discrete domain
        match domain.default_domain {
            ScaleDefaultDomain::Discrete(values) => {
                assert_eq!(values.len(), 2);
            }
            _ => panic!("Expected discrete domain"),
        }
    }

    #[test]
    fn test_data_extents_to_domain_temporal() {
        let extents = DataExtents::Temporal(0, 1000000);
        let domain = extents.to_scale_domain().unwrap();

        // Verify it created an interval domain
        match domain.default_domain {
            ScaleDefaultDomain::Interval(_, _) => {
                // Success
            }
            _ => panic!("Expected interval domain"),
        }
    }

    #[tokio::test]
    async fn test_build_scales_standard() {
        let mut builder = ScaleBuilder::new();
        let scale_spec = Box::new(Linear) as Box<dyn ScaleSpec>;
        let data_extents = DataExtents::Interval(0.0, 100.0);
        let options = HashMap::new();

        builder.add_standard("x".to_string(), scale_spec, data_extents, options);

        let mut coord_ranges = HashMap::new();
        coord_ranges.insert(
            "x".to_string(),
            ScaleRangeBinding::fixed_interval(0.0, 400.0),
        );

        let ctx = SessionContext::new();
        let params = IndexMap::new();

        let scales = builder
            .build_scales(
                400.0,
                300.0,
                &coord_ranges,
                &HashMap::new(),
                &no_default_range,
                &Theme::light(),
                &ctx,
                &params,
            )
            .await
            .unwrap();

        assert_eq!(scales.len(), 1);
        assert!(scales.contains_key("x"));

        // Verify the scale has the correct domain
        let x_scale = scales.get("x").unwrap();
        let (d_min, d_max) = x_scale.configured().numeric_interval_domain().unwrap();
        assert!(d_min <= 0.0);
        assert!(d_max >= 100.0);
    }

    #[tokio::test]
    async fn test_build_scales_standard_with_raw_domain_override() {
        let mut builder = ScaleBuilder::new();
        let scale_spec = Box::new(Linear) as Box<dyn ScaleSpec>;
        builder.add_standard(
            "x".to_string(),
            scale_spec,
            DataExtents::Interval(0.0, 100.0),
            HashMap::new(),
        );

        let mut coord_ranges = HashMap::new();
        coord_ranges.insert(
            "x".to_string(),
            ScaleRangeBinding::fixed_interval(0.0, 400.0),
        );

        let mut scale_specs = HashMap::new();
        let scale_config = Scale::<Auto>::from_spec(Box::new(Linear))
            .raw_domain(make_array(vec![lit(20.0), lit(40.0)]))
            .into_config();
        scale_specs.insert("x".to_string(), PlotScaleSpec::Local(scale_config));

        let ctx = SessionContext::new();
        let scales = builder
            .build_scales(
                400.0,
                300.0,
                &coord_ranges,
                &scale_specs,
                &no_default_range,
                &Theme::light(),
                &ctx,
                &IndexMap::new(),
            )
            .await
            .unwrap();

        let x_scale = scales.get("x").unwrap();
        let (d_min, d_max) = x_scale.configured().numeric_interval_domain().unwrap();
        assert_eq!((d_min, d_max), (20.0, 40.0));
    }

    #[tokio::test]
    async fn test_build_scales_raw_domain_only_override_preserves_explicit_fallback_domain() {
        let mut builder = ScaleBuilder::new();
        builder.add_explicit_domain(
            "x".to_string(),
            Box::new(Linear),
            HashMap::new(),
            ScaleDomain::new_interval(lit(0.0), lit(100.0)),
        );

        let mut coord_ranges = HashMap::new();
        coord_ranges.insert(
            "x".to_string(),
            ScaleRangeBinding::fixed_interval(0.0, 400.0),
        );

        let mut scale_specs = HashMap::new();
        let scale_config = Scale::<Auto>::from_spec(Box::new(Linear))
            .raw_domain(make_array(vec![lit(20.0), lit(40.0)]))
            .into_config();
        scale_specs.insert("x".to_string(), PlotScaleSpec::Local(scale_config));

        let ctx = SessionContext::new();
        let scales = builder
            .build_scales(
                400.0,
                300.0,
                &coord_ranges,
                &scale_specs,
                &no_default_range,
                &Theme::light(),
                &ctx,
                &IndexMap::new(),
            )
            .await
            .unwrap();

        let x_scale = scales.get("x").unwrap();
        let (d_min, d_max) = x_scale.configured().numeric_interval_domain().unwrap();
        assert_eq!((d_min, d_max), (20.0, 40.0));
    }

    #[tokio::test]
    async fn test_build_scales_radius_aware() {
        let mut builder = ScaleBuilder::new();
        let scale_spec = Box::new(Linear) as Box<dyn ScaleSpec>;
        let position_data = vec![0.0, 50.0, 100.0];
        let radius_lower = vec![5.0, 5.0, 5.0];
        let radius_upper = vec![5.0, 5.0, 5.0];
        let options = HashMap::new();

        builder.add_radius_aware(
            "x".to_string(),
            scale_spec,
            position_data,
            radius_lower,
            radius_upper,
            options,
        );

        let mut coord_ranges = HashMap::new();
        coord_ranges.insert(
            "x".to_string(),
            ScaleRangeBinding::fixed_interval(0.0, 400.0),
        );

        let ctx = SessionContext::new();
        let params = IndexMap::new();

        let scales = builder
            .build_scales(
                400.0,
                300.0,
                &coord_ranges,
                &HashMap::new(),
                &no_default_range,
                &Theme::light(),
                &ctx,
                &params,
            )
            .await
            .unwrap();

        assert_eq!(scales.len(), 1);
        assert!(scales.contains_key("x"));

        // Verify the scale has the correct domain (should be expanded for radius)
        let x_scale = scales.get("x").unwrap();
        let (d_min, d_max) = x_scale.configured().numeric_interval_domain().unwrap();

        // Domain should be expanded beyond data range to accommodate radius
        assert!(d_min < 0.0, "d_min ({}) should be less than 0.0", d_min);
        assert!(
            d_max > 100.0,
            "d_max ({}) should be greater than 100.0",
            d_max
        );
    }

    #[tokio::test]
    async fn test_build_scales_radius_aware_range_change() {
        let mut builder = ScaleBuilder::new();
        let scale_spec = Box::new(Linear) as Box<dyn ScaleSpec>;
        let position_data = vec![0.0, 50.0, 100.0];
        let radius_lower = vec![5.0, 5.0, 5.0];
        let radius_upper = vec![5.0, 5.0, 5.0];
        let options = HashMap::new();

        builder.add_radius_aware(
            "x".to_string(),
            scale_spec,
            position_data,
            radius_lower,
            radius_upper,
            options,
        );

        let ctx = SessionContext::new();
        let params = IndexMap::new();

        // Build with first range
        let mut coord_ranges1 = HashMap::new();
        coord_ranges1.insert(
            "x".to_string(),
            ScaleRangeBinding::fixed_interval(0.0, 400.0),
        );

        let scales1 = builder
            .build_scales(
                400.0,
                300.0,
                &coord_ranges1,
                &HashMap::new(),
                &no_default_range,
                &Theme::light(),
                &ctx,
                &params,
            )
            .await
            .unwrap();

        let (d_min1, d_max1) = scales1
            .get("x")
            .unwrap()
            .configured()
            .numeric_interval_domain()
            .unwrap();

        // Build with second range (wider)
        let mut coord_ranges2 = HashMap::new();
        coord_ranges2.insert(
            "x".to_string(),
            ScaleRangeBinding::fixed_interval(0.0, 800.0),
        );

        let scales2 = builder
            .build_scales(
                800.0,
                300.0,
                &coord_ranges2,
                &HashMap::new(),
                &no_default_range,
                &Theme::light(),
                &ctx,
                &params,
            )
            .await
            .unwrap();

        let (d_min2, d_max2) = scales2
            .get("x")
            .unwrap()
            .configured()
            .numeric_interval_domain()
            .unwrap();

        // With a wider range, the domain padding should be smaller
        // (same pixel radius covers less data space)
        let padding1 = d_max1 - d_min1 - 100.0;
        let padding2 = d_max2 - d_min2 - 100.0;

        assert!(
            padding2 < padding1,
            "Wider range should result in smaller data-space padding: {} vs {}",
            padding2,
            padding1
        );
    }

    #[test]
    fn test_extract_domain_extents_standard() {
        let mut builder = ScaleBuilder::new();
        let scale_spec = Box::new(Linear) as Box<dyn ScaleSpec>;
        let data_extents = DataExtents::Interval(0.0, 100.0);
        let options = HashMap::new();

        builder.add_standard("x".to_string(), scale_spec, data_extents, options);

        let extents = builder.extract_domain_extents(&["x"]);

        assert_eq!(extents.len(), 1);
        let x_extent = extents.get("x").unwrap();
        assert_eq!(x_extent.numeric_bounds(), Some((0.0, 100.0)));
        assert!(!x_extent.has_radius());
    }

    #[test]
    fn test_extract_domain_extents_radius_aware() {
        let mut builder = ScaleBuilder::new();
        let scale_spec = Box::new(Linear) as Box<dyn ScaleSpec>;
        let position_data = vec![0.0, 50.0, 100.0];
        let radius_lower = vec![5.0, 3.0, 5.0];
        let radius_upper = vec![7.0, 7.0, 4.0];
        let options = HashMap::new();

        builder.add_radius_aware(
            "y".to_string(),
            scale_spec,
            position_data,
            radius_lower,
            radius_upper,
            options,
        );

        let extents = builder.extract_domain_extents(&["y"]);

        assert_eq!(extents.len(), 1);
        let y_extent = extents.get("y").unwrap();
        assert_eq!(y_extent.numeric_bounds(), Some((0.0, 100.0)));
        assert!(y_extent.has_radius());

        // Check radius values are the max of the arrays
        let radius = y_extent.radius.as_ref().unwrap();
        assert_eq!(radius.max_lower, 5.0);
        assert_eq!(radius.max_upper, 7.0);
    }

    #[test]
    fn test_extract_domain_extents_mixed() {
        let mut builder = ScaleBuilder::new();

        // Add standard scale
        let scale_spec1 = Box::new(Linear) as Box<dyn ScaleSpec>;
        builder.add_standard(
            "x".to_string(),
            scale_spec1,
            DataExtents::Interval(0.0, 100.0),
            HashMap::new(),
        );

        // Add radius-aware scale
        let scale_spec2 = Box::new(Linear) as Box<dyn ScaleSpec>;
        builder.add_radius_aware(
            "y".to_string(),
            scale_spec2,
            vec![10.0, 20.0],
            vec![2.0, 3.0],
            vec![4.0, 5.0],
            HashMap::new(),
        );

        // Add discrete scale
        let scale_spec3 = Box::new(Linear) as Box<dyn ScaleSpec>;
        builder.add_standard(
            "color".to_string(),
            scale_spec3,
            DataExtents::Discrete(vec![
                ScalarValue::Utf8(Some("red".to_string())),
                ScalarValue::Utf8(Some("blue".to_string())),
            ]),
            HashMap::new(),
        );

        let extents = builder.extract_domain_extents(&["x", "y", "color", "missing"]);

        // Should have 3 channels (missing is ignored)
        assert_eq!(extents.len(), 3);

        // x: numeric without radius
        let x = extents.get("x").unwrap();
        assert_eq!(x.numeric_bounds(), Some((0.0, 100.0)));
        assert!(!x.has_radius());

        // y: numeric with radius
        let y = extents.get("y").unwrap();
        assert_eq!(y.numeric_bounds(), Some((10.0, 20.0)));
        assert!(y.has_radius());

        // color: discrete
        let color = extents.get("color").unwrap();
        assert!(color.discrete_values().is_some());
        assert_eq!(color.discrete_values().unwrap().len(), 2);
    }

    #[test]
    fn test_extend_with_domain_extents_standard_numeric() {
        let mut builder = ScaleBuilder::new();
        let scale_spec = Box::new(Linear) as Box<dyn ScaleSpec>;
        builder.add_standard(
            "x".to_string(),
            scale_spec,
            DataExtents::Interval(10.0, 50.0),
            HashMap::new(),
        );

        // Create shared extent with wider range
        let mut shared = HashMap::new();
        shared.insert("x".to_string(), DomainExtent::numeric(0.0, 100.0));

        builder.extend_with_domain_extents(&shared);

        // Verify extents were extended
        let extents = builder.extract_domain_extents(&["x"]);
        let x = extents.get("x").unwrap();
        assert_eq!(x.numeric_bounds(), Some((0.0, 100.0))); // Union of (10,50) and (0,100)
    }

    #[test]
    fn test_extend_with_domain_extents_radius_aware_with_radius() {
        let mut builder = ScaleBuilder::new();
        let scale_spec = Box::new(Linear) as Box<dyn ScaleSpec>;
        builder.add_radius_aware(
            "y".to_string(),
            scale_spec,
            vec![10.0, 20.0],
            vec![2.0, 3.0],
            vec![4.0, 5.0],
            HashMap::new(),
        );

        // Create shared extent WITH radius info
        let mut shared = HashMap::new();
        shared.insert(
            "y".to_string(),
            DomainExtent::numeric_with_radius(0.0, 100.0, 10.0, 15.0),
        );

        builder.extend_with_domain_extents(&shared);

        // Verify extents include boundary samples with shared radius values.
        let extents = builder.extract_domain_extents(&["y"]);
        let y = extents.get("y").unwrap();
        assert_eq!(y.numeric_bounds(), Some((0.0, 100.0)));
        assert!(y.has_radius());

        // The max radius should be the max of local (3, 5) and shared (10, 15)
        let radius = y.radius.as_ref().unwrap();
        assert_eq!(radius.max_lower, 10.0); // Shared 10 > local 3
        assert_eq!(radius.max_upper, 15.0); // Shared 15 > local 5
    }

    #[test]
    fn test_extend_with_domain_extents_radius_aware_without_radius() {
        let mut builder = ScaleBuilder::new();
        let scale_spec = Box::new(Linear) as Box<dyn ScaleSpec>;
        builder.add_radius_aware(
            "y".to_string(),
            scale_spec,
            vec![10.0, 20.0],
            vec![2.0, 3.0],
            vec![4.0, 5.0],
            HashMap::new(),
        );

        // Create shared extent WITHOUT radius info
        let mut shared = HashMap::new();
        shared.insert("y".to_string(), DomainExtent::numeric(0.0, 100.0));

        builder.extend_with_domain_extents(&shared);

        // Verify extents include boundary samples.
        let extents = builder.extract_domain_extents(&["y"]);
        let y = extents.get("y").unwrap();
        assert_eq!(y.numeric_bounds(), Some((0.0, 100.0)));
        assert!(y.has_radius());

        // When shared extent has no radius, local radius values should be used
        let radius = y.radius.as_ref().unwrap();
        // Local max values: lower=3.0, upper=5.0
        // These should be preserved since shared has no radius info
        assert!(radius.max_lower >= 3.0);
        assert!(radius.max_upper >= 5.0);
    }

    #[test]
    fn test_extend_with_domain_extents_discrete() {
        let mut builder = ScaleBuilder::new();
        let scale_spec = Box::new(Linear) as Box<dyn ScaleSpec>;
        builder.add_standard(
            "color".to_string(),
            scale_spec,
            DataExtents::Discrete(vec![ScalarValue::Utf8(Some("red".to_string()))]),
            HashMap::new(),
        );

        // Create shared discrete extent with more values
        let mut shared = HashMap::new();
        shared.insert(
            "color".to_string(),
            DomainExtent::discrete(vec![
                SerializableDomainValue::String("red".to_string()),
                SerializableDomainValue::String("green".to_string()),
                SerializableDomainValue::String("blue".to_string()),
            ]),
        );

        builder.extend_with_domain_extents(&shared);

        // Verify discrete values were replaced
        let extents = builder.extract_domain_extents(&["color"]);
        let color = extents.get("color").unwrap();
        let values = color.discrete_values().unwrap();
        assert_eq!(values.len(), 3);
    }
}
