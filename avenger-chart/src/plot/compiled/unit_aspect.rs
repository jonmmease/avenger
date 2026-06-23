use std::collections::HashMap;

use avenger_chart_core::{
    AvengerChartError, ResolvedUnitAspectConstraint, SharingLevel, UnitAspectAdjustedAxis,
    UnitAspectAdjustment, UnitAspectPolicy,
};
use avenger_chart_scales::ConfiguredScaleWithSpec;
use avenger_scales::scales::{DomainKind, RangeKind};

use super::{CompiledPlot, child_frame_domain_sharing_levels_for_plot};
const UNIT_ASPECT_EPSILON: f64 = 1e-9;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum UnitAspectSharingPolicy {
    AllowSharedExpansion,
    ForbidSharedExpansion,
}

impl CompiledPlot {
    pub(crate) fn apply_unit_aspect_constraints(
        &self,
        scales: &mut HashMap<String, ConfiguredScaleWithSpec>,
        sharing_policy: UnitAspectSharingPolicy,
    ) -> Result<Vec<UnitAspectAdjustment>, AvengerChartError> {
        let constraints = self.resolved_unit_aspect_constraints()?;
        if constraints.is_empty() {
            return Ok(Vec::new());
        }

        if matches!(
            sharing_policy,
            UnitAspectSharingPolicy::ForbidSharedExpansion
        ) {
            validate_local_unit_aspect_sharing(self, &constraints)?;
        }

        constraints
            .iter()
            .map(|constraint| apply_unit_aspect_constraint(scales, constraint))
            .collect()
    }
}

fn validate_local_unit_aspect_sharing(
    plot: &CompiledPlot,
    constraints: &[ResolvedUnitAspectConstraint],
) -> Result<(), AvengerChartError> {
    let sharing = child_frame_domain_sharing_levels_for_plot(plot);
    for constraint in constraints {
        for (axis, scale_name) in [
            ("x", constraint.x_scale.as_str()),
            ("y", constraint.y_scale.as_str()),
        ] {
            let Some(coordination) = sharing.get(scale_name) else {
                continue;
            };
            if SharingLevel::from(coordination.scope).is_free() {
                continue;
            }
            return Err(AvengerChartError::InvalidArgument(format!(
                "unit_aspect scale '{scale_name}' ({axis} channel) uses non-free domain sharing; \
                 shared unit_aspect domains require the sharing-aware domain solver"
            )));
        }
    }
    Ok(())
}

fn apply_unit_aspect_constraint(
    scales: &mut HashMap<String, ConfiguredScaleWithSpec>,
    constraint: &ResolvedUnitAspectConstraint,
) -> Result<UnitAspectAdjustment, AvengerChartError> {
    if constraint.policy != UnitAspectPolicy::ExpandDomain {
        return Err(AvengerChartError::InvalidArgument(
            "unit_aspect only supports expand-domain policy in v1".to_string(),
        ));
    }

    let x_domain = numeric_linear_domain(scales, &constraint.x_scale, "x")?;
    let y_domain = numeric_linear_domain(scales, &constraint.y_scale, "y")?;
    let x_range = numeric_linear_range(scales, &constraint.x_scale, "x")?;
    let y_range = numeric_linear_range(scales, &constraint.y_scale, "y")?;

    let x_span = finite_positive_span(x_domain, "x domain", &constraint.x_scale)?;
    let y_span = finite_positive_span(y_domain, "y domain", &constraint.y_scale)?;
    let x_range_span = finite_positive_span(x_range, "x range", &constraint.x_scale)?;
    let y_range_span = finite_positive_span(y_range, "y range", &constraint.y_scale)?;
    if !constraint.ratio.is_finite() || constraint.ratio <= 0.0 {
        return Err(AvengerChartError::InvalidArgument(format!(
            "unit_aspect ratio must be positive and finite, got {}",
            constraint.ratio
        )));
    }

    let actual_ratio = (y_range_span / y_span) / (x_range_span / x_span);
    let mut adjusted_axis = UnitAspectAdjustedAxis::None;
    let mut adjusted_x_domain = x_domain;
    let mut adjusted_y_domain = y_domain;

    if (actual_ratio - constraint.ratio).abs() > UNIT_ASPECT_EPSILON {
        if actual_ratio > constraint.ratio {
            let target_y_span = y_range_span * x_span / (constraint.ratio * x_range_span);
            adjusted_y_domain = expanded_domain_around_center(y_domain, target_y_span);
            adjusted_axis = UnitAspectAdjustedAxis::Y;
        } else {
            let target_x_span = constraint.ratio * x_range_span * y_span / y_range_span;
            adjusted_x_domain = expanded_domain_around_center(x_domain, target_x_span);
            adjusted_axis = UnitAspectAdjustedAxis::X;
        }
    }

    if adjusted_axis == UnitAspectAdjustedAxis::X {
        let x_scale = scales.get_mut(&constraint.x_scale).ok_or_else(|| {
            AvengerChartError::InvalidArgument(format!(
                "unit_aspect x scale '{}' is missing",
                constraint.x_scale
            ))
        })?;
        x_scale.set_configured(
            x_scale
                .configured()
                .clone()
                .with_domain_interval((adjusted_x_domain.0 as f32, adjusted_x_domain.1 as f32)),
        );
    } else if adjusted_axis == UnitAspectAdjustedAxis::Y {
        let y_scale = scales.get_mut(&constraint.y_scale).ok_or_else(|| {
            AvengerChartError::InvalidArgument(format!(
                "unit_aspect y scale '{}' is missing",
                constraint.y_scale
            ))
        })?;
        y_scale.set_configured(
            y_scale
                .configured()
                .clone()
                .with_domain_interval((adjusted_y_domain.0 as f32, adjusted_y_domain.1 as f32)),
        );
    }

    Ok(UnitAspectAdjustment {
        x_scale: constraint.x_scale.clone(),
        y_scale: constraint.y_scale.clone(),
        adjusted_axis,
        original_x_domain: x_domain,
        original_y_domain: y_domain,
        adjusted_x_domain,
        adjusted_y_domain,
    })
}

fn numeric_linear_domain(
    scales: &HashMap<String, ConfiguredScaleWithSpec>,
    scale_name: &str,
    axis: &str,
) -> Result<(f64, f64), AvengerChartError> {
    let scale = numeric_linear_scale(scales, scale_name, axis)?;
    scale
        .configured()
        .numeric_interval_domain()
        .map(|(a, b)| (a as f64, b as f64))
        .map_err(|err| {
            AvengerChartError::InvalidArgument(format!(
                "unit_aspect {axis} scale '{scale_name}' requires a numeric interval domain: {err}"
            ))
        })
}

fn numeric_linear_range(
    scales: &HashMap<String, ConfiguredScaleWithSpec>,
    scale_name: &str,
    axis: &str,
) -> Result<(f64, f64), AvengerChartError> {
    let scale = numeric_linear_scale(scales, scale_name, axis)?;
    scale
        .configured()
        .numeric_interval_range()
        .map(|(a, b)| (a as f64, b as f64))
        .map_err(|err| {
            AvengerChartError::InvalidArgument(format!(
                "unit_aspect {axis} scale '{scale_name}' requires a numeric interval range: {err}"
            ))
        })
}

fn numeric_linear_scale<'a>(
    scales: &'a HashMap<String, ConfiguredScaleWithSpec>,
    scale_name: &str,
    axis: &str,
) -> Result<&'a ConfiguredScaleWithSpec, AvengerChartError> {
    let scale = scales.get(scale_name).ok_or_else(|| {
        AvengerChartError::InvalidArgument(format!(
            "unit_aspect {axis} scale '{scale_name}' is missing"
        ))
    })?;
    let configured = scale.configured();
    let scale_impl = configured.scale_impl.as_ref();
    if scale_impl.scale_type() != "linear"
        || scale_impl.domain_kind() != DomainKind::Numeric
        || scale_impl.range_kind() != RangeKind::Continuous
    {
        return Err(AvengerChartError::InvalidArgument(format!(
            "unit_aspect {axis} scale '{scale_name}' requires a continuous linear numeric scale, got {}",
            scale_impl.scale_type()
        )));
    }
    Ok(scale)
}

fn finite_positive_span(
    interval: (f64, f64),
    label: &str,
    scale_name: &str,
) -> Result<f64, AvengerChartError> {
    let span = (interval.1 - interval.0).abs();
    if span.is_finite() && span > 0.0 {
        Ok(span)
    } else {
        Err(AvengerChartError::InvalidArgument(format!(
            "unit_aspect {label} for scale '{scale_name}' must have positive finite span, got {:?}",
            interval
        )))
    }
}

fn expanded_domain_around_center(domain: (f64, f64), target_span: f64) -> (f64, f64) {
    let center = (domain.0 + domain.1) / 2.0;
    let half = target_span / 2.0;
    if domain.0 <= domain.1 {
        (center - half, center + half)
    } else {
        (center + half, center - half)
    }
}
