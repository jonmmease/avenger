use std::collections::HashMap;

use datafusion::common::ScalarValue;

use crate::{
    coords::CellDomainInfo,
    facet::{coord::ChannelDomainExtent, sharing_policy},
    plot::compiled::{
        ChildFrameDomainRequest, CoordinationKind, CoordinationScopeKey, SharingLevel,
        aggregate_domain_requests,
    },
    scales::domain_extent::DomainExtent,
};

pub(crate) fn aggregate_domain_extents(
    infos: &[CellDomainInfo],
) -> HashMap<CoordinationScopeKey, DomainExtent> {
    aggregate_domain_requests(infos.iter().map(domain_request_for_info))
}

fn domain_request_for_info(info: &CellDomainInfo) -> ChildFrameDomainRequest {
    ChildFrameDomainRequest::new(
        domain_coordination_scope_key(
            &info.channel,
            &info.full_cell_path,
            SharingLevel::from_raw(info.domain_sharing_level),
            info.facet_depth,
        ),
        info.extent.clone(),
    )
}

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
            facet_depth,
            extent: annotated.extent.clone(),
        })
        .collect()
}

pub(crate) fn coordinated_extents_for_cell(
    full_cell_path: &[ScalarValue],
    local_domain_extents: &HashMap<String, ChannelDomainExtent>,
    channel_domain_sharing_levels: &HashMap<String, SharingLevel>,
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
    for (channel, sharing_level) in channel_domain_sharing_levels {
        if *sharing_level == 0 || coordinated.contains_key(channel) {
            continue;
        }

        let key =
            domain_coordination_scope_key(channel, full_cell_path, *sharing_level, facet_depth);

        if let Some(unified_extent) = unified.get(&key) {
            coordinated.insert(channel.clone(), unified_extent.clone());
        }
    }

    coordinated
}

pub(crate) fn domain_coordination_scope_key(
    channel: &str,
    full_cell_path: &[ScalarValue],
    sharing_level: SharingLevel,
    facet_depth: u8,
) -> CoordinationScopeKey {
    let ancestor_key = sharing_policy::domain_group_key(full_cell_path, sharing_level, facet_depth);
    CoordinationScopeKey::partition_path(CoordinationKind::ScaleDomain, ancestor_key)
        .with_channel(channel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scales::domain_extent::{DomainBounds, DomainExtent, SerializableDomainValue};

    fn s(value: &str) -> ScalarValue {
        ScalarValue::Utf8(Some(value.to_string()))
    }

    fn numeric_info(
        path: Vec<ScalarValue>,
        channel: &str,
        sharing: u8,
        max: f64,
    ) -> CellDomainInfo {
        CellDomainInfo {
            full_cell_path: path,
            channel: channel.to_string(),
            domain_sharing_level: sharing,
            facet_depth: 2,
            extent: DomainExtent::numeric(0.0, max),
        }
    }

    #[test]
    fn aggregate_domain_extents_groups_by_channel_and_sharing_key() {
        let info_a = CellDomainInfo {
            full_cell_path: vec![s("A"), s("B"), s("X")],
            channel: "x".to_string(),
            domain_sharing_level: 1,
            facet_depth: 3,
            extent: DomainExtent::numeric(0.0, 1.0),
        };
        let info_b = CellDomainInfo {
            full_cell_path: vec![s("A"), s("B"), s("Y")],
            channel: "x".to_string(),
            domain_sharing_level: 1,
            facet_depth: 3,
            extent: DomainExtent::numeric(0.0, 2.0),
        };
        let info_c = CellDomainInfo {
            full_cell_path: vec![s("A"), s("Z"), s("Q")],
            channel: "x".to_string(),
            domain_sharing_level: 1,
            facet_depth: 3,
            extent: DomainExtent::numeric(5.0, 7.0),
        };

        let aggregated =
            aggregate_domain_extents(&[info_a.clone(), info_b.clone(), info_c.clone()]);
        let key_ab = domain_coordination_scope_key(
            &info_a.channel,
            &info_a.full_cell_path,
            SharingLevel::from_raw(info_a.domain_sharing_level),
            info_a.facet_depth,
        );
        let key_c = domain_coordination_scope_key(
            &info_c.channel,
            &info_c.full_cell_path,
            SharingLevel::from_raw(info_c.domain_sharing_level),
            info_c.facet_depth,
        );

        assert_eq!(aggregated.len(), 2);
        assert!(aggregated.contains_key(&key_ab));
        assert!(aggregated.contains_key(&key_c));
    }

    #[test]
    fn domain_scope_keys_separate_channels() {
        let x_key =
            domain_coordination_scope_key("x", &[s("A"), s("B")], SharingLevel::from_raw(1), 2);
        let y_key =
            domain_coordination_scope_key("y", &[s("A"), s("B")], SharingLevel::from_raw(1), 2);

        assert_ne!(x_key, y_key);
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
            info_ab1.facet_depth,
        );
        let c_key = domain_coordination_scope_key(
            "x",
            &info_cb.full_cell_path,
            SharingLevel::from_raw(1),
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
        let channel_levels = HashMap::from([("x".to_string(), SharingLevel::GLOBAL)]);

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
            facet_depth: 1,
            extent: DomainExtent::discrete(vec![
                SerializableDomainValue::String("A".to_string()),
                SerializableDomainValue::String("D".to_string()),
            ]),
        };
        let info_b = CellDomainInfo {
            full_cell_path: vec![s("col_b")],
            channel: "x".to_string(),
            domain_sharing_level: 255,
            facet_depth: 1,
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
}
