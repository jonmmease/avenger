//! Validation methods for CompiledPlot

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use avenger_chart_core::{CompiledMark, DefaultLogicalExprNodeExt};
use avenger_chart_scales::PlotScaleSpec;
use datafusion::logical_expr::Expr;
use datafusion::prelude::SessionContext;
use datafusion_common::tree_node::{TreeNode, TreeNodeRecursion};

use crate::{
    error::AvengerChartError, facet::marks::facet::facet_subplot_ref,
    scales::ConfiguredScaleWithSpec,
};

use super::CompiledPlot;

impl CompiledPlot {
    /// Validate that all required positional scales exist
    pub(super) fn validate_positional_scales_exist(
        &self,
        _scales: &HashMap<String, ConfiguredScaleWithSpec>,
    ) -> Result<(), AvengerChartError> {
        // Get the required positional channels from the coordinate system
        let required_channels = self.coord_transform.required_channels();

        // Build a set of positional channel names to check
        // Include both base channels and their interval variants (e.g., "x" and "x2")
        let mut positional_channels = HashSet::new();
        for &channel in required_channels {
            positional_channels.insert(channel.to_string());
            // Also check for interval variant (e.g., "x2" for "x")
            positional_channels.insert(format!("{}2", channel));
        }

        // Check which marks use positional channels
        let mut used_channels = HashSet::new();
        for mark in &self.marks {
            for channel_name in mark.data_context().channels().keys() {
                if positional_channels.contains(channel_name) {
                    used_channels.insert(channel_name.clone());
                }
            }
        }

        // For now, we'll just return Ok since scale building happens elsewhere
        // In the future, we might want to validate that scales exist for used channels
        Ok(())
    }

    /// Validate that positional channels have numeric data types
    pub(super) fn validate_positional_channel_types(
        &self,
        data_batch: &Option<datafusion::arrow::record_batch::RecordBatch>,
        scalar_batch: &datafusion::arrow::record_batch::RecordBatch,
    ) -> Result<(), AvengerChartError> {
        // Get coordinate system name for error messages
        let coord_system_name = std::any::type_name_of_val(&*self.coord_transform);

        // Check each positional channel
        for channel_name in self.coord_transform.required_channels() {
            // Check in data batch first
            if let Some(data) = data_batch
                && let Some(column) = data.column_by_name(channel_name)
            {
                let dtype = column.data_type();
                if !Self::is_numeric_type(dtype) {
                    return self.create_positional_type_error(
                        channel_name,
                        dtype,
                        coord_system_name,
                    );
                }
            }

            // Check in scalar batch
            if let Some(column) = scalar_batch.column_by_name(channel_name) {
                let dtype = column.data_type();
                if !Self::is_numeric_type(dtype) {
                    return self.create_positional_type_error(
                        channel_name,
                        dtype,
                        coord_system_name,
                    );
                }
            }
        }

        Ok(())
    }

    /// Validate that raw-domain placeholder params are shared at least as broadly
    /// as the scales they drive.
    ///
    /// A `raw_domain` param partitioned more finely than the shared domain it
    /// feeds is ambiguous: multiple per-cell param values would map onto a single
    /// shared-domain group, so "the domain" is undefined. The rule (using
    /// `CoordinationScope::to_level()`, where a higher level is broader) is: error when
    /// `param_level < scale_level`. `Shared` params (level `u8::MAX`) are always
    /// valid; a `Free` param (0) is valid only for a `Free`/unscoped scale.
    ///
    /// Params are declared at the (root) plot while the `raw_domain`-driven scales
    /// usually live in faceted leaf subplots, so this collects param sharing
    /// levels top-down and then recurses through facet subplots to reach the
    /// scales.
    pub(crate) fn validate_scoped_raw_domain_sharing(
        &self,
        ctx: &SessionContext,
    ) -> Result<(), AvengerChartError> {
        let mut param_levels: HashMap<String, u8> = HashMap::new();
        self.collect_param_sharing_levels(&mut param_levels);
        self.validate_raw_domain_sharing_recursive(ctx, &param_levels)
    }

    pub(crate) fn validate_transform_output_scale_sharing(&self) -> Result<(), AvengerChartError> {
        self.validate_transform_output_scale_sharing_recursive()
    }

    fn collect_param_sharing_levels(&self, out: &mut HashMap<String, u8>) {
        for (name, spec) in &self.param_specs {
            out.entry(name.clone())
                .or_insert_with(|| spec.sharing.to_level());
        }
        for mark in &self.marks {
            if let Some(facet) = facet_subplot_ref(mark.as_ref()) {
                facet.compiled_subplot().collect_param_sharing_levels(out);
            }
        }
    }

    fn validate_raw_domain_sharing_recursive(
        &self,
        ctx: &SessionContext,
        param_levels: &HashMap<String, u8>,
    ) -> Result<(), AvengerChartError> {
        let scale_share_levels = scale_domain_share_levels(&self.marks);
        for (scale_name, spec) in &self.scale_specs {
            let PlotScaleSpec::Local(config) = spec;
            let Some(domain) = config.domain.as_option() else {
                continue;
            };
            let Some(raw_domain) = &domain.raw_domain else {
                continue;
            };
            let expr = raw_domain.to_expr(ctx)?;
            // An unscoped scale (no explicit `share_mode`) defaults to per-cell
            // (`Free`, level 0), which can never be stricter than any param.
            let scale_level = scale_share_levels.get(scale_name).copied().unwrap_or(0);
            for param_name in placeholder_param_names(&expr)? {
                let Some(&param_level) = param_levels.get(&param_name) else {
                    continue;
                };
                if param_level < scale_level {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "raw-domain param '{param}' is shared at {param_sharing} but scale \
                         '{scale}' is shared at {scale_sharing}; the param must be shared at \
                         {scale_sharing} or broader so every shared-domain group resolves to a \
                         single param value",
                        param = param_name,
                        param_sharing = describe_sharing_level(param_level),
                        scale = scale_name,
                        scale_sharing = describe_sharing_level(scale_level),
                    )));
                }
            }
        }
        for mark in &self.marks {
            if let Some(facet) = facet_subplot_ref(mark.as_ref()) {
                facet
                    .compiled_subplot()
                    .validate_raw_domain_sharing_recursive(ctx, param_levels)?;
            }
        }
        Ok(())
    }

    fn validate_transform_output_scale_sharing_recursive(&self) -> Result<(), AvengerChartError> {
        let scale_share_levels = scale_domain_share_levels(&self.marks);
        for mark in &self.marks {
            if let Some(facet) = facet_subplot_ref(mark.as_ref()) {
                facet
                    .compiled_subplot()
                    .validate_transform_output_scale_sharing_recursive()?;
                continue;
            }
            for (channel_name, channel_value) in mark.data_context().channels() {
                let Some(transform_scope) = channel_value.get_transform_scope() else {
                    continue;
                };
                let Some(scale_name) = channel_value.get_scale_name(channel_name) else {
                    continue;
                };
                let transform_level = transform_scope.to_level();
                let scale_level = scale_share_levels.get(&scale_name).copied().unwrap_or(0);
                if scale_level > transform_level {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "channel '{channel}' uses values produced by a transform at {transform_sharing}, \
                         but scale '{scale}' is shared at {scale_sharing}; the scale cannot be shared \
                         more broadly than the transform output that feeds it",
                        channel = channel_name,
                        transform_sharing = describe_sharing_level(transform_level),
                        scale = scale_name,
                        scale_sharing = describe_sharing_level(scale_level),
                    )));
                }
            }
        }
        Ok(())
    }

    /// Create error for non-numeric positional channel
    fn create_positional_type_error(
        &self,
        channel_name: &str,
        dtype: &datafusion::arrow::datatypes::DataType,
        coord_system_name: &str,
    ) -> Result<(), AvengerChartError> {
        use datafusion::arrow::datatypes::DataType;

        // Extract coordinate system name (remove module path)
        let coord_system = coord_system_name
            .split("::")
            .last()
            .unwrap_or("Unknown")
            .to_string();

        // Provide helpful suggestion based on the data type
        let (literal_value, suggestion) = match dtype {
            DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => (
                "string literal".to_string(),
                "Use col(\"column_name\") to reference a data column instead of a string literal.\n  \
                         If you need a fixed position, use a numeric value like lit(100.0)".to_string(),
            ),
            _ => (
                format!("{:?} value", dtype),
                "Positional channels require numeric values. \
                         Use col(\"column_name\") to reference a numeric column.".to_string(),
            ),
        };

        Err(AvengerChartError::PositionalScaleLiteralError {
            scale_name: channel_name.to_string(),
            coord_system,
            literal_value,
            suggestion,
        })
    }
}

/// Map each scale name to the broadest domain sharing level declared by the
/// (non-facet) marks at this plot level.
///
/// Channels without an explicit `share_mode` are omitted (treated as `Free`,
/// level 0, by the caller). Facet subplot marks are skipped because their
/// channels live one nesting level deeper and are validated by the recursion.
fn scale_domain_share_levels(marks: &[Arc<dyn CompiledMark>]) -> HashMap<String, u8> {
    let mut result: HashMap<String, u8> = HashMap::new();
    for mark in marks {
        if facet_subplot_ref(mark.as_ref()).is_some() {
            continue;
        }
        for (channel_name, channel_value) in mark.data_context().channels() {
            let Some(level) = channel_value.get_share_mode().map(|mode| mode.to_level()) else {
                continue;
            };
            let Some(scale_name) = channel_value.get_scale_name(channel_name) else {
                continue;
            };
            result
                .entry(scale_name)
                .and_modify(|existing| *existing = (*existing).max(level))
                .or_insert(level);
        }
    }
    result
}

/// Collect the names of placeholder params (`$name`) referenced anywhere in a
/// raw-domain expression.
fn placeholder_param_names(expr: &Expr) -> Result<Vec<String>, AvengerChartError> {
    let mut names = Vec::new();
    expr.apply(|node| {
        if let Expr::Placeholder(placeholder) = node
            && let Some(name) = placeholder.id.strip_prefix('$')
        {
            names.push(name.to_string());
        }
        Ok(TreeNodeRecursion::Continue)
    })
    .map_err(|err| {
        AvengerChartError::InternalError(format!("walk raw-domain expression: {err}"))
    })?;
    Ok(names)
}

/// Human-readable description of a `CoordinationScope::to_level()` value.
fn describe_sharing_level(level: u8) -> String {
    match level {
        0 => "Free".to_string(),
        u8::MAX => "Shared".to_string(),
        n => format!("Level({n})"),
    }
}
