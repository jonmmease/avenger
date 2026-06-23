use std::collections::{HashMap, VecDeque};

use avenger_chart_core::{
    AvengerChartError, ResolvedUnitAspectConstraint, SharingLevel, UnitAspectAdjustedAxis,
    UnitAspectAdjustment, UnitAspectPolicy,
};
use avenger_chart_scales::ConfiguredScaleWithSpec;
use avenger_chart_scales::domain_extent::{DomainBounds, DomainExtent};
use avenger_scales::scales::{DomainKind, RangeKind};

use super::{
    CompiledPlot, CoordinationScopeKey, child_frame_domain_sharing_levels_for_plot,
    union_domain_extents,
};
const UNIT_ASPECT_EPSILON: f64 = 1e-9;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum UnitAspectSharingPolicy {
    AllowSharedExpansion,
    ForbidSharedExpansion,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum UnitAspectDomainNode {
    Free { cell: String, scale: String },
    Shared(CoordinationScopeKey),
}

#[derive(Clone, Debug)]
pub(crate) struct UnitAspectSpanGraphInput {
    pub(crate) x_node: UnitAspectDomainNode,
    pub(crate) y_node: UnitAspectDomainNode,
    pub(crate) x_extent: DomainExtent,
    pub(crate) y_extent: DomainExtent,
    pub(crate) x_range_span: f64,
    pub(crate) y_range_span: f64,
    pub(crate) ratio: f64,
}

pub(crate) fn solve_unit_aspect_span_graph(
    inputs: &[UnitAspectSpanGraphInput],
) -> Result<HashMap<UnitAspectDomainNode, DomainExtent>, AvengerChartError> {
    let mut extents = HashMap::<UnitAspectDomainNode, DomainExtent>::new();
    let mut edges = HashMap::<UnitAspectDomainNode, Vec<(UnitAspectDomainNode, f64)>>::new();

    for input in inputs {
        validate_positive_finite(input.ratio, "unit_aspect ratio")?;
        let x_range_span = validate_positive_finite(input.x_range_span, "x range span")?;
        let y_range_span = validate_positive_finite(input.y_range_span, "y range span")?;
        merge_node_extent(&mut extents, input.x_node.clone(), input.x_extent.clone());
        merge_node_extent(&mut extents, input.y_node.clone(), input.y_extent.clone());

        let offset = (y_range_span / (input.ratio * x_range_span)).ln();
        if input.x_node == input.y_node {
            if offset.abs() > UNIT_ASPECT_EPSILON {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "unit_aspect uses the same domain node for x and y but the plot-area \
                     equation requires different spans"
                )));
            }
            continue;
        }

        edges
            .entry(input.x_node.clone())
            .or_default()
            .push((input.y_node.clone(), offset));
        edges
            .entry(input.y_node.clone())
            .or_default()
            .push((input.x_node.clone(), -offset));
    }

    let mut offsets = HashMap::<UnitAspectDomainNode, f64>::new();
    let mut component_by_node = HashMap::<UnitAspectDomainNode, UnitAspectDomainNode>::new();
    for node in extents.keys() {
        if offsets.contains_key(node) {
            continue;
        }
        let component = node.clone();
        offsets.insert(node.clone(), 0.0);
        component_by_node.insert(node.clone(), component.clone());
        let mut queue = VecDeque::from([node.clone()]);
        while let Some(current) = queue.pop_front() {
            let current_offset = offsets[&current];
            for (next, edge_offset) in edges.get(&current).into_iter().flatten() {
                let candidate = current_offset + edge_offset;
                match offsets.get(next) {
                    Some(existing) => {
                        if (existing - candidate).abs() > UNIT_ASPECT_EPSILON {
                            return Err(AvengerChartError::InvalidArgument(
                                "unit_aspect shared domain equations are inconsistent".to_string(),
                            ));
                        }
                    }
                    None => {
                        offsets.insert(next.clone(), candidate);
                        component_by_node.insert(next.clone(), component.clone());
                        queue.push_back(next.clone());
                    }
                }
            }
        }
    }

    let mut component_required_shift = HashMap::<UnitAspectDomainNode, f64>::new();
    for node in extents.keys() {
        let component = component_by_node[node].clone();
        let base_span = numeric_extent_span(&extents[node], node)?;
        let required = base_span.ln() - offsets[node];
        component_required_shift
            .entry(component)
            .and_modify(|existing| *existing = existing.max(required))
            .or_insert(required);
    }

    extents
        .into_iter()
        .map(|(node, extent)| {
            let component = component_by_node[&node].clone();
            let shift = component_required_shift[&component];
            let target_span = (offsets[&node] + shift).exp();
            Ok((node, expand_numeric_extent(&extent, target_span)))
        })
        .collect()
}

fn validate_positive_finite(value: f64, label: &str) -> Result<f64, AvengerChartError> {
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(AvengerChartError::InvalidArgument(format!(
            "{label} must be positive and finite, got {value}"
        )))
    }
}

fn merge_node_extent(
    extents: &mut HashMap<UnitAspectDomainNode, DomainExtent>,
    node: UnitAspectDomainNode,
    extent: DomainExtent,
) {
    extents
        .entry(node)
        .and_modify(|existing| *existing = union_domain_extents(existing, &extent))
        .or_insert(extent);
}

fn numeric_extent_span(
    extent: &DomainExtent,
    node: &UnitAspectDomainNode,
) -> Result<f64, AvengerChartError> {
    let DomainBounds::Numeric { min, max } = &extent.bounds else {
        return Err(AvengerChartError::InvalidArgument(format!(
            "unit_aspect span graph requires numeric extents, got {node:?}"
        )));
    };
    validate_positive_finite((max - min).abs(), "domain span")
}

fn expand_numeric_extent(extent: &DomainExtent, target_span: f64) -> DomainExtent {
    let DomainBounds::Numeric { min, max } = &extent.bounds else {
        return extent.clone();
    };
    let center = (*min + *max) / 2.0;
    let half = target_span / 2.0;
    DomainExtent {
        bounds: DomainBounds::Numeric {
            min: center - half,
            max: center + half,
        },
        radius: extent.radius.clone(),
        ordered_discrete: extent.ordered_discrete,
    }
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

        let sharing = matches!(
            sharing_policy,
            UnitAspectSharingPolicy::ForbidSharedExpansion
        )
        .then(|| child_frame_domain_sharing_levels_for_plot(self));

        constraints
            .iter()
            .map(|constraint| apply_unit_aspect_constraint(scales, constraint, sharing.as_ref()))
            .collect()
    }
}

fn apply_unit_aspect_constraint(
    scales: &mut HashMap<String, ConfiguredScaleWithSpec>,
    constraint: &ResolvedUnitAspectConstraint,
    sharing: Option<&HashMap<String, avenger_chart_core::DomainCoordination>>,
) -> Result<UnitAspectAdjustment, AvengerChartError> {
    if constraint.policy != UnitAspectPolicy::ExpandDomain {
        return Err(AvengerChartError::InvalidArgument(
            "unit_aspect supports expand-domain policy".to_string(),
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

    reject_forbidden_shared_axis_expansion(constraint, adjusted_axis, sharing)?;

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

fn reject_forbidden_shared_axis_expansion(
    constraint: &ResolvedUnitAspectConstraint,
    adjusted_axis: UnitAspectAdjustedAxis,
    sharing: Option<&HashMap<String, avenger_chart_core::DomainCoordination>>,
) -> Result<(), AvengerChartError> {
    let Some(sharing) = sharing else {
        return Ok(());
    };
    let (axis, scale_name) = match adjusted_axis {
        UnitAspectAdjustedAxis::X => ("x", constraint.x_scale.as_str()),
        UnitAspectAdjustedAxis::Y => ("y", constraint.y_scale.as_str()),
        UnitAspectAdjustedAxis::None => return Ok(()),
    };
    let Some(coordination) = sharing.get(scale_name) else {
        return Ok(());
    };
    if SharingLevel::from(coordination.scope).is_free() {
        return Ok(());
    }
    Err(AvengerChartError::InvalidArgument(format!(
        "unit_aspect scale '{scale_name}' ({axis} channel) would expand a non-free shared \
         domain; shared unit_aspect domains require the sharing-aware domain solver"
    )))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plot::compiled::CoordinationKind;

    fn free(cell: &str, scale: &str) -> UnitAspectDomainNode {
        UnitAspectDomainNode::Free {
            cell: cell.to_string(),
            scale: scale.to_string(),
        }
    }

    fn shared(group: &str) -> UnitAspectDomainNode {
        UnitAspectDomainNode::Shared(CoordinationScopeKey::container_group(
            CoordinationKind::ScaleDomain,
            0,
            group,
        ))
    }

    fn extent(span: f64) -> DomainExtent {
        DomainExtent::numeric(0.0, span)
    }

    fn graph_input(
        x_node: UnitAspectDomainNode,
        y_node: UnitAspectDomainNode,
        x_span: f64,
        y_span: f64,
        x_range_span: f64,
        y_range_span: f64,
    ) -> UnitAspectSpanGraphInput {
        UnitAspectSpanGraphInput {
            x_node,
            y_node,
            x_extent: extent(x_span),
            y_extent: extent(y_span),
            x_range_span,
            y_range_span,
            ratio: 1.0,
        }
    }

    fn numeric_span(extent: &DomainExtent) -> f64 {
        let DomainBounds::Numeric { min, max } = &extent.bounds else {
            panic!("expected numeric extent");
        };
        *max - *min
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-6,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn span_graph_solves_free_free_component() {
        let x = free("cell0", "x");
        let y = free("cell0", "y");
        let solved = solve_unit_aspect_span_graph(&[graph_input(
            x.clone(),
            y.clone(),
            10.0,
            10.0,
            200.0,
            100.0,
        )])
        .expect("solve");

        assert_close(numeric_span(&solved[&x]), 20.0);
        assert_close(numeric_span(&solved[&y]), 10.0);
    }

    #[test]
    fn span_graph_solves_shared_free_component() {
        let shared_x = shared("x");
        let y0 = free("cell0", "y");
        let y1 = free("cell1", "y");
        let solved = solve_unit_aspect_span_graph(&[
            graph_input(shared_x.clone(), y0.clone(), 10.0, 10.0, 200.0, 100.0),
            graph_input(shared_x.clone(), y1.clone(), 8.0, 5.0, 200.0, 100.0),
        ])
        .expect("solve");

        assert_close(numeric_span(&solved[&shared_x]), 20.0);
        assert_close(numeric_span(&solved[&y0]), 10.0);
        assert_close(numeric_span(&solved[&y1]), 10.0);
    }

    #[test]
    fn span_graph_solves_free_shared_component() {
        let x0 = free("cell0", "x");
        let x1 = free("cell1", "x");
        let shared_y = shared("y");
        let solved = solve_unit_aspect_span_graph(&[
            graph_input(x0.clone(), shared_y.clone(), 10.0, 10.0, 200.0, 100.0),
            graph_input(x1.clone(), shared_y.clone(), 5.0, 8.0, 200.0, 100.0),
        ])
        .expect("solve");

        assert_close(numeric_span(&solved[&x0]), 20.0);
        assert_close(numeric_span(&solved[&x1]), 20.0);
        assert_close(numeric_span(&solved[&shared_y]), 10.0);
    }

    #[test]
    fn span_graph_solves_shared_shared_component() {
        let x = shared("x");
        let y = shared("y");
        let solved = solve_unit_aspect_span_graph(&[graph_input(
            x.clone(),
            y.clone(),
            10.0,
            10.0,
            200.0,
            100.0,
        )])
        .expect("solve");

        assert_close(numeric_span(&solved[&x]), 20.0);
        assert_close(numeric_span(&solved[&y]), 10.0);
    }

    #[test]
    fn span_graph_solves_repeat_matrix_component() {
        let x_a = shared("x_a");
        let x_b = shared("x_b");
        let y_a = shared("y_a");
        let y_b = shared("y_b");
        let solved = solve_unit_aspect_span_graph(&[
            graph_input(x_a.clone(), y_a.clone(), 10.0, 10.0, 200.0, 100.0),
            graph_input(x_a.clone(), y_b.clone(), 10.0, 10.0, 200.0, 100.0),
            graph_input(x_b.clone(), y_a.clone(), 10.0, 10.0, 200.0, 100.0),
            graph_input(x_b.clone(), y_b.clone(), 10.0, 10.0, 200.0, 100.0),
        ])
        .expect("solve");

        assert_close(numeric_span(&solved[&x_a]), 20.0);
        assert_close(numeric_span(&solved[&x_b]), 20.0);
        assert_close(numeric_span(&solved[&y_a]), 10.0);
        assert_close(numeric_span(&solved[&y_b]), 10.0);
    }

    #[test]
    fn span_graph_rejects_inconsistent_cycle() {
        let x = shared("x");
        let y = shared("y");
        let err = solve_unit_aspect_span_graph(&[
            graph_input(x.clone(), y.clone(), 10.0, 10.0, 200.0, 100.0),
            graph_input(x, y, 10.0, 10.0, 400.0, 100.0),
        ])
        .expect_err("inconsistent equations");

        assert!(err.to_string().contains("inconsistent"));
    }

    #[test]
    fn span_graph_rejects_incompatible_same_domain_xy_node() {
        let node = shared("same");
        let err = solve_unit_aspect_span_graph(&[graph_input(
            node.clone(),
            node,
            10.0,
            10.0,
            200.0,
            100.0,
        )])
        .expect_err("same node incompatible");

        assert!(err.to_string().contains("same domain node"));
    }

    #[test]
    fn span_graph_allows_compatible_same_domain_xy_node() {
        let node = shared("same");
        let solved = solve_unit_aspect_span_graph(&[graph_input(
            node.clone(),
            node.clone(),
            10.0,
            10.0,
            100.0,
            100.0,
        )])
        .expect("same node compatible");

        assert_close(numeric_span(&solved[&node]), 10.0);
    }

    #[test]
    fn span_graph_preserves_radius_metadata() {
        let x = shared("x");
        let y = shared("y");
        let input = UnitAspectSpanGraphInput {
            x_node: x.clone(),
            y_node: y.clone(),
            x_extent: DomainExtent::numeric_with_radius(0.0, 10.0, 3.0, 4.0),
            y_extent: extent(10.0),
            x_range_span: 200.0,
            y_range_span: 100.0,
            ratio: 1.0,
        };
        let solved = solve_unit_aspect_span_graph(&[input]).expect("solve");

        assert_eq!(
            solved[&x]
                .radius
                .as_ref()
                .map(|radius| (radius.max_lower, radius.max_upper)),
            Some((3.0, 4.0))
        );
        assert_close(numeric_span(&solved[&x]), 20.0);
    }
}
