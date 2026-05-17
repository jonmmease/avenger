use std::collections::HashMap;

use datafusion::common::ScalarValue;

use crate::{
    coords::CellDomainInfo,
    facet::{
        coord::{ChannelDomainExtent, union_domain_extents},
        sharing_level::SharingLevel,
        sharing_policy,
    },
    scales::domain_extent::DomainExtent,
};

pub(crate) fn aggregate_domain_extents(
    infos: &[CellDomainInfo],
) -> HashMap<(String, Vec<ScalarValue>), DomainExtent> {
    let mut groups: HashMap<(String, Vec<ScalarValue>), Vec<&DomainExtent>> = HashMap::new();

    for info in infos {
        let ancestor_key = sharing_policy::domain_group_key(
            &info.full_cell_path,
            SharingLevel::from_raw(info.domain_sharing_level),
            info.facet_depth,
        );
        groups
            .entry((info.channel.clone(), ancestor_key))
            .or_default()
            .push(&info.extent);
    }

    groups
        .into_iter()
        .map(|(key, extents)| {
            let mut extents = extents.into_iter();
            let first = extents
                .next()
                .expect("Facet domain coordination invariant violated: non-empty group expected");
            let unified = extents.fold(first.clone(), |acc, extent| {
                union_domain_extents(&acc, extent)
            });
            (key, unified)
        })
        .collect()
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
    unified: &HashMap<(String, Vec<ScalarValue>), DomainExtent>,
) -> HashMap<String, DomainExtent> {
    let mut coordinated = HashMap::new();

    for (channel, annotated) in local_domain_extents {
        if annotated.domain_sharing_level == 0 {
            continue;
        }

        let ancestor_key = sharing_policy::domain_group_key(
            full_cell_path,
            annotated.domain_sharing_level,
            facet_depth,
        );

        if let Some(unified_extent) = unified.get(&(channel.clone(), ancestor_key)) {
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

        let ancestor_key =
            sharing_policy::domain_group_key(full_cell_path, *sharing_level, facet_depth);

        if let Some(unified_extent) = unified.get(&(channel.clone(), ancestor_key)) {
            coordinated.insert(channel.clone(), unified_extent.clone());
        }
    }

    coordinated
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        facet::sharing_policy,
        scales::domain_extent::{DomainBounds, DomainExtent, SerializableDomainValue},
    };

    fn s(value: &str) -> ScalarValue {
        ScalarValue::Utf8(Some(value.to_string()))
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
        let key_ab = sharing_policy::domain_group_key(
            &info_a.full_cell_path,
            SharingLevel::from_raw(info_a.domain_sharing_level),
            info_a.facet_depth,
        );
        let key_c = sharing_policy::domain_group_key(
            &info_c.full_cell_path,
            SharingLevel::from_raw(info_c.domain_sharing_level),
            info_c.facet_depth,
        );

        assert_eq!(aggregated.len(), 2);
        assert!(aggregated.contains_key(&("x".to_string(), key_ab)));
        assert!(aggregated.contains_key(&("x".to_string(), key_c)));
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
