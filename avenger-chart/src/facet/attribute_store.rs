use std::collections::HashMap;

use crate::{
    coords::OverflowSpaceRequirement,
    facet::attribute_context::{FacetCellMeasureContextKey, FacetInheritedContextKey},
    scales::ScaleBuilder,
};

#[derive(Clone, Debug)]
pub(crate) struct FacetCellProbeSummary {
    pub(crate) guide_overflow: OverflowSpaceRequirement,
    pub(crate) total_overflow: OverflowSpaceRequirement,
    pub(crate) max_child_padding: f32,
}

#[derive(Clone, Debug)]
pub(crate) struct FacetSynthesisProbePayload {
    pub(crate) measurement_key: FacetCellMeasureContextKey,
    pub(crate) cell_scale_builder: Option<ScaleBuilder>,
}

#[derive(Clone, Debug)]
pub(crate) struct FacetBandProbeSynthesis {
    pub(crate) cell_probe_summary: FacetCellProbeSummary,
    pub(crate) child_cell_summaries: Vec<FacetCellProbeSummary>,
}

#[derive(Clone, Debug)]
pub(crate) enum FacetSynthesisValue {
    LeafMeasured {
        cell_probe_summary: FacetCellProbeSummary,
        probe_payload: Option<FacetSynthesisProbePayload>,
    },
    BandAggregated {
        band_probe_synthesis: FacetBandProbeSynthesis,
    },
}

impl FacetSynthesisValue {
    pub(crate) fn cell_probe_summary(&self) -> &FacetCellProbeSummary {
        match self {
            FacetSynthesisValue::LeafMeasured {
                cell_probe_summary, ..
            } => cell_probe_summary,
            FacetSynthesisValue::BandAggregated {
                band_probe_synthesis,
            } => &band_probe_synthesis.cell_probe_summary,
        }
    }

    pub(crate) fn as_probe_payload(&self) -> Option<&FacetSynthesisProbePayload> {
        match self {
            FacetSynthesisValue::LeafMeasured {
                probe_payload: Some(payload),
                ..
            } => Some(payload),
            _ => None,
        }
    }
}

#[derive(Default)]
pub(crate) struct FacetAttributeStore {
    synthesized_by_key: HashMap<FacetInheritedContextKey, FacetSynthesisValue>,
}

impl FacetAttributeStore {
    pub(crate) fn get(&self, key: &FacetInheritedContextKey) -> Option<&FacetSynthesisValue> {
        self.synthesized_by_key.get(key)
    }

    pub(crate) fn insert_value(
        &mut self,
        key: FacetInheritedContextKey,
        value: FacetSynthesisValue,
    ) {
        self.synthesized_by_key.insert(key, value);
    }

    pub(crate) fn insert_leaf(
        &mut self,
        key: FacetInheritedContextKey,
        cell_probe_summary: FacetCellProbeSummary,
        probe_payload: Option<FacetSynthesisProbePayload>,
    ) {
        self.synthesized_by_key.insert(
            key,
            FacetSynthesisValue::LeafMeasured {
                cell_probe_summary,
                probe_payload,
            },
        );
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        coords::OverflowSpaceRequirement,
        facet::attribute_context::{FacetInheritedContextKey, ScaleScopeKey},
    };
    use datafusion::common::ScalarValue;

    fn key(width: f32) -> FacetInheritedContextKey {
        FacetInheritedContextKey::from_parts(
            vec![ScalarValue::Utf8(Some("a".to_string()))],
            width,
            10.0,
            false,
            false,
            ScaleScopeKey::Shared,
            0,
        )
    }

    fn summary() -> FacetCellProbeSummary {
        FacetCellProbeSummary {
            guide_overflow: OverflowSpaceRequirement::default(),
            total_overflow: OverflowSpaceRequirement::default(),
            max_child_padding: 0.0,
        }
    }

    #[test]
    fn attribute_store_returns_exact_key_hits_only() {
        let mut store = FacetAttributeStore::default();
        store.insert_leaf(key(10.0), summary(), None);
        assert!(store.get(&key(10.0)).is_some());
        assert!(store.get(&key(10.5)).is_none());
    }
}
