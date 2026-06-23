//! Validation methods for CompiledPlot

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use avenger_chart_core::{
    CompiledMark, CompiledParamSpec, DefaultLogicalExprNodeExt, DomainCoordination,
    DomainCoordinationGroup,
};
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
        let mut param_specs: HashMap<String, CompiledParamSpec> = HashMap::new();
        self.collect_param_specs(&mut param_specs);
        let mut raw_domain_param_groups = HashMap::new();
        self.validate_raw_domain_sharing_recursive(ctx, &param_specs, &mut raw_domain_param_groups)
    }

    pub(crate) fn validate_transform_output_scale_sharing(&self) -> Result<(), AvengerChartError> {
        self.validate_transform_output_scale_sharing_recursive()
    }

    pub(crate) fn validate_unit_aspect_constraints(&self) -> Result<(), AvengerChartError> {
        if self.has_unit_aspect_constraints() {
            self.resolved_unit_aspect_constraints()?;
        }
        Ok(())
    }

    fn collect_param_specs(&self, out: &mut HashMap<String, CompiledParamSpec>) {
        for (name, spec) in &self.param_specs {
            out.entry(name.clone()).or_insert_with(|| spec.clone());
        }
        for mark in &self.marks {
            if let Some(facet) = facet_subplot_ref(mark.as_ref()) {
                facet.compiled_subplot().collect_param_specs(out);
            }
        }
    }

    fn validate_raw_domain_sharing_recursive(
        &self,
        ctx: &SessionContext,
        param_specs: &HashMap<String, CompiledParamSpec>,
        raw_domain_param_groups: &mut HashMap<String, String>,
    ) -> Result<(), AvengerChartError> {
        let scale_coordinations =
            scale_domain_coordinations(&self.marks, self.coord_transform.as_ref())?;
        for (scale_name, spec) in &self.scale_specs {
            let PlotScaleSpec::Local(config) = spec;
            let Some(domain) = config.domain.as_option() else {
                continue;
            };
            let Some(raw_domain) = &domain.raw_domain else {
                continue;
            };
            let expr = raw_domain.to_expr(ctx)?;
            // An unscoped scale (no explicit `domain_coordination`) defaults to per-cell
            // (`Free`, level 0), which can never be stricter than any param.
            let scale_coordination = scale_coordinations
                .get(scale_name)
                .cloned()
                .unwrap_or_default();
            let scale_coordination =
                resolve_scale_domain_coordination(scale_name, &scale_coordination)?;
            let scale_level = scale_coordination.scope.to_level();
            for param_name in placeholder_param_names(&expr)? {
                let Some(param_spec) = param_specs.get(&param_name) else {
                    continue;
                };
                let param_level = param_spec.sharing.to_level();
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
                if let Some(param_coordination) = &param_spec.domain_coordination {
                    let param_coordination =
                        resolve_scale_domain_coordination(scale_name, param_coordination)?;
                    if resolved_domain_group_id(&param_coordination)
                        != resolved_domain_group_id(&scale_coordination)
                    {
                        return Err(AvengerChartError::InvalidArgument(format!(
                            "raw-domain param '{param}' is coordinated with domain group \
                             '{param_group}' but scale '{scale}' is coordinated with domain group \
                             '{scale_group}'",
                            param = param_name,
                            param_group = resolved_domain_group_id(&param_coordination),
                            scale = scale_name,
                            scale_group = resolved_domain_group_id(&scale_coordination),
                        )));
                    }
                }
                let scale_group = resolved_domain_group_id(&scale_coordination);
                match raw_domain_param_groups.get(&param_name) {
                    Some(existing) if existing != &scale_group => {
                        return Err(AvengerChartError::InvalidArgument(format!(
                            "raw-domain param '{param}' drives incompatible domain groups \
                             '{first}' and '{second}'",
                            param = param_name,
                            first = existing,
                            second = scale_group,
                        )));
                    }
                    Some(_) => {}
                    None => {
                        raw_domain_param_groups.insert(param_name, scale_group);
                    }
                }
            }
        }
        for mark in &self.marks {
            if let Some(facet) = facet_subplot_ref(mark.as_ref()) {
                facet
                    .compiled_subplot()
                    .validate_raw_domain_sharing_recursive(
                        ctx,
                        param_specs,
                        raw_domain_param_groups,
                    )?;
            }
        }
        Ok(())
    }

    fn validate_transform_output_scale_sharing_recursive(&self) -> Result<(), AvengerChartError> {
        let scale_coordinations =
            scale_domain_coordinations(&self.marks, self.coord_transform.as_ref())?;
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
                let scale_level = scale_coordinations
                    .get(&scale_name)
                    .map(|coordination| coordination.scope.to_level())
                    .unwrap_or(0);
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

/// Map each scale name to the broadest domain coordination target declared by the
/// (non-facet) marks at this plot level.
///
/// Channels without an explicit `domain_coordination` are omitted (treated as
/// `Free` with scale-name grouping by the caller). Facet subplot marks are
/// skipped because their channels live one nesting level deeper and are
/// validated by the recursion.
fn scale_domain_coordinations(
    marks: &[Arc<dyn CompiledMark>],
    coord_transform: &dyn avenger_chart_core::CoordinateSystemTransformCore,
) -> Result<HashMap<String, DomainCoordination>, AvengerChartError> {
    let mut result: HashMap<String, DomainCoordination> = HashMap::new();
    for mark in marks {
        if facet_subplot_ref(mark.as_ref()).is_some() {
            continue;
        }
        for (channel_name, channel_value) in mark.data_context().channels() {
            if !coord_transform.channel_uses_scale(channel_name) {
                continue;
            }
            let Some(coordination) = channel_value.get_domain_coordination().cloned() else {
                continue;
            };
            let Some(scale_name) = channel_value.get_scale_name(channel_name) else {
                continue;
            };
            match result.get_mut(&scale_name) {
                Some(existing) => {
                    if existing.group != coordination.group {
                        return Err(AvengerChartError::InvalidArgument(format!(
                            "scale '{scale_name}' has incompatible domain groups"
                        )));
                    }
                    if coordination.scope.to_level() > existing.scope.to_level() {
                        existing.scope = coordination.scope;
                    }
                }
                None => {
                    result.insert(scale_name, coordination);
                }
            }
        }
    }
    Ok(result)
}

fn resolve_scale_domain_coordination(
    scale_name: &str,
    coordination: &DomainCoordination,
) -> Result<DomainCoordination, AvengerChartError> {
    let group = match &coordination.group {
        DomainCoordinationGroup::ScaleName => scale_name.to_string(),
        DomainCoordinationGroup::Named(group) => group.clone(),
    };
    Ok(DomainCoordination::new(
        coordination.scope,
        DomainCoordinationGroup::Named(group),
    ))
}

fn resolved_domain_group_id(coordination: &DomainCoordination) -> String {
    match &coordination.group {
        DomainCoordinationGroup::ScaleName => "<scale-name>".to_string(),
        DomainCoordinationGroup::Named(group) => group.clone(),
    }
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
