use std::collections::HashMap;

use avenger_chart_core::{
    DomainCoordination, NestScope, NestedBandSpec, SharingLevel, scalar_total_cmp,
};
use avenger_chart_scales::domain_extent::{
    DomainBounds, DomainExtent, SerializableDomainValue, SerializableStructField,
};
use datafusion::common::ScalarValue;

use crate::{
    coords::CellDomainInfo,
    facet::{coord::ChannelDomainExtent, sharing_policy},
    plot::compiled::{
        ChildFrameDomainRequest, CoordinationKind, CoordinationScopeKey, aggregate_domain_requests,
        apply_domain_group_to_key,
    },
};

pub(crate) fn aggregate_domain_extents(
    infos: &[CellDomainInfo],
) -> HashMap<CoordinationScopeKey, DomainExtent> {
    aggregate_domain_requests(infos.iter().map(domain_request_for_info))
}

fn domain_request_for_info(info: &CellDomainInfo) -> ChildFrameDomainRequest {
    ChildFrameDomainRequest::new(
        domain_coordination_scope_key_with_owner(
            &info.channel,
            &info.full_cell_path,
            SharingLevel::from_raw(info.domain_sharing_level),
            &info.domain_coordination,
            info.facet_depth,
            info.owner_path.as_deref(),
        ),
        info.extent.clone(),
    )
}

#[cfg(test)]
pub(crate) fn domain_infos_for_cell(
    full_cell_path: &[ScalarValue],
    local_domain_extents: &HashMap<String, ChannelDomainExtent>,
    facet_depth: u8,
) -> Vec<CellDomainInfo> {
    local_domain_extents
        .iter()
        .map(|(channel, annotated)| CellDomainInfo {
            full_cell_path: full_cell_path.to_vec(),
            channel: channel.clone(),
            domain_sharing_level: annotated.domain_sharing_level.raw(),
            domain_coordination: annotated.domain_coordination.clone(),
            facet_depth,
            owner_path: None,
            extent: annotated.extent.clone(),
        })
        .collect()
}

pub(crate) fn domain_infos_for_cell_with_owner_paths(
    full_cell_path: &[ScalarValue],
    local_domain_extents: &HashMap<String, ChannelDomainExtent>,
    facet_depth: u8,
    owner_path_for: &dyn Fn(SharingLevel) -> Vec<ScalarValue>,
) -> Vec<CellDomainInfo> {
    local_domain_extents
        .iter()
        .map(|(channel, annotated)| CellDomainInfo {
            full_cell_path: full_cell_path.to_vec(),
            channel: channel.clone(),
            domain_sharing_level: annotated.domain_sharing_level.raw(),
            domain_coordination: annotated.domain_coordination.clone(),
            facet_depth,
            owner_path: Some(owner_path_for(annotated.domain_sharing_level)),
            extent: annotated.extent.clone(),
        })
        .collect()
}

#[cfg(test)]
pub(crate) fn coordinated_extents_for_cell(
    full_cell_path: &[ScalarValue],
    local_domain_extents: &HashMap<String, ChannelDomainExtent>,
    channel_domain_coordinations: &HashMap<String, DomainCoordination>,
    facet_depth: u8,
    unified: &HashMap<CoordinationScopeKey, DomainExtent>,
) -> HashMap<String, DomainExtent> {
    let mut coordinated = HashMap::new();

    for (channel, annotated) in local_domain_extents {
        if annotated.domain_sharing_level == 0 {
            continue;
        }

        let key = domain_coordination_scope_key(
            channel,
            full_cell_path,
            annotated.domain_sharing_level,
            &annotated.domain_coordination,
            facet_depth,
        );

        if let Some(unified_extent) = unified.get(&key) {
            coordinated.insert(channel.clone(), unified_extent.clone());
        }
    }

    // Data-empty cells can have no local domain extents even when
    // channel-domain sharing is enabled. Fill coordinated extents from saved
    // sharing levels so owner cells without local data still render shared plot
    // scale domains.
    for (channel, coordination) in channel_domain_coordinations {
        if SharingLevel::from(coordination.scope).is_free() || coordinated.contains_key(channel) {
            continue;
        }

        let key = domain_coordination_scope_key(
            channel,
            full_cell_path,
            SharingLevel::from(coordination.scope),
            coordination,
            facet_depth,
        );

        if let Some(unified_extent) = unified.get(&key) {
            coordinated.insert(channel.clone(), unified_extent.clone());
        }
    }

    coordinated
}

pub(crate) fn coordinated_extents_for_cell_with_owner_paths(
    full_cell_path: &[ScalarValue],
    local_domain_extents: &HashMap<String, ChannelDomainExtent>,
    channel_domain_coordinations: &HashMap<String, DomainCoordination>,
    facet_depth: u8,
    unified: &HashMap<CoordinationScopeKey, DomainExtent>,
    owner_path_for: &dyn Fn(SharingLevel) -> Vec<ScalarValue>,
) -> HashMap<String, DomainExtent> {
    let mut coordinated = HashMap::new();

    for (channel, annotated) in local_domain_extents {
        if annotated.domain_sharing_level == 0 {
            continue;
        }

        let owner_path = owner_path_for(annotated.domain_sharing_level);
        let key = domain_coordination_scope_key_with_owner(
            channel,
            full_cell_path,
            annotated.domain_sharing_level,
            &annotated.domain_coordination,
            facet_depth,
            Some(&owner_path),
        );

        if let Some(unified_extent) = unified.get(&key) {
            coordinated.insert(channel.clone(), unified_extent.clone());
        }
    }

    for (channel, coordination) in channel_domain_coordinations {
        let sharing_level = SharingLevel::from(coordination.scope);
        if sharing_level.is_free() || coordinated.contains_key(channel) {
            continue;
        }

        let owner_path = owner_path_for(sharing_level);
        let key = domain_coordination_scope_key_with_owner(
            channel,
            full_cell_path,
            sharing_level,
            coordination,
            facet_depth,
            Some(&owner_path),
        );

        if let Some(unified_extent) = unified.get(&key) {
            coordinated.insert(channel.clone(), unified_extent.clone());
        }
    }

    coordinated
}

pub(crate) fn coordinated_nested_extent_for_cell(
    full_cell_path: &[ScalarValue],
    channel: &str,
    config: &NestedBandSpec,
    domain_infos: &[CellDomainInfo],
    owner_path_for: &dyn Fn(&[ScalarValue], SharingLevel) -> Vec<ScalarValue>,
) -> Option<DomainExtent> {
    if !has_nested_level_coordination(config) {
        return None;
    }

    let paths = nested_domain_paths_for_channel(channel, domain_infos);
    let field_names = paths.first()?.field_names.clone();
    let depth = field_names.len();
    if depth == 0 {
        return None;
    }

    let mut component_paths = Vec::new();
    collect_nested_component_paths_for_cell(
        &mut Vec::new(),
        0,
        depth,
        full_cell_path,
        config,
        &paths,
        owner_path_for,
        &mut component_paths,
    );
    if component_paths.is_empty() {
        return None;
    }

    let ordered = paths.iter().any(|path| path.ordered);
    let mut values = component_paths
        .into_iter()
        .map(|components| {
            SerializableDomainValue::Struct(
                field_names
                    .iter()
                    .cloned()
                    .zip(components)
                    .map(|(name, value)| SerializableStructField {
                        name,
                        value: Box::new(value),
                    })
                    .collect(),
            )
        })
        .collect::<Vec<_>>();

    if ordered {
        dedup_serializable_domain_values(&mut values);
        Some(DomainExtent::ordered_discrete(values))
    } else {
        values.sort_by(|lhs, rhs| scalar_total_cmp(&lhs.to_scalar(), &rhs.to_scalar()));
        dedup_serializable_domain_values(&mut values);
        Some(DomainExtent::discrete(values))
    }
}

fn has_nested_level_coordination(config: &NestedBandSpec) -> bool {
    config
        .levels
        .values()
        .any(|level| level.domain_coordination.is_some())
}

#[derive(Clone, Debug)]
struct NestedDomainPath {
    full_cell_path: Vec<ScalarValue>,
    field_names: Vec<String>,
    components: Vec<SerializableDomainValue>,
    ordered: bool,
}

fn nested_domain_paths_for_channel(
    channel: &str,
    domain_infos: &[CellDomainInfo],
) -> Vec<NestedDomainPath> {
    let mut paths = Vec::new();
    for info in domain_infos {
        if info.channel != channel {
            continue;
        }

        let DomainBounds::Discrete(values) = &info.extent.bounds else {
            continue;
        };

        for value in values {
            let SerializableDomainValue::Struct(fields) = value else {
                continue;
            };
            paths.push(NestedDomainPath {
                full_cell_path: info.full_cell_path.clone(),
                field_names: fields.iter().map(|field| field.name.clone()).collect(),
                components: fields
                    .iter()
                    .map(|field| field.value.as_ref().clone())
                    .collect(),
                ordered: info.extent.ordered_discrete,
            });
        }
    }
    paths
}

#[allow(clippy::too_many_arguments)]
fn collect_nested_component_paths_for_cell(
    prefix: &mut Vec<SerializableDomainValue>,
    level: usize,
    depth: usize,
    full_cell_path: &[ScalarValue],
    config: &NestedBandSpec,
    paths: &[NestedDomainPath],
    owner_path_for: &dyn Fn(&[ScalarValue], SharingLevel) -> Vec<ScalarValue>,
    output: &mut Vec<Vec<SerializableDomainValue>>,
) {
    if level == depth {
        output.push(prefix.clone());
        return;
    }

    for component in nested_level_component_candidates_for_cell(
        prefix,
        level,
        full_cell_path,
        config,
        paths,
        owner_path_for,
    ) {
        prefix.push(component);
        collect_nested_component_paths_for_cell(
            prefix,
            level + 1,
            depth,
            full_cell_path,
            config,
            paths,
            owner_path_for,
            output,
        );
        prefix.pop();
    }
}

fn nested_level_component_candidates_for_cell(
    prefix: &[SerializableDomainValue],
    level: usize,
    full_cell_path: &[ScalarValue],
    config: &NestedBandSpec,
    paths: &[NestedDomainPath],
    owner_path_for: &dyn Fn(&[ScalarValue], SharingLevel) -> Vec<ScalarValue>,
) -> Vec<SerializableDomainValue> {
    let level_config = config.level(level);
    let domain_sharing_level = level_config
        .and_then(|level| level.domain_coordination.as_ref())
        .map(|coordination| SharingLevel::from(coordination.scope));
    let nest_scope = level_config
        .and_then(|level| level.nest_scope)
        .unwrap_or(NestScope::Free);
    let collect_globally = level > 0 && nest_scope == NestScope::Shared;
    let target_owner_path =
        domain_sharing_level.map(|sharing| owner_path_for(full_cell_path, sharing));

    let mut candidates = Vec::new();
    for path in paths {
        if path.components.len() <= level {
            continue;
        }

        match (domain_sharing_level, target_owner_path.as_ref()) {
            (Some(sharing), Some(target_owner_path)) => {
                if owner_path_for(&path.full_cell_path, sharing) != *target_owner_path {
                    continue;
                }
            }
            _ if path.full_cell_path != full_cell_path => continue,
            _ => {}
        }

        if !collect_globally && !serializable_prefix_matches(&path.components, prefix) {
            continue;
        }

        push_unique_serializable_value(&mut candidates, path.components[level].clone());
    }

    candidates
}

fn serializable_prefix_matches(
    components: &[SerializableDomainValue],
    prefix: &[SerializableDomainValue],
) -> bool {
    components.len() >= prefix.len()
        && components
            .iter()
            .zip(prefix)
            .all(|(component, expected)| component == expected)
}

fn push_unique_serializable_value(
    target: &mut Vec<SerializableDomainValue>,
    value: SerializableDomainValue,
) {
    if !target.iter().any(|existing| existing == &value) {
        target.push(value);
    }
}

fn dedup_serializable_domain_values(values: &mut Vec<SerializableDomainValue>) {
    let mut unique = Vec::with_capacity(values.len());
    for value in values.drain(..) {
        push_unique_serializable_value(&mut unique, value);
    }
    *values = unique;
}

#[cfg(test)]
pub(crate) fn domain_coordination_scope_key(
    channel: &str,
    full_cell_path: &[ScalarValue],
    sharing_level: SharingLevel,
    coordination: &DomainCoordination,
    facet_depth: u8,
) -> CoordinationScopeKey {
    domain_coordination_scope_key_with_owner(
        channel,
        full_cell_path,
        sharing_level,
        coordination,
        facet_depth,
        None,
    )
}

pub(crate) fn domain_coordination_scope_key_with_owner(
    channel: &str,
    full_cell_path: &[ScalarValue],
    sharing_level: SharingLevel,
    coordination: &DomainCoordination,
    facet_depth: u8,
    owner_path: Option<&[ScalarValue]>,
) -> CoordinationScopeKey {
    let ancestor_key = owner_path.map(ToOwned::to_owned).unwrap_or_else(|| {
        sharing_policy::domain_group_key(full_cell_path, sharing_level, facet_depth)
    });
    let key = CoordinationScopeKey::partition_path(CoordinationKind::ScaleDomain, ancestor_key);
    apply_domain_group_to_key(key, channel, coordination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_chart_scales::domain_extent::{
        DomainBounds, DomainExtent, SerializableDomainValue,
    };

    fn s(value: &str) -> ScalarValue {
        ScalarValue::Utf8(Some(value.to_string()))
    }

    fn scale_name_coordination(sharing: u8) -> DomainCoordination {
        DomainCoordination::scale_name(SharingLevel::from_raw(sharing).into())
    }

    fn named_coordination(sharing: u8, group: &str) -> DomainCoordination {
        DomainCoordination::named(SharingLevel::from_raw(sharing).into(), group).unwrap()
    }

    fn numeric_info(
        path: Vec<ScalarValue>,
        channel: &str,
        sharing: u8,
        max: f64,
    ) -> CellDomainInfo {
        let facet_depth = path.len() as u8;
        CellDomainInfo {
            full_cell_path: path,
            channel: channel.to_string(),
            domain_sharing_level: sharing,
            domain_coordination: scale_name_coordination(sharing),
            facet_depth,
            owner_path: None,
            extent: DomainExtent::numeric(0.0, max),
        }
    }

    fn nested_domain_value(parts: &[(&str, &str)]) -> SerializableDomainValue {
        SerializableDomainValue::Struct(
            parts
                .iter()
                .map(|(name, value)| SerializableStructField {
                    name: (*name).to_string(),
                    value: Box::new(SerializableDomainValue::String((*value).to_string())),
                })
                .collect(),
        )
    }

    fn nested_domain_info(
        cell: &str,
        channel: &str,
        values: Vec<SerializableDomainValue>,
    ) -> CellDomainInfo {
        CellDomainInfo {
            full_cell_path: vec![s(cell)],
            channel: channel.to_string(),
            domain_sharing_level: SharingLevel::FREE.raw(),
            domain_coordination: scale_name_coordination(SharingLevel::FREE.raw()),
            facet_depth: 1,
            owner_path: Some(vec![s(cell)]),
            extent: DomainExtent::discrete(values),
        }
    }

    fn nested_ordered_domain_info(
        cell: &str,
        channel: &str,
        values: Vec<SerializableDomainValue>,
    ) -> CellDomainInfo {
        let mut info = nested_domain_info(cell, channel, values);
        let DomainBounds::Discrete(values) = info.extent.bounds else {
            panic!("expected discrete extent");
        };
        info.extent = DomainExtent::ordered_discrete(values);
        info
    }

    fn nested_config(levels: &[(usize, u8, Option<NestScope>)]) -> NestedBandSpec {
        let mut config = NestedBandSpec::default();
        for (level, sharing, nest_scope) in levels {
            let level_config = config.level_mut(*level);
            level_config.domain_coordination = Some(scale_name_coordination(*sharing));
            level_config.nest_scope = *nest_scope;
        }
        config
    }

    fn owner_path_for_test(path: &[ScalarValue], sharing: SharingLevel) -> Vec<ScalarValue> {
        sharing_policy::domain_group_key(path, sharing, path.len() as u8)
    }

    fn nested_extent_labels(extent: &DomainExtent) -> Vec<Vec<String>> {
        let DomainBounds::Discrete(values) = &extent.bounds else {
            panic!("expected discrete extent");
        };
        values
            .iter()
            .map(|value| {
                let SerializableDomainValue::Struct(fields) = value else {
                    panic!("expected struct value");
                };
                fields
                    .iter()
                    .map(|field| match field.value.as_ref() {
                        SerializableDomainValue::String(value) => value.clone(),
                        other => format!("{other:?}"),
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn aggregate_domain_extents_groups_by_channel_and_sharing_key() {
        let info_a = CellDomainInfo {
            full_cell_path: vec![s("A"), s("B"), s("X")],
            channel: "x".to_string(),
            domain_sharing_level: 1,
            domain_coordination: scale_name_coordination(1),
            facet_depth: 3,
            owner_path: None,
            extent: DomainExtent::numeric(0.0, 1.0),
        };
        let info_b = CellDomainInfo {
            full_cell_path: vec![s("A"), s("B"), s("Y")],
            channel: "x".to_string(),
            domain_sharing_level: 1,
            domain_coordination: scale_name_coordination(1),
            facet_depth: 3,
            owner_path: None,
            extent: DomainExtent::numeric(0.0, 2.0),
        };
        let info_c = CellDomainInfo {
            full_cell_path: vec![s("A"), s("Z"), s("Q")],
            channel: "x".to_string(),
            domain_sharing_level: 1,
            domain_coordination: scale_name_coordination(1),
            facet_depth: 3,
            owner_path: None,
            extent: DomainExtent::numeric(5.0, 7.0),
        };

        let aggregated =
            aggregate_domain_extents(&[info_a.clone(), info_b.clone(), info_c.clone()]);
        let key_ab = domain_coordination_scope_key(
            &info_a.channel,
            &info_a.full_cell_path,
            SharingLevel::from_raw(info_a.domain_sharing_level),
            &info_a.domain_coordination,
            info_a.facet_depth,
        );
        let key_c = domain_coordination_scope_key(
            &info_c.channel,
            &info_c.full_cell_path,
            SharingLevel::from_raw(info_c.domain_sharing_level),
            &info_c.domain_coordination,
            info_c.facet_depth,
        );

        assert_eq!(aggregated.len(), 2);
        assert!(aggregated.contains_key(&key_ab));
        assert!(aggregated.contains_key(&key_c));
    }

    #[test]
    fn domain_scope_keys_separate_channels() {
        let coordination = scale_name_coordination(1);
        let x_key = domain_coordination_scope_key(
            "x",
            &[s("A"), s("B")],
            SharingLevel::from_raw(1),
            &coordination,
            2,
        );
        let y_key = domain_coordination_scope_key(
            "y",
            &[s("A"), s("B")],
            SharingLevel::from_raw(1),
            &coordination,
            2,
        );

        assert_ne!(x_key, y_key);
    }

    #[test]
    fn named_domain_scope_keys_group_different_channels() {
        let coordination = named_coordination(1, "height");
        let x_key = domain_coordination_scope_key(
            "x",
            &[s("A"), s("B")],
            SharingLevel::from_raw(1),
            &coordination,
            2,
        );
        let y_key = domain_coordination_scope_key(
            "y",
            &[s("A"), s("B")],
            SharingLevel::from_raw(1),
            &coordination,
            2,
        );

        assert_eq!(x_key, y_key);
    }

    #[test]
    fn named_domain_scope_keys_stay_scoped_to_outer_facet() {
        let coordination = named_coordination(1, "height");
        let east_x = domain_coordination_scope_key(
            "x",
            &[s("East"), s("A")],
            SharingLevel::from_raw(1),
            &coordination,
            2,
        );
        let east_y = domain_coordination_scope_key(
            "y",
            &[s("East"), s("B")],
            SharingLevel::from_raw(1),
            &coordination,
            2,
        );
        let west_x = domain_coordination_scope_key(
            "x",
            &[s("West"), s("A")],
            SharingLevel::from_raw(1),
            &coordination,
            2,
        );

        assert_eq!(east_x, east_y);
        assert_ne!(east_x, west_x);
    }

    #[test]
    fn owner_paths_make_shared_domain_keys_stable_across_child_cells() {
        let coordination = scale_name_coordination(SharingLevel::GLOBAL.raw());
        let shared_owner = vec![s("global-owner")];
        let left_key = domain_coordination_scope_key_with_owner(
            "x",
            &[s("north"), s("leaf_a")],
            SharingLevel::GLOBAL,
            &coordination,
            2,
            Some(&shared_owner),
        );
        let right_key = domain_coordination_scope_key_with_owner(
            "x",
            &[s("south"), s("leaf_b")],
            SharingLevel::GLOBAL,
            &coordination,
            2,
            Some(&shared_owner),
        );
        let other_owner_key = domain_coordination_scope_key_with_owner(
            "x",
            &[s("south"), s("leaf_b")],
            SharingLevel::GLOBAL,
            &coordination,
            2,
            Some(&[s("other-owner")]),
        );

        assert_eq!(left_key, right_key);
        assert_ne!(left_key, other_owner_key);
    }

    #[test]
    fn aggregate_named_domain_extents_groups_different_channels() {
        let mut x_info = numeric_info(vec![s("A")], "x", SharingLevel::GLOBAL.raw(), 2.0);
        x_info.domain_coordination = named_coordination(SharingLevel::GLOBAL.raw(), "height");
        let mut y_info = numeric_info(vec![s("B")], "y", SharingLevel::GLOBAL.raw(), 101.0);
        y_info.domain_coordination = named_coordination(SharingLevel::GLOBAL.raw(), "height");

        let aggregated = aggregate_domain_extents(&[x_info.clone(), y_info.clone()]);
        let x_key = domain_coordination_scope_key(
            "x",
            &x_info.full_cell_path,
            SharingLevel::GLOBAL,
            &x_info.domain_coordination,
            x_info.facet_depth,
        );
        let y_key = domain_coordination_scope_key(
            "y",
            &y_info.full_cell_path,
            SharingLevel::GLOBAL,
            &y_info.domain_coordination,
            y_info.facet_depth,
        );

        assert_eq!(x_key, y_key);
        assert_eq!(aggregated.len(), 1);
        assert_eq!(
            aggregated.get(&x_key),
            Some(&DomainExtent::numeric(0.0, 101.0))
        );
    }

    #[test]
    fn level_domain_scope_groups_by_ancestor_prefix() {
        let info_ab1 = numeric_info(vec![s("A"), s("B1")], "x", 1, 1.0);
        let info_ab2 = numeric_info(vec![s("A"), s("B2")], "x", 1, 2.0);
        let info_cb = numeric_info(vec![s("C"), s("B")], "x", 1, 3.0);

        let aggregated =
            aggregate_domain_extents(&[info_ab1.clone(), info_ab2.clone(), info_cb.clone()]);
        let a_key = domain_coordination_scope_key(
            "x",
            &info_ab1.full_cell_path,
            SharingLevel::from_raw(1),
            &info_ab1.domain_coordination,
            info_ab1.facet_depth,
        );
        let c_key = domain_coordination_scope_key(
            "x",
            &info_cb.full_cell_path,
            SharingLevel::from_raw(1),
            &info_cb.domain_coordination,
            info_cb.facet_depth,
        );

        assert_eq!(aggregated.len(), 2);
        assert_eq!(
            aggregated.get(&a_key),
            Some(&DomainExtent::numeric(0.0, 2.0))
        );
        assert_eq!(
            aggregated.get(&c_key),
            Some(&DomainExtent::numeric(0.0, 3.0))
        );
    }

    #[test]
    fn free_domain_scope_does_not_return_coordinated_extent_for_cell() {
        let mut local = HashMap::new();
        local.insert(
            "x".to_string(),
            ChannelDomainExtent {
                extent: DomainExtent::numeric(0.0, 1.0),
                domain_sharing_level: SharingLevel::from_raw(0),
                domain_coordination: scale_name_coordination(0),
            },
        );
        let unified = aggregate_domain_extents(&domain_infos_for_cell(&[s("A")], &local, 1));

        let coordinated =
            coordinated_extents_for_cell(&[s("A")], &local, &HashMap::new(), 1, &unified);

        assert!(coordinated.is_empty());
    }

    #[test]
    fn empty_cell_uses_saved_domain_scope_level() {
        let info_a = numeric_info(vec![s("A")], "x", SharingLevel::GLOBAL.raw(), 1.0);
        let info_b = numeric_info(vec![s("B")], "x", SharingLevel::GLOBAL.raw(), 2.0);
        let unified = aggregate_domain_extents(&[info_a, info_b]);
        let channel_levels = HashMap::from([(
            "x".to_string(),
            scale_name_coordination(SharingLevel::GLOBAL.raw()),
        )]);

        let coordinated =
            coordinated_extents_for_cell(&[s("C")], &HashMap::new(), &channel_levels, 1, &unified);

        assert_eq!(coordinated.get("x"), Some(&DomainExtent::numeric(0.0, 2.0)));
    }

    #[test]
    fn shared_discrete_domains_are_sorted_after_union() {
        let info_a = CellDomainInfo {
            full_cell_path: vec![s("col_a")],
            channel: "x".to_string(),
            domain_sharing_level: 255,
            domain_coordination: scale_name_coordination(255),
            facet_depth: 1,
            owner_path: None,
            extent: DomainExtent::discrete(vec![
                SerializableDomainValue::String("A".to_string()),
                SerializableDomainValue::String("D".to_string()),
            ]),
        };
        let info_b = CellDomainInfo {
            full_cell_path: vec![s("col_b")],
            channel: "x".to_string(),
            domain_sharing_level: 255,
            domain_coordination: scale_name_coordination(255),
            facet_depth: 1,
            owner_path: None,
            extent: DomainExtent::discrete(vec![
                SerializableDomainValue::String("B".to_string()),
                SerializableDomainValue::String("C".to_string()),
            ]),
        };

        let aggregated = aggregate_domain_extents(&[info_a.clone(), info_b.clone()]);
        let extent = aggregated
            .values()
            .next()
            .expect("expected one shared x domain");
        let DomainBounds::Discrete(values) = &extent.bounds else {
            panic!("expected discrete extent");
        };

        assert_eq!(
            values,
            &vec![
                SerializableDomainValue::String("A".to_string()),
                SerializableDomainValue::String("B".to_string()),
                SerializableDomainValue::String("C".to_string()),
                SerializableDomainValue::String("D".to_string()),
            ]
        );

        let aggregated_reversed = aggregate_domain_extents(&[info_b, info_a]);
        let reversed_extent = aggregated_reversed
            .values()
            .next()
            .expect("expected one shared x domain");
        let DomainBounds::Discrete(reversed_values) = &reversed_extent.bounds else {
            panic!("expected discrete extent");
        };

        assert_eq!(reversed_values, values);
    }

    #[test]
    fn nested_shared_parent_free_leaf_shares_children_per_parent() {
        let infos = vec![
            nested_domain_info(
                "north",
                "x",
                vec![
                    nested_domain_value(&[("cyl", "4"), ("make", "ford")]),
                    nested_domain_value(&[("cyl", "6"), ("make", "amc")]),
                ],
            ),
            nested_domain_info(
                "south",
                "x",
                vec![
                    nested_domain_value(&[("cyl", "4"), ("make", "toyota")]),
                    nested_domain_value(&[("cyl", "6"), ("make", "ford")]),
                ],
            ),
        ];
        let config = nested_config(&[
            (0, SharingLevel::GLOBAL.raw(), None),
            (1, SharingLevel::GLOBAL.raw(), Some(NestScope::Free)),
        ]);

        let extent = coordinated_nested_extent_for_cell(
            &[s("north")],
            "x",
            &config,
            &infos,
            &owner_path_for_test,
        )
        .expect("nested extent");

        assert_eq!(
            nested_extent_labels(&extent),
            vec![
                vec!["4".to_string(), "ford".to_string()],
                vec!["4".to_string(), "toyota".to_string()],
                vec!["6".to_string(), "amc".to_string()],
                vec!["6".to_string(), "ford".to_string()],
            ]
        );
    }

    #[test]
    fn ordered_nested_domains_use_shared_owner_order() {
        let infos = vec![
            nested_ordered_domain_info(
                "north",
                "x",
                vec![
                    nested_domain_value(&[("cyl", "6"), ("make", "ford")]),
                    nested_domain_value(&[("cyl", "4"), ("make", "toyota")]),
                ],
            ),
            nested_ordered_domain_info(
                "south",
                "x",
                vec![
                    nested_domain_value(&[("cyl", "4"), ("make", "amc")]),
                    nested_domain_value(&[("cyl", "6"), ("make", "bmw")]),
                ],
            ),
        ];
        let config = nested_config(&[
            (0, SharingLevel::GLOBAL.raw(), None),
            (1, SharingLevel::GLOBAL.raw(), Some(NestScope::Free)),
        ]);

        let extent = coordinated_nested_extent_for_cell(
            &[s("north")],
            "x",
            &config,
            &infos,
            &owner_path_for_test,
        )
        .expect("nested extent");

        assert!(extent.ordered_discrete);
        assert_eq!(
            nested_extent_labels(&extent),
            vec![
                vec!["6".to_string(), "ford".to_string()],
                vec!["6".to_string(), "bmw".to_string()],
                vec!["4".to_string(), "toyota".to_string()],
                vec!["4".to_string(), "amc".to_string()],
            ]
        );
    }

    #[test]
    fn nested_shared_parent_shared_leaf_crosses_children_under_each_parent() {
        let infos = vec![
            nested_domain_info(
                "north",
                "x",
                vec![
                    nested_domain_value(&[("cyl", "4"), ("make", "ford")]),
                    nested_domain_value(&[("cyl", "6"), ("make", "amc")]),
                ],
            ),
            nested_domain_info(
                "south",
                "x",
                vec![
                    nested_domain_value(&[("cyl", "4"), ("make", "toyota")]),
                    nested_domain_value(&[("cyl", "6"), ("make", "ford")]),
                ],
            ),
        ];
        let config = nested_config(&[
            (0, SharingLevel::GLOBAL.raw(), None),
            (1, SharingLevel::GLOBAL.raw(), Some(NestScope::Shared)),
        ]);

        let extent = coordinated_nested_extent_for_cell(
            &[s("north")],
            "x",
            &config,
            &infos,
            &owner_path_for_test,
        )
        .expect("nested extent");

        assert_eq!(
            nested_extent_labels(&extent),
            vec![
                vec!["4".to_string(), "amc".to_string()],
                vec!["4".to_string(), "ford".to_string()],
                vec!["4".to_string(), "toyota".to_string()],
                vec!["6".to_string(), "amc".to_string()],
                vec!["6".to_string(), "ford".to_string()],
                vec!["6".to_string(), "toyota".to_string()],
            ]
        );
    }

    #[test]
    fn nested_free_domains_stay_local_to_facet_cell() {
        let infos = vec![
            nested_domain_info(
                "north",
                "x",
                vec![nested_domain_value(&[("cyl", "4"), ("make", "ford")])],
            ),
            nested_domain_info(
                "south",
                "x",
                vec![nested_domain_value(&[("cyl", "6"), ("make", "toyota")])],
            ),
        ];
        let config = nested_config(&[
            (0, SharingLevel::FREE.raw(), None),
            (1, SharingLevel::FREE.raw(), Some(NestScope::Free)),
        ]);

        let extent = coordinated_nested_extent_for_cell(
            &[s("north")],
            "x",
            &config,
            &infos,
            &owner_path_for_test,
        )
        .expect("nested extent");

        assert_eq!(
            nested_extent_labels(&extent),
            vec![vec!["4".to_string(), "ford".to_string()]]
        );
    }
}
