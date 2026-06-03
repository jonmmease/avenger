use datafusion::{
    arrow::datatypes::DataType, logical_expr::expr::Placeholder, prelude::Expr, scalar::ScalarValue,
};
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelectionValueExpr {
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub expr: LogicalExprNode,
}

impl SelectionValueExpr {
    pub fn new(expr: impl IntoExpr) -> Self {
        Self {
            expr: expr_node(expr.into_expr(), "selection value expression"),
        }
    }

    pub fn to_expr(&self) -> Result<Expr, AvengerChartError> {
        self.expr
            .to_expr(&datafusion::prelude::SessionContext::new())
    }
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelectionIntervalDimensionUpdate {
    pub id: String,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub field_expr: LogicalExprNode,
    pub min: SelectionValueExpr,
    pub max: SelectionValueExpr,
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelectionEqualityDimensionUpdate {
    pub id: String,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub field_expr: LogicalExprNode,
    pub value: SelectionValueExpr,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SelectionPredicateUpdate {
    Interval {
        dimensions: Vec<SelectionIntervalDimensionUpdate>,
    },
    Equality {
        dimensions: Vec<SelectionEqualityDimensionUpdate>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelectionClauseUpdate {
    pub id: SelectionValueExpr,
    #[serde(default = "default_clause_facet_scope")]
    pub facet_scope: Sharing,
    pub predicate: SelectionPredicateUpdate,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SelectionUpdate {
    Clear,
    ClearInScope {
        scope: Sharing,
    },
    ReplaceAllClauses {
        clauses: Vec<SelectionClauseUpdate>,
    },
    ReplaceClausesInScope {
        scope: Sharing,
        clauses: Vec<SelectionClauseUpdate>,
    },
    UpsertClauses {
        clauses: Vec<SelectionClauseUpdate>,
    },
    ToggleClauses {
        clauses: Vec<SelectionClauseUpdate>,
    },
    DeleteClauses {
        ids: Vec<SelectionValueExpr>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedSelectionClauseScope {
    pub sharing: Sharing,
    pub owner_path: Vec<ScalarValue>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SelectionFacetContextValue {
    pub id: String,
    pub value: ScalarValue,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SelectionIntervalDimensionValue {
    pub id: String,
    pub field_expr: LogicalExprNode,
    pub min: ScalarValue,
    pub max: ScalarValue,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SelectionEqualityDimensionValue {
    pub id: String,
    pub field_expr: LogicalExprNode,
    pub value: ScalarValue,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SelectionPredicateSpec {
    Interval {
        dimensions: Vec<SelectionIntervalDimensionValue>,
    },
    Equality {
        dimensions: Vec<SelectionEqualityDimensionValue>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SelectionClause {
    pub id: String,
    pub scope: ResolvedSelectionClauseScope,
    pub predicate: SelectionPredicateSpec,
    pub facet_context: Vec<SelectionFacetContextValue>,
}

impl SelectionUpdate {
    pub fn clear() -> Self {
        Self::Clear
    }

    pub fn clear_in_scope(scope: Sharing) -> Self {
        Self::ClearInScope { scope }
    }

    pub fn replace_all_clauses<I, C>(clauses: I) -> Self
    where
        I: IntoIterator<Item = C>,
        C: Into<SelectionClauseUpdate>,
    {
        Self::ReplaceAllClauses {
            clauses: clauses.into_iter().map(Into::into).collect(),
        }
    }

    pub fn replace_clause(clause: impl Into<SelectionClauseUpdate>) -> Self {
        Self::replace_all_clauses([clause])
    }

    pub fn replace_clauses_in_scope<I, C>(scope: Sharing, clauses: I) -> Self
    where
        I: IntoIterator<Item = C>,
        C: Into<SelectionClauseUpdate>,
    {
        Self::ReplaceClausesInScope {
            scope,
            clauses: clauses.into_iter().map(Into::into).collect(),
        }
    }

    pub fn upsert_clauses<I, C>(clauses: I) -> Self
    where
        I: IntoIterator<Item = C>,
        C: Into<SelectionClauseUpdate>,
    {
        Self::UpsertClauses {
            clauses: clauses.into_iter().map(Into::into).collect(),
        }
    }

    pub fn upsert_clause(clause: impl Into<SelectionClauseUpdate>) -> Self {
        Self::upsert_clauses([clause])
    }

    pub fn toggle_clauses<I, C>(clauses: I) -> Self
    where
        I: IntoIterator<Item = C>,
        C: Into<SelectionClauseUpdate>,
    {
        Self::ToggleClauses {
            clauses: clauses.into_iter().map(Into::into).collect(),
        }
    }

    pub fn toggle_clause(clause: impl Into<SelectionClauseUpdate>) -> Self {
        Self::toggle_clauses([clause])
    }

    pub fn delete_clauses<I, E>(ids: I) -> Self
    where
        I: IntoIterator<Item = E>,
        E: IntoExpr,
    {
        Self::DeleteClauses {
            ids: ids.into_iter().map(SelectionValueExpr::new).collect(),
        }
    }
}

impl SelectionClauseUpdate {
    pub fn interval(id: impl IntoExpr) -> SelectionIntervalClauseBuilder {
        SelectionIntervalClauseBuilder {
            update: Self {
                id: SelectionValueExpr::new(id),
                facet_scope: Sharing::Free,
                predicate: SelectionPredicateUpdate::Interval {
                    dimensions: Vec::new(),
                },
            },
        }
    }

    pub fn equality(id: impl IntoExpr) -> SelectionEqualityClauseBuilder {
        SelectionEqualityClauseBuilder {
            update: Self {
                id: SelectionValueExpr::new(id),
                facet_scope: Sharing::Free,
                predicate: SelectionPredicateUpdate::Equality {
                    dimensions: Vec::new(),
                },
            },
        }
    }
}

pub struct SelectionIntervalClauseBuilder {
    update: SelectionClauseUpdate,
}

impl SelectionIntervalClauseBuilder {
    pub fn facet_scope(mut self, facet_scope: Sharing) -> Self {
        self.update.facet_scope = facet_scope;
        self
    }

    pub fn dimension(self, field_expr: impl IntoExpr) -> SelectionIntervalDimensionBuilder {
        let field_expr = field_expr.into_expr();
        let id = dimension_id_from_expr(&field_expr);
        self.dimension_named(id, field_expr)
    }

    pub fn dimension_named(
        self,
        id: impl Into<String>,
        field_expr: impl IntoExpr,
    ) -> SelectionIntervalDimensionBuilder {
        SelectionIntervalDimensionBuilder {
            builder: self,
            id: id.into(),
            field_expr: expr_node(field_expr.into_expr(), "selection interval dimension field"),
        }
    }

    pub fn build(self) -> SelectionClauseUpdate {
        self.update
    }
}

fn dimension_id_from_expr(expr: &Expr) -> String {
    let id = expr.to_string();
    if id.is_empty() {
        "dimension".to_string()
    } else {
        id
    }
}

impl From<SelectionIntervalClauseBuilder> for SelectionClauseUpdate {
    fn from(builder: SelectionIntervalClauseBuilder) -> Self {
        builder.build()
    }
}

pub struct SelectionIntervalDimensionBuilder {
    builder: SelectionIntervalClauseBuilder,
    id: String,
    field_expr: LogicalExprNode,
}

impl SelectionIntervalDimensionBuilder {
    pub fn endpoints(
        mut self,
        min: impl IntoExpr,
        max: impl IntoExpr,
    ) -> SelectionIntervalClauseBuilder {
        let dimensions = match &mut self.builder.update.predicate {
            SelectionPredicateUpdate::Interval { dimensions } => dimensions,
            SelectionPredicateUpdate::Equality { .. } => {
                unreachable!("interval builder always owns interval predicate")
            }
        };
        dimensions.push(SelectionIntervalDimensionUpdate {
            id: self.id,
            field_expr: self.field_expr,
            min: SelectionValueExpr::new(min),
            max: SelectionValueExpr::new(max),
        });
        self.builder
    }
}

pub struct SelectionEqualityClauseBuilder {
    update: SelectionClauseUpdate,
}

impl SelectionEqualityClauseBuilder {
    pub fn facet_scope(mut self, facet_scope: Sharing) -> Self {
        self.update.facet_scope = facet_scope;
        self
    }

    pub fn dimension(self, field_expr: impl IntoExpr, value: impl IntoExpr) -> Self {
        let field_expr = field_expr.into_expr();
        let id = dimension_id_from_expr(&field_expr);
        self.dimension_named(id, field_expr, value)
    }

    pub fn dimension_named(
        mut self,
        id: impl Into<String>,
        field_expr: impl IntoExpr,
        value: impl IntoExpr,
    ) -> Self {
        let dimensions = match &mut self.update.predicate {
            SelectionPredicateUpdate::Equality { dimensions } => dimensions,
            SelectionPredicateUpdate::Interval { .. } => {
                unreachable!("equality builder always owns equality predicate")
            }
        };
        dimensions.push(SelectionEqualityDimensionUpdate {
            id: id.into(),
            field_expr: expr_node(field_expr.into_expr(), "selection equality dimension field"),
            value: SelectionValueExpr::new(value),
        });
        self
    }

    pub fn build(self) -> SelectionClauseUpdate {
        self.update
    }
}

impl From<SelectionEqualityClauseBuilder> for SelectionClauseUpdate {
    fn from(builder: SelectionEqualityClauseBuilder) -> Self {
        builder.build()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Selection {
    pub id: String,
    pub empty: EmptySelectionBehavior,
    #[serde(default)]
    pub combine: SelectionCombine,
    #[serde(default)]
    pub facet_context: Vec<SelectionFacetContextSpec>,
}

impl Selection {
    fn base(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            empty: EmptySelectionBehavior::SelectNothing,
            combine: SelectionCombine::Union,
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
        Ok(CompiledSelectionSpec {
            id: self.id.clone(),
            empty: self.empty,
            combine: self.combine,
            facet_context: self.facet_context.clone(),
        })
    }
}

fn default_clause_facet_scope() -> Sharing {
    Sharing::Free
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompiledSelectionSpec {
    pub id: String,
    pub empty: EmptySelectionBehavior,
    #[serde(default)]
    pub combine: SelectionCombine,
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
    fn neutral_selection_compiles_without_source() {
        let selection = Selection::new("brush").empty_selects_nothing();
        let compiled = selection.compile().expect("compile selection");
        assert_eq!(compiled.empty, EmptySelectionBehavior::SelectNothing);
    }

    #[test]
    fn facet_context_selection_compiles_facet_spec_and_predicate() {
        let selection =
            Selection::new("brush").facet_context_field("group_name", col("group_name"));
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

    #[test]
    fn interval_clause_update_builder_serializes() {
        let clause = SelectionClauseUpdate::interval("active")
            .facet_scope(Sharing::Level(1))
            .dimension(col("x"))
            .endpoints(lit(1.0), lit(2.0))
            .dimension_named("vertical", col("y"))
            .endpoints(lit(3.0), lit(4.0))
            .build();
        let json = serde_json::to_string(&clause).expect("serialize clause");
        let restored: SelectionClauseUpdate =
            serde_json::from_str(&json).expect("deserialize clause");
        assert_eq!(restored.facet_scope, Sharing::Level(1));
        let SelectionPredicateUpdate::Interval { dimensions } = restored.predicate else {
            panic!("expected interval predicate");
        };
        assert_eq!(dimensions.len(), 2);
        assert_eq!(dimensions[0].id, "x");
        assert_eq!(dimensions[1].id, "vertical");
    }

    #[test]
    fn equality_clause_update_builder_serializes() {
        let clause = SelectionClauseUpdate::equality("active")
            .facet_scope(Sharing::Level(1))
            .dimension(col("category"), event::datum("category"))
            .dimension_named("region_key", col("region"), event::datum("region"))
            .build();
        let json = serde_json::to_string(&clause).expect("serialize clause");
        let restored: SelectionClauseUpdate =
            serde_json::from_str(&json).expect("deserialize clause");
        assert_eq!(restored.facet_scope, Sharing::Level(1));
        let SelectionPredicateUpdate::Equality { dimensions } = restored.predicate else {
            panic!("expected equality predicate");
        };
        assert_eq!(dimensions.len(), 2);
        assert_eq!(dimensions[0].id, "category");
        assert_eq!(dimensions[1].id, "region_key");
    }

    #[test]
    fn toggle_clause_update_serializes() {
        let update = SelectionUpdate::toggle_clause(
            SelectionClauseUpdate::equality(event::datum("category"))
                .dimension(col("category"), event::datum("category")),
        );
        let json = serde_json::to_string(&update).expect("serialize update");
        let restored: SelectionUpdate = serde_json::from_str(&json).expect("deserialize update");
        let SelectionUpdate::ToggleClauses { clauses } = restored else {
            panic!("expected toggle clauses");
        };
        assert_eq!(clauses.len(), 1);
    }
}
