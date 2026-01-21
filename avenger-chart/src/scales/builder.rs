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

use crate::error::AvengerChartError;
use datafusion_common::ScalarValue;
use std::collections::HashMap;

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
    pub(crate) channel_builders: HashMap<String, ChannelScaleBuilder>,
    /// Data type for each channel (needed for default_channel_range())
    pub(crate) channel_data_types: HashMap<String, datafusion::arrow::datatypes::DataType>,
}

/// Cached data for a single channel's scale
#[derive(Debug, Clone)]
pub enum ChannelScaleBuilder {
    /// Standard scale: cache final data extents
    ///
    /// For scales where domain doesn't depend on plot dimensions,
    /// we cache the computed extents (min/max or unique values).
    Standard {
        /// The scale specification (linear, log, ordinal, etc.)
        scale_spec: Box<dyn crate::scales::ScaleSpec>,
        /// Cached data extents from query
        data_extents: DataExtents,
        /// Scale options (e.g., nice, zero, padding)
        options: HashMap<String, datafusion_proto::protobuf::LogicalExprNode>,
    },

    /// Radius-aware scale: cache raw data, recompute domain on each build
    ///
    /// For scales where domain padding depends on plot dimensions,
    /// we cache the raw position and radius vectors and recompute
    /// the domain on each `build_scales()` call.
    RadiusAware {
        /// The scale specification (typically linear)
        scale_spec: Box<dyn crate::scales::ScaleSpec>,
        /// Cached position values from query
        position_data: Vec<f64>,
        /// Cached lower radius values from query
        radius_lower_data: Vec<f64>,
        /// Cached upper radius values from query
        radius_upper_data: Vec<f64>,
        /// Scale options (e.g., nice, zero, padding)
        options: HashMap<String, datafusion_proto::protobuf::LogicalExprNode>,
    },

    /// Explicit domain scale: no data caching, domain set explicitly
    ///
    /// For scales with explicitly set domains (e.g., `.domain((0.0, 100.0))`),
    /// we don't cache any data - the domain is already known.
    ExplicitDomain {
        /// The scale specification (linear, log, ordinal, etc.)
        scale_spec: Box<dyn crate::scales::ScaleSpec>,
        /// Scale options (e.g., nice, zero, padding)
        options: HashMap<String, datafusion_proto::protobuf::LogicalExprNode>,
        /// The explicit domain that was set by the user
        domain: crate::scales::ScaleDomain,
    },
}

/// Cached data extents for standard scales
#[derive(Debug, Clone)]
pub enum DataExtents {
    /// Numeric interval: (min, max)
    Interval(f64, f64),
    /// Categorical: unique values
    Discrete(Vec<ScalarValue>),
    /// Temporal interval: (min, max) as Unix timestamps
    Temporal(i64, i64),
}

impl ScaleBuilder {
    /// Create a new empty scale builder
    pub fn new() -> Self {
        Self {
            channel_builders: HashMap::new(),
            channel_data_types: HashMap::new(),
        }
    }

    /// Set the data type for a channel
    pub fn set_channel_data_type(
        &mut self,
        channel_name: String,
        data_type: datafusion::arrow::datatypes::DataType,
    ) {
        self.channel_data_types.insert(channel_name, data_type);
    }

    /// Add a standard scale channel
    pub fn add_standard(
        &mut self,
        channel_name: String,
        scale_spec: Box<dyn crate::scales::ScaleSpec>,
        data_extents: DataExtents,
        options: HashMap<String, datafusion_proto::protobuf::LogicalExprNode>,
    ) {
        self.channel_builders.insert(
            channel_name,
            ChannelScaleBuilder::Standard {
                scale_spec,
                data_extents,
                options,
            },
        );
    }

    /// Add a radius-aware scale channel
    pub fn add_radius_aware(
        &mut self,
        channel_name: String,
        scale_spec: Box<dyn crate::scales::ScaleSpec>,
        position_data: Vec<f64>,
        radius_lower_data: Vec<f64>,
        radius_upper_data: Vec<f64>,
        options: HashMap<String, datafusion_proto::protobuf::LogicalExprNode>,
    ) {
        self.channel_builders.insert(
            channel_name,
            ChannelScaleBuilder::RadiusAware {
                scale_spec,
                position_data,
                radius_lower_data,
                radius_upper_data,
                options,
            },
        );
    }

    /// Add a scale with explicit domain (no data caching needed)
    pub fn add_explicit_domain(
        &mut self,
        channel_name: String,
        scale_spec: Box<dyn crate::scales::ScaleSpec>,
        options: HashMap<String, datafusion_proto::protobuf::LogicalExprNode>,
        domain: crate::scales::ScaleDomain,
    ) {
        self.channel_builders.insert(
            channel_name,
            ChannelScaleBuilder::ExplicitDomain {
                scale_spec,
                options,
                domain,
            },
        );
    }

    /// Get the channel builders
    pub fn channel_builders(&self) -> &HashMap<String, ChannelScaleBuilder> {
        &self.channel_builders
    }

    /// Extract data extents for specified channels as SerializableDataExtents
    ///
    /// This is used to extract extents from a ScaleBuilder to pass to inner facets
    /// for SharedInColumn mode. Returns a HashMap of channel name to serializable extents.
    pub fn extract_serializable_extents(
        &self,
        channels: &[&str],
    ) -> std::collections::HashMap<String, crate::facet::coordination::SerializableDataExtents>
    {
        use crate::facet::coordination::SerializableDataExtents;

        let mut result = std::collections::HashMap::new();

        for channel in channels {
            if let Some(builder) = self.channel_builders.get(*channel) {
                match builder {
                    ChannelScaleBuilder::Standard { data_extents, .. } => {
                        let serializable = match data_extents {
                            DataExtents::Interval(min, max) => {
                                SerializableDataExtents::interval(*min, *max)
                            }
                            DataExtents::Temporal(min, max) => {
                                SerializableDataExtents::temporal(*min, *max)
                            }
                            DataExtents::Discrete(values) => {
                                SerializableDataExtents::discrete(values.clone())
                            }
                        };
                        result.insert(channel.to_string(), serializable);
                    }
                    ChannelScaleBuilder::RadiusAware {
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
                                SerializableDataExtents::radius_aware_interval(
                                    min,
                                    max,
                                    max_radius_lower,
                                    max_radius_upper,
                                ),
                            );
                        }
                    }
                    ChannelScaleBuilder::ExplicitDomain { .. } => {
                        // Explicit domain doesn't have extractable extents
                    }
                }
            }
        }

        result
    }

    /// Extend data extents using shared extents from coordination context
    ///
    /// This is used for nested facets with shared scales. The outer facet computes
    /// the data extents from the FULL dataset and passes them through the coordination
    /// context. This method extends the local extents (computed from filtered data)
    /// to include the full dataset range, ensuring consistent scale domains across
    /// all subplots.
    ///
    /// For interval extents, the resulting domain is the union (min of mins, max of maxes).
    pub fn extend_with_shared_extents(
        &mut self,
        shared_extents: &std::collections::HashMap<
            String,
            crate::facet::coordination::SerializableDataExtents,
        >,
    ) {
        use crate::facet::coordination::SerializableDataExtents;

        if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
            eprintln!(
                "extend_with_shared_extents called with {} channels",
                shared_extents.len()
            );
            for (ch, ext) in shared_extents {
                eprintln!("  shared extent {}: {:?}", ch, ext);
            }
            eprintln!(
                "  channel_builders available: {:?}",
                self.channel_builders.keys().collect::<Vec<_>>()
            );
        }

        for (channel, shared_extent) in shared_extents {
            if let Some(channel_builder) = self.channel_builders.get_mut(channel) {
                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                    let variant = match channel_builder {
                        ChannelScaleBuilder::Standard { data_extents, .. } => {
                            format!("Standard({:?})", data_extents)
                        }
                        ChannelScaleBuilder::RadiusAware { .. } => "RadiusAware".to_string(),
                        ChannelScaleBuilder::ExplicitDomain { .. } => "ExplicitDomain".to_string(),
                    };
                    eprintln!("  Processing channel {}: builder={}", channel, variant);
                }
                match channel_builder {
                    ChannelScaleBuilder::Standard { data_extents, .. } => {
                        // Extend the local extents with shared extents
                        match (data_extents, shared_extent) {
                            (
                                DataExtents::Interval(local_min, local_max),
                                SerializableDataExtents::Interval {
                                    min: shared_min,
                                    max: shared_max,
                                },
                            ) => {
                                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                                    eprintln!(
                                        "  Extending {} extents: local=({}, {}) shared=({}, {})",
                                        channel, local_min, local_max, shared_min, shared_max
                                    );
                                }
                                // Take union: min of mins, max of maxes
                                *local_min = local_min.min(*shared_min);
                                *local_max = local_max.max(*shared_max);
                                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                                    eprintln!("  Result: ({}, {})", local_min, local_max);
                                }
                            }
                            (
                                DataExtents::Temporal(local_min, local_max),
                                SerializableDataExtents::Temporal {
                                    min: shared_min,
                                    max: shared_max,
                                },
                            ) => {
                                *local_min = (*local_min).min(*shared_min);
                                *local_max = (*local_max).max(*shared_max);
                            }
                            (
                                DataExtents::Discrete(local_values),
                                SerializableDataExtents::Discrete(shared_values),
                            ) => {
                                // For categorical scale sharing, replace local values with shared values.
                                // The shared values represent the full dataset's unique values in sorted order,
                                // ensuring consistent category ordering across all facet cells.
                                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                                    eprintln!(
                                        "  Extending {} discrete extents: local={} values -> shared={} values",
                                        channel,
                                        local_values.len(),
                                        shared_values.len()
                                    );
                                }
                                *local_values =
                                    shared_values.iter().map(|v| v.to_scalar()).collect();
                            }
                            _ => {
                                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                                    eprintln!(
                                        "  No match for data_extents/shared_extent combination"
                                    );
                                }
                            }
                        }
                    }
                    ChannelScaleBuilder::RadiusAware {
                        position_data,
                        radius_lower_data,
                        radius_upper_data,
                        ..
                    } => {
                        // For RadiusAware scales, we need to add synthetic data points at the shared extents
                        // to ensure the domain includes them. Use shared radius values if available.
                        match shared_extent {
                            SerializableDataExtents::RadiusAwareInterval {
                                min: shared_min,
                                max: shared_max,
                                max_radius_lower,
                                max_radius_upper,
                            } => {
                                let len_before = position_data.len();

                                // Add synthetic points at shared min and max with proper radius values
                                position_data.push(*shared_min);
                                radius_lower_data.push(*max_radius_lower);
                                radius_upper_data.push(0.0);

                                position_data.push(*shared_max);
                                radius_lower_data.push(0.0);
                                radius_upper_data.push(*max_radius_upper);

                                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                                    eprintln!(
                                        "  Extended RadiusAware {} with shared radius-aware extents ({}, {}), radii=({}, {}), len {} -> {}",
                                        channel,
                                        shared_min,
                                        shared_max,
                                        max_radius_lower,
                                        max_radius_upper,
                                        len_before,
                                        position_data.len()
                                    );
                                }
                            }
                            SerializableDataExtents::Interval {
                                min: shared_min,
                                max: shared_max,
                            } => {
                                let len_before = position_data.len();

                                // Compute max radius values from existing data to ensure proper
                                // domain extension. Without this, Level(N) domains stored as plain
                                // Interval would not extend beyond exact min/max, while Shared
                                // domains with RadiusAwareInterval would extend properly.
                                let max_radius_lower = radius_lower_data
                                    .iter()
                                    .copied()
                                    .fold(0.0_f64, f64::max);
                                let max_radius_upper = radius_upper_data
                                    .iter()
                                    .copied()
                                    .fold(0.0_f64, f64::max);

                                // Add synthetic points at shared min and max with proper radius
                                // values to ensure radius-aware domain padding is applied
                                position_data.push(*shared_min);
                                radius_lower_data.push(max_radius_lower);
                                radius_upper_data.push(0.0);

                                position_data.push(*shared_max);
                                radius_lower_data.push(0.0);
                                radius_upper_data.push(max_radius_upper);

                                if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() {
                                    eprintln!(
                                        "  Extended RadiusAware {} with shared extents ({}, {}), radii=({:.1}, {:.1}), len {} -> {}",
                                        channel,
                                        shared_min,
                                        shared_max,
                                        max_radius_lower,
                                        max_radius_upper,
                                        len_before,
                                        position_data.len()
                                    );
                                }
                            }
                            _ => {}
                        }
                    }
                    ChannelScaleBuilder::ExplicitDomain { .. } => {
                        // Explicit domains should not be modified by shared extents
                    }
                }
            }
        }
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
    /// * `coord_system_ranges` - Map of channel name to (min, max) range values
    /// * `scale_specs` - Scale specifications for overrides
    /// * `compiled_marks` - Marks for getting default ranges
    /// * `theme` - Theme for default ranges
    /// * `ctx` - DataFusion session context
    /// * `params` - Parameters for expression evaluation
    ///
    /// # Returns
    /// HashMap of channel name to ConfiguredScaleWithSpec
    pub async fn build_scales(
        &self,
        width: f32,
        height: f32,
        coord_system_ranges: &HashMap<String, (f64, f64)>,
        scale_specs: &HashMap<String, crate::plot::ScaleSpec>,
        compiled_marks: &[std::sync::Arc<dyn crate::marks::CompiledMark>],
        theme: &crate::theme::Theme,
        ctx: &datafusion::prelude::SessionContext,
        params: &indexmap::IndexMap<String, datafusion_common::ScalarValue>,
    ) -> Result<HashMap<String, crate::scales::ConfiguredScaleWithSpec>, AvengerChartError> {
        use crate::scales::spec::Auto;
        use crate::scales::{ConfiguredScaleWithSpec, Scale};
        use datafusion::logical_expr::lit;

        let mut result = HashMap::new();

        // Iterate channel builders in a deterministic order
        let mut builder_entries: Vec<(&String, &ChannelScaleBuilder)> =
            self.channel_builders.iter().collect();
        builder_entries.sort_by(|a, b| a.0.cmp(b.0));

        for (channel_name, channel_builder) in builder_entries.into_iter() {
            if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() && channel_name == "y" {
                match channel_builder {
                    ChannelScaleBuilder::RadiusAware { position_data, .. } => {
                        eprintln!(
                            "build_scales: starting {} with RadiusAware(len={})",
                            channel_name,
                            position_data.len()
                        );
                    }
                    _ => {}
                }
            }
            match channel_builder {
                ChannelScaleBuilder::Standard {
                    scale_spec,
                    data_extents,
                    options,
                } => {
                    // For standard scales: use cached extents directly (no query!)
                    let mut scale = Scale::<Auto>::from_spec(scale_spec.as_ref().clone_box());

                    // Apply cached options
                    for (key, value_node) in options {
                        use crate::serialization::LogicalExprNodeExt;
                        let expr = value_node.to_expr(ctx)?;
                        scale = scale.option(key, expr);
                    }

                    // Check if there's an explicit scale spec that overrides the domain
                    // If so, we should NOT use the cached data_extents
                    let use_cached_domain = if let Some(spec) = scale_specs.get(channel_name) {
                        // Apply the spec to see if it has an explicit domain
                        match spec {
                            crate::plot::ScaleSpec::Local(scale_changes) => {
                                scale = scale.update(scale_changes.clone());
                            }
                        }

                        // Check if domain is still DomainExprs (needs data) or explicit
                        use crate::maybe::Maybe;
                        use crate::scales::domain::ScaleDefaultDomain;
                        match &scale.domain {
                            Maybe::Set(domain) => {
                                match &domain.default_domain {
                                    ScaleDefaultDomain::DomainExprs(_) => {
                                        true // Use cached data
                                    }
                                    _ => {
                                        false // Skip cached data, use explicit domain
                                    }
                                }
                            }
                            Maybe::Unset => {
                                true // Use cached data if domain not set
                            }
                        }
                    } else {
                        true // No override, use cached data
                    };

                    // Set domain from cached extents only if needed
                    if use_cached_domain {
                        let domain = data_extents.to_scale_domain()?;
                        scale = scale.domain(domain);
                    }

                    // Set range if available
                    if let Some((range_min, range_max)) = coord_system_ranges.get(channel_name) {
                        scale = scale.range_interval(lit(*range_min), lit(*range_max));
                    }

                    // Normalize domain (apply zero, nice, padding)
                    scale = scale.normalize_domain(width, height, ctx, params).await?;

                    // Apply default range if not already set (mirrors old code at scales.rs:1216-1246)
                    if scale.get_range().is_none() {
                        // Try to get default range from mark
                        if let Some(data_type) = self.channel_data_types.get(channel_name) {
                            if let (Some(scale_impl), Some(domain)) =
                                (scale.get_scale_impl(), scale.get_domain())
                            {
                                if let Ok(resolved_domain) = domain.to_resolved() {
                                    // Find first mark that uses this channel
                                    for mark in compiled_marks {
                                        if let Some(mark_range) = mark.default_channel_range(
                                            channel_name,
                                            scale_impl.as_ref(),
                                            &resolved_domain,
                                            data_type,
                                            theme,
                                            params,
                                        ) {
                                            scale = scale.range(mark_range);
                                            break;
                                        }
                                    }
                                }
                            }
                        }

                        // Final fallback: query theme with generic "mark" type, then use hardcoded defaults
                        if scale.get_range().is_none() {
                            let range_kind = scale
                                .get_scale_impl()
                                .map(|impl_arc| impl_arc.range_kind())
                                .unwrap_or(avenger_scales::scales::RangeKind::Continuous);

                            let range = theme
                                .get_range_for_channel(
                                    "mark",
                                    channel_name,
                                    range_kind,
                                    None,
                                    params,
                                )
                                .unwrap_or_else(|| {
                                    crate::scales::default_range_for_channel(
                                        channel_name,
                                        range_kind,
                                    )
                                });

                            scale = scale.range(range);
                        }
                    }

                    // Create configured scale
                    let configured = scale
                        .clone()
                        .create_configured_scale(width, height, ctx, params)
                        .await?;

                    result.insert(
                        channel_name.clone(),
                        ConfiguredScaleWithSpec::new(scale, configured),
                    );
                }
                ChannelScaleBuilder::RadiusAware {
                    scale_spec,
                    position_data,
                    radius_lower_data,
                    radius_upper_data,
                    options,
                } => {
                    // For radius-aware scales: recompute domain with new range (cheap math, no query!)
                    let mut scale = Scale::<Auto>::from_spec(scale_spec.as_ref().clone_box());

                    // Apply cached options
                    for (key, value_node) in options {
                        use crate::serialization::LogicalExprNodeExt;
                        let expr = value_node.to_expr(ctx)?;
                        scale = scale.option(key, expr);
                    }

                    // Get range for this channel
                    let (range_min, range_max) =
                        coord_system_ranges.get(channel_name).ok_or_else(|| {
                            AvengerChartError::InternalError(format!(
                                "No range found for radius-aware channel '{}'",
                                channel_name
                            ))
                        })?;

                    let range_width = (range_max - range_min).abs();

                    // Recompute domain with new range_width using cached data
                    use avenger_scales::scales::domain_solver::compute_domain_from_data_with_padding_linear;
                    let (d_min, d_max) = compute_domain_from_data_with_padding_linear(
                        position_data,
                        radius_lower_data,
                        radius_upper_data,
                        range_width,
                    )?;

                    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() && channel_name == "y" {
                        let data_preview: Vec<f64> =
                            position_data.iter().take(12).cloned().collect();
                        eprintln!(
                            "  RadiusAware {} domain after compute_domain_from_data: ({:.2}, {:.2}), position_data len={}, range_width={:.2}, data={:?}",
                            channel_name,
                            d_min,
                            d_max,
                            position_data.len(),
                            range_width,
                            data_preview
                        );
                    }

                    // Set domain and range
                    scale = scale.domain_interval(lit(d_min), lit(d_max));
                    scale = scale.range_interval(lit(*range_min), lit(*range_max));

                    // Normalize domain (apply zero, nice, padding)
                    scale = scale.normalize_domain(width, height, ctx, params).await?;

                    if std::env::var("AVENGER_CHART_DEBUG_LAYOUT").is_ok() && channel_name == "y" {
                        if let Ok((norm_min, norm_max)) = scale
                            .clone()
                            .create_configured_scale(width, height, ctx, params)
                            .await
                            .and_then(|c| Ok(c.numeric_interval_domain()?))
                        {
                            eprintln!(
                                "  RadiusAware {} domain after normalize: ({:.2}, {:.2})",
                                channel_name, norm_min, norm_max
                            );
                        }
                    }

                    // Apply default range if not already set (same logic as Standard path)
                    if scale.get_range().is_none() {
                        if let Some(data_type) = self.channel_data_types.get(channel_name) {
                            if let (Some(scale_impl), Some(domain)) =
                                (scale.get_scale_impl(), scale.get_domain())
                            {
                                if let Ok(resolved_domain) = domain.to_resolved() {
                                    for mark in compiled_marks {
                                        if let Some(mark_range) = mark.default_channel_range(
                                            channel_name,
                                            scale_impl.as_ref(),
                                            &resolved_domain,
                                            data_type,
                                            theme,
                                            params,
                                        ) {
                                            scale = scale.range(mark_range);
                                            break;
                                        }
                                    }
                                }
                            }
                        }

                        if scale.get_range().is_none() {
                            let range_kind = scale
                                .get_scale_impl()
                                .map(|impl_arc| impl_arc.range_kind())
                                .unwrap_or(avenger_scales::scales::RangeKind::Continuous);

                            let range = theme
                                .get_range_for_channel(
                                    "mark",
                                    channel_name,
                                    range_kind,
                                    None,
                                    params,
                                )
                                .unwrap_or_else(|| {
                                    crate::scales::default_range_for_channel(
                                        channel_name,
                                        range_kind,
                                    )
                                });

                            scale = scale.range(range);
                        }
                    }

                    // Create configured scale
                    let configured = scale
                        .clone()
                        .create_configured_scale(width, height, ctx, params)
                        .await?;

                    result.insert(
                        channel_name.clone(),
                        ConfiguredScaleWithSpec::new(scale, configured),
                    );
                }
                ChannelScaleBuilder::ExplicitDomain {
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
                        use crate::serialization::LogicalExprNodeExt;
                        let expr = value_node.to_expr(ctx)?;
                        scale = scale.option(key, expr);
                    }

                    // Apply scale spec overrides (which might override the explicit domain)
                    if let Some(spec) = scale_specs.get(channel_name) {
                        match spec {
                            crate::plot::ScaleSpec::Local(scale_changes) => {
                                scale = scale.update(scale_changes.clone());
                            }
                        }
                    }

                    // Set range if available
                    if let Some((range_min, range_max)) = coord_system_ranges.get(channel_name) {
                        scale = scale.range_interval(lit(*range_min), lit(*range_max));
                    }

                    // Normalize domain (apply zero, nice, padding)
                    scale = scale.normalize_domain(width, height, ctx, params).await?;

                    // Apply default range if not already set
                    if scale.get_range().is_none() {
                        if let Some(data_type) = self.channel_data_types.get(channel_name) {
                            if let (Some(scale_impl), Some(domain)) =
                                (scale.get_scale_impl(), scale.get_domain())
                            {
                                if let Ok(resolved_domain) = domain.to_resolved() {
                                    for mark in compiled_marks {
                                        if let Some(mark_range) = mark.default_channel_range(
                                            channel_name,
                                            scale_impl.as_ref(),
                                            &resolved_domain,
                                            data_type,
                                            theme,
                                            params,
                                        ) {
                                            scale = scale.range(mark_range);
                                            break;
                                        }
                                    }
                                }
                            }
                        }

                        if scale.get_range().is_none() {
                            let range_kind = scale
                                .get_scale_impl()
                                .map(|impl_arc| impl_arc.range_kind())
                                .unwrap_or(avenger_scales::scales::RangeKind::Continuous);

                            let range = theme
                                .get_range_for_channel(
                                    "mark",
                                    channel_name,
                                    range_kind,
                                    None,
                                    params,
                                )
                                .unwrap_or_else(|| {
                                    crate::scales::default_range_for_channel(
                                        channel_name,
                                        range_kind,
                                    )
                                });

                            scale = scale.range(range);
                        }
                    }

                    // Create configured scale
                    let configured = scale
                        .clone()
                        .create_configured_scale(width, height, ctx, params)
                        .await?;

                    result.insert(
                        channel_name.clone(),
                        ConfiguredScaleWithSpec::new(scale, configured),
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
    pub fn to_scale_domain(&self) -> Result<crate::scales::domain::ScaleDomain, AvengerChartError> {
        use crate::scales::domain::ScaleDomain;
        use datafusion::logical_expr::lit;

        match self {
            DataExtents::Interval(min, max) => Ok(ScaleDomain::new_interval(lit(*min), lit(*max))),
            DataExtents::Discrete(values) => {
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

    #[test]
    fn test_scale_builder_new() {
        let builder = ScaleBuilder::new();
        assert!(builder.channel_builders.is_empty());
    }

    #[test]
    fn test_add_standard_scale() {
        use crate::scales::spec::Linear;

        let mut builder = ScaleBuilder::new();
        let scale_spec = Box::new(Linear::default()) as Box<dyn crate::scales::ScaleSpec>;
        let data_extents = DataExtents::Interval(0.0, 100.0);
        let options = HashMap::new();

        builder.add_standard("x".to_string(), scale_spec, data_extents, options);

        assert_eq!(builder.channel_builders.len(), 1);
        assert!(builder.channel_builders.contains_key("x"));
    }

    #[test]
    fn test_add_radius_aware_scale() {
        use crate::scales::spec::Linear;

        let mut builder = ScaleBuilder::new();
        let scale_spec = Box::new(Linear::default()) as Box<dyn crate::scales::ScaleSpec>;
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

        assert_eq!(builder.channel_builders.len(), 1);
        assert!(builder.channel_builders.contains_key("x"));
    }

    #[test]
    fn test_data_extents_to_domain_interval() {
        let extents = DataExtents::Interval(0.0, 100.0);
        let domain = extents.to_scale_domain().unwrap();

        // Verify it created an interval domain
        match domain.default_domain {
            crate::scales::domain::ScaleDefaultDomain::Interval(_, _) => {
                // Success
            }
            _ => panic!("Expected interval domain"),
        }
    }

    #[test]
    fn test_data_extents_to_domain_discrete() {
        use datafusion_common::ScalarValue;

        let extents = DataExtents::Discrete(vec![
            ScalarValue::Utf8(Some("a".to_string())),
            ScalarValue::Utf8(Some("b".to_string())),
        ]);
        let domain = extents.to_scale_domain().unwrap();

        // Verify it created a discrete domain
        match domain.default_domain {
            crate::scales::domain::ScaleDefaultDomain::Discrete(values) => {
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
            crate::scales::domain::ScaleDefaultDomain::Interval(_, _) => {
                // Success
            }
            _ => panic!("Expected interval domain"),
        }
    }

    #[tokio::test]
    async fn test_build_scales_standard() {
        use crate::scales::spec::Linear;
        use datafusion::prelude::SessionContext;
        use indexmap::IndexMap;

        let mut builder = ScaleBuilder::new();
        let scale_spec = Box::new(Linear::default()) as Box<dyn crate::scales::ScaleSpec>;
        let data_extents = DataExtents::Interval(0.0, 100.0);
        let options = HashMap::new();

        builder.add_standard("x".to_string(), scale_spec, data_extents, options);

        let mut coord_ranges = HashMap::new();
        coord_ranges.insert("x".to_string(), (0.0, 400.0));

        let ctx = SessionContext::new();
        let params = IndexMap::new();

        let scales = builder
            .build_scales(
                400.0,
                300.0,
                &coord_ranges,
                &HashMap::new(),
                &[], // No marks in this test
                &crate::theme::Theme::light(),
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
    async fn test_build_scales_radius_aware() {
        use crate::scales::spec::Linear;
        use datafusion::prelude::SessionContext;
        use indexmap::IndexMap;

        let mut builder = ScaleBuilder::new();
        let scale_spec = Box::new(Linear::default()) as Box<dyn crate::scales::ScaleSpec>;
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
        coord_ranges.insert("x".to_string(), (0.0, 400.0));

        let ctx = SessionContext::new();
        let params = IndexMap::new();

        let scales = builder
            .build_scales(
                400.0,
                300.0,
                &coord_ranges,
                &HashMap::new(),
                &[], // No marks in this test
                &crate::theme::Theme::light(),
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
        use crate::scales::spec::Linear;
        use datafusion::prelude::SessionContext;
        use indexmap::IndexMap;

        let mut builder = ScaleBuilder::new();
        let scale_spec = Box::new(Linear::default()) as Box<dyn crate::scales::ScaleSpec>;
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
        coord_ranges1.insert("x".to_string(), (0.0, 400.0));

        let scales1 = builder
            .build_scales(
                400.0,
                300.0,
                &coord_ranges1,
                &HashMap::new(),
                &[], // No marks in this test
                &crate::theme::Theme::light(),
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
        coord_ranges2.insert("x".to_string(), (0.0, 800.0));

        let scales2 = builder
            .build_scales(
                800.0,
                300.0,
                &coord_ranges2,
                &HashMap::new(),
                &[], // No marks in this test
                &crate::theme::Theme::light(),
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
}
