use std::collections::HashMap;

use crate::{coords::OverflowSpaceRequirement, facet::attribute_context::FacetInheritedContextKey};

#[derive(Clone, Debug)]
pub(crate) struct FacetSynthesisSummary {
    pub(crate) guide_overflow: OverflowSpaceRequirement,
    pub(crate) total_overflow: OverflowSpaceRequirement,
    pub(crate) max_child_padding: f32,
}

#[derive(Clone, Debug)]
pub(crate) enum FacetSynthesisValue {
    LeafMeasured(FacetSynthesisSummary),
    BandAggregated(FacetSynthesisSummary),
}

impl FacetSynthesisValue {
    pub(crate) fn summary(&self) -> &FacetSynthesisSummary {
        match self {
            FacetSynthesisValue::LeafMeasured(summary)
            | FacetSynthesisValue::BandAggregated(summary) => summary,
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

    pub(crate) fn insert_leaf(
        &mut self,
        key: FacetInheritedContextKey,
        summary: FacetSynthesisSummary,
    ) {
        self.synthesized_by_key
            .insert(key, FacetSynthesisValue::LeafMeasured(summary));
    }

    pub(crate) fn insert_band(
        &mut self,
        key: FacetInheritedContextKey,
        summary: FacetSynthesisSummary,
    ) {
        self.synthesized_by_key
            .insert(key, FacetSynthesisValue::BandAggregated(summary));
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

    fn summary() -> FacetSynthesisSummary {
        FacetSynthesisSummary {
            guide_overflow: OverflowSpaceRequirement::default(),
            total_overflow: OverflowSpaceRequirement::default(),
            max_child_padding: 0.0,
        }
    }

    #[test]
    fn attribute_store_returns_exact_key_hits_only() {
        let mut store = FacetAttributeStore::default();
        store.insert_leaf(key(10.0), summary());
        assert!(store.get(&key(10.0)).is_some());
        assert!(store.get(&key(10.5)).is_none());
    }
}
