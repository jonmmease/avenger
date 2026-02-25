use crate::{
    error::AvengerChartError,
    facet::{
        attribute_context::FacetInheritedContextKey,
        attribute_store::{FacetAttributeStore, FacetSynthesisSummary},
    },
};
use std::future::Future;

#[allow(dead_code)]
pub(crate) enum SynthesisKind {
    Leaf,
    Band,
}

#[cfg(test)]
pub(crate) fn evaluate_synthesized<F>(
    store: &mut FacetAttributeStore,
    key: FacetInheritedContextKey,
    kind: SynthesisKind,
    evaluate: F,
) -> Result<(FacetSynthesisSummary, bool), AvengerChartError>
where
    F: FnOnce() -> Result<FacetSynthesisSummary, AvengerChartError>,
{
    if let Some(hit) = store.get(&key) {
        return Ok((hit.summary().clone(), true));
    }

    let summary = evaluate()?;
    match kind {
        SynthesisKind::Leaf => store.insert_leaf(key, summary.clone()),
        SynthesisKind::Band => store.insert_band(key, summary.clone()),
    }
    Ok((summary, false))
}

pub(crate) async fn evaluate_synthesized_async<F, Fut>(
    store: &mut FacetAttributeStore,
    key: FacetInheritedContextKey,
    kind: SynthesisKind,
    evaluate: F,
) -> Result<(FacetSynthesisSummary, bool), AvengerChartError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<FacetSynthesisSummary, AvengerChartError>>,
{
    if let Some(hit) = store.get(&key) {
        return Ok((hit.summary().clone(), true));
    }

    let summary = evaluate().await?;
    match kind {
        SynthesisKind::Leaf => store.insert_leaf(key, summary.clone()),
        SynthesisKind::Band => store.insert_band(key, summary.clone()),
    }
    Ok((summary, false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        coords::OverflowSpaceRequirement,
        facet::{
            attribute_context::{FacetInheritedContextKey, ScaleScopeKey},
            attribute_store::FacetSynthesisSummary,
        },
    };
    use datafusion::common::ScalarValue;

    fn key() -> FacetInheritedContextKey {
        FacetInheritedContextKey::from_parts(
            vec![ScalarValue::Utf8(Some("k".to_string()))],
            20.0,
            30.0,
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
    fn synthesized_scheduler_evaluates_children_before_parent() {
        let mut store = FacetAttributeStore::default();
        let mut called = 0usize;

        let (_value, hit) = evaluate_synthesized(&mut store, key(), SynthesisKind::Leaf, || {
            called += 1;
            Ok(summary())
        })
        .expect("first evaluation should succeed");
        assert!(!hit);

        let (_value, hit) = evaluate_synthesized(&mut store, key(), SynthesisKind::Leaf, || {
            called += 1;
            Ok(summary())
        })
        .expect("second evaluation should succeed");
        assert!(hit);
        assert_eq!(called, 1);
    }
}
