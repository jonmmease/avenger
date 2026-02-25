use crate::{
    error::AvengerChartError,
    facet::{
        attribute_context::FacetInheritedContextKey,
        attribute_store::{FacetAttributeStore, FacetSynthesisValue},
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
    evaluate: F,
) -> Result<(FacetSynthesisValue, bool), AvengerChartError>
where
    F: FnOnce() -> Result<FacetSynthesisValue, AvengerChartError>,
{
    if let Some(hit) = store.get(&key) {
        return Ok((hit.clone(), true));
    }

    let value = evaluate()?;
    store.insert_value(key, value.clone());
    Ok((value, false))
}

#[allow(dead_code)]
pub(crate) async fn evaluate_synthesized_async<F, Fut>(
    store: &mut FacetAttributeStore,
    key: FacetInheritedContextKey,
    evaluate: F,
) -> Result<(FacetSynthesisValue, bool), AvengerChartError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<FacetSynthesisValue, AvengerChartError>>,
{
    if let Some(hit) = store.get(&key) {
        return Ok((hit.clone(), true));
    }

    let value = evaluate().await?;
    store.insert_value(key, value.clone());
    Ok((value, false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        coords::OverflowSpaceRequirement,
        facet::{
            attribute_context::{FacetInheritedContextKey, ScaleScopeKey},
            attribute_store::{FacetCellProbeSummary, FacetSynthesisValue},
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

    fn value() -> FacetSynthesisValue {
        FacetSynthesisValue::LeafMeasured {
            cell_probe_summary: FacetCellProbeSummary {
                guide_overflow: OverflowSpaceRequirement::default(),
                total_overflow: OverflowSpaceRequirement::default(),
                max_child_padding: 0.0,
            },
            probe_payload: None,
        }
    }

    #[test]
    fn synthesized_scheduler_evaluates_children_before_parent() {
        let mut store = FacetAttributeStore::default();
        let mut called = 0usize;

        let (_value, hit) = evaluate_synthesized(&mut store, key(), || {
            called += 1;
            Ok(value())
        })
        .expect("first evaluation should succeed");
        assert!(!hit);

        let (_value, hit) = evaluate_synthesized(&mut store, key(), || {
            called += 1;
            Ok(value())
        })
        .expect("second evaluation should succeed");
        assert!(hit);
        assert_eq!(called, 1);
    }
}
