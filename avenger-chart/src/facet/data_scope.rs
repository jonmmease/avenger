//! Runtime helpers for mark-level faceted data scope.

use datafusion::{common::ScalarValue, dataframe::DataFrame, logical_expr::Expr};

use avenger_chart_core::FacetDataScope;

use crate::{error::AvengerChartError, facet::evaluated_facet_tree::EvaluatedFacetTree};

/// Facet data available while preparing a mark inside a facet cell.
#[derive(Clone, Copy)]
pub(crate) struct FacetDataScopeContext<'a> {
    pub(crate) facet_tree: &'a EvaluatedFacetTree,
    pub(crate) root_data: Option<&'a DataFrame>,
    pub(crate) full_path: &'a [ScalarValue],
}

impl<'a> FacetDataScopeContext<'a> {
    pub(crate) fn new(
        facet_tree: &'a EvaluatedFacetTree,
        root_data: Option<&'a DataFrame>,
        full_path: &'a [ScalarValue],
    ) -> Self {
        Self {
            facet_tree,
            root_data,
            full_path,
        }
    }
}

pub(crate) fn predicate_for_scope(
    facet_tree: &EvaluatedFacetTree,
    full_path: &[ScalarValue],
    scope: FacetDataScope,
) -> Option<Expr> {
    facet_tree.cell_predicate(full_path, scope.raw_level())
}

pub(crate) fn data_for_scope(
    root_data: &DataFrame,
    facet_tree: &EvaluatedFacetTree,
    full_path: &[ScalarValue],
    scope: FacetDataScope,
) -> Result<DataFrame, AvengerChartError> {
    if let Some(predicate) = predicate_for_scope(facet_tree, full_path, scope) {
        Ok(root_data.clone().filter(predicate)?)
    } else {
        Ok(root_data.clone())
    }
}

pub(crate) fn inherited_data_for_scope(
    default_data: Option<&DataFrame>,
    scope: FacetDataScope,
    context: Option<FacetDataScopeContext<'_>>,
) -> Result<Option<DataFrame>, AvengerChartError> {
    let Some(context) = context else {
        return Ok(default_data.cloned());
    };

    if context.full_path.is_empty() || scope.is_filtered() {
        return Ok(default_data.cloned());
    }

    let Some(root_data) = context.root_data.or(default_data) else {
        return Ok(None);
    };

    data_for_scope(root_data, context.facet_tree, context.full_path, scope).map(Some)
}
