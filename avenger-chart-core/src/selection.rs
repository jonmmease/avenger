use datafusion::{arrow::datatypes::DataType, logical_expr::expr::Placeholder, prelude::Expr};
use datafusion_proto::protobuf::LogicalExprNode;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::{AvengerChartError, DefaultLogicalExprNodeExt, IntoExpr, SerializableExpr, Sharing};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EmptySelectionBehavior {
    SelectAll,
    SelectNothing,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectionCombine {
    #[default]
    Union,
    Intersect,
}

#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelectionFacetContextSpec {
    pub id: String,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub field_expr: LogicalExprNode,
}

#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelectionSourceDimensionSpec {
    pub id: String,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub field_expr: LogicalExprNode,
    pub min_field: String,
    pub max_field: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelectionSourceSpec {
    pub store_name: String,
    #[serde(default)]
    pub dimensions: Vec<SelectionSourceDimensionSpec>,
}

pub type SelectionSource = SelectionSourceSpec;

impl SelectionSourceSpec {
    pub fn store(store_name: impl Into<String>) -> Self {
        Self {
            store_name: store_name.into(),
            dimensions: Vec::new(),
        }
    }

    pub fn interval(self) -> Self {
        self
    }

    pub fn dimension(
        self,
        id: impl Into<String>,
        field_expr: impl IntoExpr,
    ) -> SelectionSourceDimensionBuilder {
        SelectionSourceDimensionBuilder {
            source: self,
            id: id.into(),
            field_expr: expr_node(field_expr.into_expr(), "selection source dimension field"),
        }
    }
}

pub struct SelectionSourceDimensionBuilder {
    source: SelectionSourceSpec,
    id: String,
    field_expr: LogicalExprNode,
}

impl SelectionSourceDimensionBuilder {
    pub fn bounds(
        mut self,
        min_field: impl Into<String>,
        max_field: impl Into<String>,
    ) -> SelectionSourceSpec {
        self.source.dimensions.push(SelectionSourceDimensionSpec {
            id: self.id,
            field_expr: self.field_expr,
            min_field: min_field.into(),
            max_field: max_field.into(),
        });
        self.source
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Selection {
    pub id: String,
    pub empty: EmptySelectionBehavior,
    #[serde(default)]
    pub combine: SelectionCombine,
    #[serde(default = "default_selection_sharing")]
    pub sharing: Sharing,
    #[serde(default)]
    pub source: Option<SelectionSourceSpec>,
    #[serde(default)]
    pub facet_context: Vec<SelectionFacetContextSpec>,
}

impl Selection {
    fn base(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            empty: EmptySelectionBehavior::SelectNothing,
            combine: SelectionCombine::Union,
            sharing: Sharing::Free,
            source: None,
            facet_context: Vec::new(),
        }
    }

    pub fn new(id: impl Into<String>) -> Self {
        Self::base(id)
    }

    pub fn empty_selects_nothing(mut self) -> Self {
        self.empty = EmptySelectionBehavior::SelectNothing;
        self
    }

    pub fn empty_selects_all(mut self) -> Self {
        self.empty = EmptySelectionBehavior::SelectAll;
        self
    }

    pub fn combine(mut self, combine: SelectionCombine) -> Self {
        self.combine = combine;
        self
    }

    pub fn sharing(mut self, sharing: Sharing) -> Self {
        self.sharing = sharing;
        self
    }

    pub fn source(mut self, source: SelectionSourceSpec) -> Self {
        self.source = Some(source);
        self
    }

    pub fn facet_context_field(mut self, id: impl Into<String>, expr: impl IntoExpr) -> Self {
        self.facet_context.push(SelectionFacetContextSpec {
            id: id.into(),
            field_expr: expr_node(expr.into_expr(), "selection facet context field"),
        });
        self
    }

    pub fn predicate(&self) -> Expr {
        selection_predicate_expr(&self.id)
    }

    pub fn compile(&self) -> Result<CompiledSelectionSpec, AvengerChartError> {
        validate_selection_id(&self.id)?;
        for facet in &self.facet_context {
            validate_selection_id(&facet.id)?;
        }
        let Some(source) = &self.source else {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Selection '{}' must define a predicate source",
                self.id
            )));
        };
        validate_selection_source(&self.id, source)?;
        Ok(CompiledSelectionSpec {
            id: self.id.clone(),
            empty: self.empty,
            combine: self.combine,
            sharing: self.sharing,
            source: self.source.clone(),
            facet_context: self.facet_context.clone(),
        })
    }
}

fn validate_selection_source(
    selection_id: &str,
    source: &SelectionSourceSpec,
) -> Result<(), AvengerChartError> {
    crate::validate_store_name(&source.store_name)?;
    if source.dimensions.is_empty() {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Selection '{selection_id}' store source must define at least one dimension"
        )));
    }
    let mut ids = std::collections::HashSet::new();
    for dimension in &source.dimensions {
        validate_selection_id(&dimension.id)?;
        if !ids.insert(dimension.id.clone()) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Selection '{selection_id}' declares duplicate source dimension '{}'",
                dimension.id
            )));
        }
        if dimension.min_field.is_empty() || dimension.max_field.is_empty() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Selection '{selection_id}' source dimension '{}' must define non-empty bounds fields",
                dimension.id
            )));
        }
    }
    Ok(())
}

fn default_selection_sharing() -> Sharing {
    Sharing::Free
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompiledSelectionSpec {
    pub id: String,
    pub empty: EmptySelectionBehavior,
    #[serde(default)]
    pub combine: SelectionCombine,
    #[serde(default = "default_selection_sharing")]
    pub sharing: Sharing,
    #[serde(default)]
    pub source: Option<SelectionSourceSpec>,
    #[serde(default)]
    pub facet_context: Vec<SelectionFacetContextSpec>,
}

fn selection_predicate_expr(id: impl AsRef<str>) -> Expr {
    Expr::Placeholder(Placeholder {
        id: selection_predicate_placeholder_id(id.as_ref()),
        data_type: Some(DataType::Boolean),
    })
}

fn selection_predicate_placeholder_id(id: &str) -> String {
    format!("$__selection_predicate_{id}")
}

pub fn selection_id_from_predicate_placeholder(placeholder_id: &str) -> Option<&str> {
    placeholder_id.strip_prefix("$__selection_predicate_")
}

pub fn compile_selections(
    selections: &[Selection],
) -> Result<IndexMap<String, CompiledSelectionSpec>, AvengerChartError> {
    let mut compiled = IndexMap::new();
    for selection in selections {
        let spec = selection.compile()?;
        if compiled.contains_key(&spec.id) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Duplicate selection id '{}'",
                spec.id
            )));
        }
        compiled.insert(spec.id.clone(), spec);
    }
    Ok(compiled)
}

fn validate_selection_id(id: &str) -> Result<(), AvengerChartError> {
    if id.is_empty()
        || id.contains('.')
        || !id.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Selection id '{id}' must be a non-empty ASCII identifier without periods"
        )));
    }
    Ok(())
}

fn expr_node(expr: Expr, label: &str) -> LogicalExprNode {
    LogicalExprNode::from_expr(expr)
        .unwrap_or_else(|err| panic!("Failed to serialize {label}: {err}"))
}

#[cfg(test)]
mod tests {
    use datafusion::prelude::{col, lit};

    use super::*;
    use crate::event;

    #[test]
    fn selection_predicate_serializes() {
        let expr = Selection::new("brush").predicate();
        LogicalExprNode::from_expr(expr).expect("selection predicate serializes");
    }

    #[test]
    fn neutral_selection_requires_source() {
        let err = Selection::new("brush")
            .compile()
            .expect_err("missing source");
        assert!(format!("{err:?}").contains("must define a predicate source"));
    }

    #[test]
    fn store_source_selection_compiles() {
        let selection = Selection::new("brush")
            .source(
                SelectionSourceSpec::store("brush_boxes")
                    .interval()
                    .dimension("x", col("x"))
                    .bounds("x_min", "x_max")
                    .dimension("y", col("y"))
                    .bounds("y_min", "y_max"),
            )
            .empty_selects_nothing();
        let compiled = selection.compile().expect("compile selection");
        let source = compiled.source.expect("selection source");
        assert_eq!(source.store_name, "brush_boxes");
        assert_eq!(source.dimensions.len(), 2);
        assert_eq!(source.dimensions[0].min_field, "x_min");
        assert_eq!(compiled.empty, EmptySelectionBehavior::SelectNothing);
    }

    #[test]
    fn facet_context_selection_compiles_facet_spec_and_predicate() {
        let selection = Selection::new("brush")
            .source(
                SelectionSourceSpec::store("brush_boxes")
                    .interval()
                    .dimension("x", col("x"))
                    .bounds("x_min", "x_max"),
            )
            .facet_context_field("group_name", col("group_name"));
        let compiled = selection.compile().expect("compile selection");
        assert_eq!(compiled.facet_context.len(), 1);
        assert_eq!(compiled.facet_context[0].id, "group_name");
        LogicalExprNode::from_expr(selection.predicate())
            .expect("facet-context selection predicate serializes");
    }

    #[test]
    fn ordered_interval_expr_serializes() {
        let expr = event::interval_ordered(lit(4.0), lit(1.0));
        LogicalExprNode::from_expr(expr).expect("ordered interval serializes");
    }
}
