use std::collections::HashMap;

use avenger_chart_core::{
    AvengerChartError, CoordinateDomainBinding, CoordinateDomainCellKey,
    CoordinateDomainCellRequest, CoordinateDomainDescriptor, CoordinateDomainGroupRequest,
    CoordinateDomainMaterialization, CoordinateDomainNode, CoordinateDomainOwnership,
    CoordinateDomainResolvedState, CoordinateDomainScaleState, CoordinateDomainScaleType,
    CoordinateDomainSharingPolicy, DerivedScalarMap, DomainBounds, DomainExtent, SharingLevel,
};
use avenger_chart_scales::{ConfiguredScaleWithSpec, ScaleBuilder};
use avenger_scales::scales::{DomainKind, RangeKind};
use datafusion::common::ScalarValue;
use indexmap::IndexMap;

use super::{CompiledPlot, child_frame_domain_sharing_levels_for_plot};

const ROOT_COORDINATE_DOMAIN_CELL: &str = "root";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CoordinateDomainBuildPolicy {
    RejectSharedOverrides,
    AllowSharedOverrides,
}

impl CompiledPlot {
    pub(crate) fn coordinate_domain_descriptors(&self) -> Vec<CoordinateDomainDescriptor> {
        self.coord_transform
            .domain_provider()
            .map(|provider| provider.domain_descriptors())
            .unwrap_or_default()
    }

    pub(crate) fn materialize_coordinate_domain_builder(
        &self,
        builder: &ScaleBuilder,
        descriptors: &[CoordinateDomainDescriptor],
    ) -> Result<Option<ScaleBuilder>, AvengerChartError> {
        let mut materialized = None;
        for descriptor in descriptors {
            for binding in &descriptor.bindings {
                let CoordinateDomainMaterialization::CreateIfAbsent { scale_type } =
                    binding.materialize
                else {
                    continue;
                };
                let scale_name = effective_binding_scale_name(binding);
                if builder.channel_builders().contains_key(&scale_name)
                    || materialized.as_ref().is_some_and(|builder: &ScaleBuilder| {
                        builder.channel_builders().contains_key(&scale_name)
                    })
                {
                    continue;
                }

                let next = materialized.get_or_insert_with(|| builder.clone());
                match scale_type {
                    CoordinateDomainScaleType::LinearNumeric => {
                        next.ensure_linear_numeric_placeholder(
                            scale_name.clone(),
                            HashMap::new(),
                            DerivedScalarMap::new(),
                        );
                        next.apply_coordinate_default_options(
                            &scale_name,
                            self.coord_transform.as_ref(),
                        )?;
                    }
                }
            }
        }
        Ok(materialized)
    }

    pub(crate) fn coordinate_domain_channel_for_scale<'a>(
        &'a self,
        descriptors: &'a [CoordinateDomainDescriptor],
        scale_name: &str,
    ) -> Option<&'a str> {
        descriptors.iter().find_map(|descriptor| {
            descriptor.bindings.iter().find_map(|binding| {
                (effective_binding_scale_name(binding) == scale_name)
                    .then_some(binding.coord_channel.as_str())
            })
        })
    }

    pub(crate) fn apply_coordinate_domain_provider(
        &self,
        builder: &ScaleBuilder,
        scales: &mut HashMap<String, ConfiguredScaleWithSpec>,
        descriptors: &[CoordinateDomainDescriptor],
        plot_area_width: f32,
        plot_area_height: f32,
        params: &IndexMap<String, ScalarValue>,
        build_policy: CoordinateDomainBuildPolicy,
    ) -> Result<CoordinateDomainResolvedState, AvengerChartError> {
        let Some(provider) = self.coord_transform.domain_provider() else {
            return Ok(CoordinateDomainResolvedState::default());
        };
        if descriptors.is_empty() {
            return Ok(CoordinateDomainResolvedState::default());
        }

        let mut resolved = CoordinateDomainResolvedState::default();
        let cell_key = CoordinateDomainCellKey::new(ROOT_COORDINATE_DOMAIN_CELL);
        let forbid_shared_overrides = matches!(
            build_policy,
            CoordinateDomainBuildPolicy::RejectSharedOverrides
        );
        let domain_sharing =
            forbid_shared_overrides.then(|| child_frame_domain_sharing_levels_for_plot(self));
        for descriptor in descriptors {
            let scale_states = self.coordinate_domain_scale_states(
                builder,
                scales,
                descriptor,
                &cell_key,
                &HashMap::new(),
            )?;
            let cell = CoordinateDomainCellRequest {
                cell_key: &cell_key,
                plot_area_width,
                plot_area_height,
                params,
                scale_states: &scale_states,
            };
            let cells = [cell];
            let resolution = provider.resolve_domain_group(CoordinateDomainGroupRequest {
                descriptor_id: &descriptor.id,
                cells: &cells,
            })?;
            for cell_resolution in &resolution.cells {
                if let Some(domain_sharing) = domain_sharing.as_ref() {
                    reject_forbidden_shared_coordinate_domain_overrides(
                        descriptor,
                        &scale_states,
                        &cell_resolution.domain_overrides,
                        domain_sharing,
                    )?;
                }
                apply_coordinate_domain_overrides(scales, &cell_resolution.domain_overrides)?;
            }
            resolved.cells.extend(resolution.cells);
        }

        Ok(resolved)
    }

    pub(crate) fn coordinate_domain_scale_states(
        &self,
        builder: &ScaleBuilder,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
        descriptor: &CoordinateDomainDescriptor,
        cell_key: &CoordinateDomainCellKey,
        node_by_scale_name: &HashMap<String, CoordinateDomainNode>,
    ) -> Result<Vec<CoordinateDomainScaleState>, AvengerChartError> {
        let mut states = Vec::new();
        for binding in &descriptor.bindings {
            let scale_names = self.scale_names_for_coordinate_domain_binding(binding, scales);
            if scale_names.is_empty()
                && binding.ownership == CoordinateDomainOwnership::OwnsFinalDomain
            {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "coordinate domain descriptor '{}' owns channel '{}' but no scale was materialized",
                    descriptor.id, binding.coord_channel
                )));
            }

            for scale_name in scale_names {
                let Some(scale) = scales.get(&scale_name) else {
                    continue;
                };
                if let Some(required_scale_type) = binding.required_scale_type {
                    validate_coordinate_domain_scale_type(
                        &scale_name,
                        scale,
                        required_scale_type,
                        &binding.coord_channel,
                    )?;
                }
                let is_coordinate_domain_placeholder =
                    builder.channel_has_coordinate_domain_placeholder(&scale_name);
                let has_explicit_domain = builder.channel_has_explicit_domain(&scale_name)
                    && !is_coordinate_domain_placeholder;
                let raw_domain_param = builder
                    .channel_has_raw_domain(&scale_name)
                    .then(|| scale_name.clone());
                if binding.ownership == CoordinateDomainOwnership::OwnsFinalDomain {
                    if has_explicit_domain {
                        return Err(AvengerChartError::InvalidArgument(format!(
                            "coordinate-owned domain for scale '{scale_name}' cannot also use an explicit scale domain"
                        )));
                    }
                    if raw_domain_param.is_some() {
                        return Err(AvengerChartError::InvalidArgument(format!(
                            "coordinate-owned domain for scale '{scale_name}' cannot also use a raw_domain parameter"
                        )));
                    }
                }

                let node = node_by_scale_name
                    .get(&scale_name)
                    .cloned()
                    .unwrap_or_else(|| CoordinateDomainNode::Local {
                        cell_key: cell_key.clone(),
                        scale_name: scale_name.clone(),
                    });
                states.push(CoordinateDomainScaleState {
                    scale_name: scale_name.clone(),
                    coord_channel: binding.coord_channel.clone(),
                    role: binding.role.clone(),
                    base_domain: (!is_coordinate_domain_placeholder)
                        .then(|| domain_extent_for_scale(scale))
                        .flatten(),
                    range: numeric_range_for_scale(scale),
                    node,
                    has_explicit_domain,
                    raw_domain_param,
                });
            }
        }
        Ok(states)
    }

    pub(crate) fn scale_names_for_coordinate_domain_binding(
        &self,
        binding: &CoordinateDomainBinding,
        scales: &HashMap<String, ConfiguredScaleWithSpec>,
    ) -> Vec<String> {
        if let Some(scale_name) = &binding.scale_name {
            return scales
                .contains_key(scale_name)
                .then(|| scale_name.clone())
                .into_iter()
                .collect();
        }

        let mut names = self.scale_names_for_coord_channel(&binding.coord_channel);
        if names.is_empty() && scales.contains_key(&binding.coord_channel) {
            names.push(binding.coord_channel.clone());
        }
        names
    }
}

pub(crate) fn effective_binding_scale_name(binding: &CoordinateDomainBinding) -> String {
    binding
        .scale_name
        .clone()
        .unwrap_or_else(|| binding.coord_channel.clone())
}

fn domain_extent_for_scale(scale: &ConfiguredScaleWithSpec) -> Option<DomainExtent> {
    scale
        .configured()
        .numeric_interval_domain()
        .ok()
        .map(|(min, max)| DomainExtent::numeric(f64::from(min), f64::from(max)))
}

fn numeric_range_for_scale(scale: &ConfiguredScaleWithSpec) -> Option<(f64, f64)> {
    scale
        .configured()
        .numeric_interval_range()
        .ok()
        .map(|(min, max)| (f64::from(min), f64::from(max)))
}

fn validate_coordinate_domain_scale_type(
    scale_name: &str,
    scale: &ConfiguredScaleWithSpec,
    required: CoordinateDomainScaleType,
    coord_channel: &str,
) -> Result<(), AvengerChartError> {
    match required {
        CoordinateDomainScaleType::LinearNumeric => {
            let scale_impl = scale.configured().scale_impl.as_ref();
            if scale_impl.scale_type() != "linear"
                || scale_impl.domain_kind() != DomainKind::Numeric
                || scale_impl.range_kind() != RangeKind::Continuous
            {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "coordinate domain channel '{coord_channel}' scale '{scale_name}' requires a continuous linear numeric scale, got {}",
                    scale_impl.scale_type()
                )));
            }
        }
    }
    Ok(())
}

fn reject_forbidden_shared_coordinate_domain_overrides(
    descriptor: &CoordinateDomainDescriptor,
    scale_states: &[CoordinateDomainScaleState],
    overrides: &HashMap<String, DomainExtent>,
    domain_sharing: &HashMap<String, avenger_chart_core::DomainCoordination>,
) -> Result<(), AvengerChartError> {
    if descriptor.sharing_policy == CoordinateDomainSharingPolicy::LocalOnly {
        return Ok(());
    }

    for (scale_name, override_extent) in overrides {
        let Some(state) = scale_states
            .iter()
            .find(|state| state.scale_name == *scale_name)
        else {
            continue;
        };
        if !coordinate_domain_override_changes_base(state.base_domain.as_ref(), override_extent) {
            continue;
        }
        let Some(coordination) = domain_sharing.get(scale_name) else {
            continue;
        };
        if SharingLevel::from(coordination.scope).is_free() {
            continue;
        }
        let shared_label = if descriptor.id.contains("unit_aspect") {
            "shared unit_aspect domains"
        } else {
            "shared coordinate domains"
        };
        return Err(AvengerChartError::InvalidArgument(format!(
            "coordinate domain descriptor '{}' would override shared scale '{}'; {shared_label} require the sharing-aware domain solver",
            descriptor.id, scale_name
        )));
    }

    Ok(())
}

fn coordinate_domain_override_changes_base(
    base: Option<&DomainExtent>,
    override_extent: &DomainExtent,
) -> bool {
    const EPSILON: f64 = 1e-9;
    let Some(base) = base else {
        return true;
    };
    match (&base.bounds, &override_extent.bounds) {
        (
            DomainBounds::Numeric {
                min: base_min,
                max: base_max,
            },
            DomainBounds::Numeric {
                min: override_min,
                max: override_max,
            },
        ) => (base_min - override_min).abs() > EPSILON || (base_max - override_max).abs() > EPSILON,
        _ => base != override_extent,
    }
}

pub(crate) fn apply_coordinate_domain_overrides(
    scales: &mut HashMap<String, ConfiguredScaleWithSpec>,
    overrides: &HashMap<String, DomainExtent>,
) -> Result<(), AvengerChartError> {
    for (scale_name, extent) in overrides {
        let DomainBounds::Numeric { min, max } = &extent.bounds else {
            return Err(AvengerChartError::InvalidArgument(format!(
                "coordinate domain override for scale '{scale_name}' requires numeric bounds"
            )));
        };
        let scale = scales.get_mut(scale_name).ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "coordinate domain override references missing scale '{scale_name}'"
            ))
        })?;
        // Full f64 precision: coordinate-owned domains (e.g. Geo viewports)
        // originate as f64, and view-domain params read them back via the
        // f64 accessor. Scale math rounds to f32 at read either way.
        scale.set_configured(
            scale
                .configured()
                .clone()
                .with_domain_interval_f64((*min, *max)),
        );
    }
    Ok(())
}
