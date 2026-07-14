use std::sync::Arc;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as BASE64_URL_SAFE_NO_PAD};
use datafusion::prelude::SessionContext;
use datafusion::{
    arrow::datatypes::{DataType, Field},
    logical_expr::expr::Placeholder,
    prelude::Expr,
    scalar::ScalarValue,
};
use datafusion_proto::protobuf::LogicalExprNode;
use indexmap::IndexMap;
use prost::Message;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::{
    AvengerChartError, CoordinationScope, DefaultLogicalExprNodeExt, IntoExpr, SelectionSceneQuery,
    SerializableExpr,
};

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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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

    pub(crate) fn map_exprs(
        self,
        f: &mut impl FnMut(Expr) -> Result<Expr, AvengerChartError>,
    ) -> Result<Self, AvengerChartError> {
        Ok(Self {
            expr: map_expr_node(self.expr, f, "selection value expression")?,
        })
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

impl SelectionIntervalDimensionUpdate {
    pub(crate) fn map_exprs(
        self,
        f: &mut impl FnMut(Expr) -> Result<Expr, AvengerChartError>,
    ) -> Result<Self, AvengerChartError> {
        Ok(Self {
            id: self.id,
            field_expr: map_expr_node(self.field_expr, f, "selection interval dimension field")?,
            min: self.min.map_exprs(f)?,
            max: self.max.map_exprs(f)?,
        })
    }
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelectionEqualityDimensionUpdate {
    pub id: String,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub field_expr: LogicalExprNode,
    pub value: SelectionValueExpr,
}

impl SelectionEqualityDimensionUpdate {
    pub(crate) fn map_exprs(
        self,
        f: &mut impl FnMut(Expr) -> Result<Expr, AvengerChartError>,
    ) -> Result<Self, AvengerChartError> {
        Ok(Self {
            id: self.id,
            field_expr: map_expr_node(self.field_expr, f, "selection equality dimension field")?,
            value: self.value.map_exprs(f)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelectionPredicateValueUpdate {
    pub id: String,
    pub value: SelectionValueExpr,
}

impl SelectionPredicateValueUpdate {
    pub(crate) fn map_exprs(
        self,
        f: &mut impl FnMut(Expr) -> Result<Expr, AvengerChartError>,
    ) -> Result<Self, AvengerChartError> {
        Ok(Self {
            id: self.id,
            value: self.value.map_exprs(f)?,
        })
    }
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SelectionPredicateUpdate {
    Interval {
        dimensions: Vec<SelectionIntervalDimensionUpdate>,
    },
    Equality {
        dimensions: Vec<SelectionEqualityDimensionUpdate>,
    },
    Predicate {
        values: Vec<SelectionPredicateValueUpdate>,
        #[serde_as(as = "FromInto<SerializableExpr>")]
        expr: LogicalExprNode,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<String>,
    },
}

impl SelectionPredicateUpdate {
    pub(crate) fn map_exprs(
        self,
        f: &mut impl FnMut(Expr) -> Result<Expr, AvengerChartError>,
    ) -> Result<Self, AvengerChartError> {
        Ok(match self {
            Self::Interval { dimensions } => Self::Interval {
                dimensions: dimensions
                    .into_iter()
                    .map(|dimension| dimension.map_exprs(f))
                    .collect::<Result<_, AvengerChartError>>()?,
            },
            Self::Equality { dimensions } => Self::Equality {
                dimensions: dimensions
                    .into_iter()
                    .map(|dimension| dimension.map_exprs(f))
                    .collect::<Result<_, AvengerChartError>>()?,
            },
            Self::Predicate { values, expr, kind } => Self::Predicate {
                values: values
                    .into_iter()
                    .map(|value| value.map_exprs(f))
                    .collect::<Result<_, AvengerChartError>>()?,
                expr: map_expr_node(expr, f, "selection generic predicate expression")?,
                kind,
            },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelectionClauseUpdate {
    pub id: SelectionValueExpr,
    #[serde(default = "default_clause_facet_scope")]
    pub facet_scope: CoordinationScope,
    pub predicate: SelectionPredicateUpdate,
}

impl SelectionClauseUpdate {
    pub(crate) fn map_exprs(
        self,
        f: &mut impl FnMut(Expr) -> Result<Expr, AvengerChartError>,
    ) -> Result<Self, AvengerChartError> {
        Ok(Self {
            id: self.id.map_exprs(f)?,
            facet_scope: self.facet_scope,
            predicate: self.predicate.map_exprs(f)?,
        })
    }
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SelectionUpdate {
    Clear,
    ClearInScope {
        scope: CoordinationScope,
    },
    ReplaceAllClauses {
        clauses: Vec<SelectionClauseUpdate>,
    },
    ReplaceClausesInScope {
        scope: CoordinationScope,
        clauses: Vec<SelectionClauseUpdate>,
    },
    UpsertClauses {
        clauses: Vec<SelectionClauseUpdate>,
    },
    ToggleClauses {
        clauses: Vec<SelectionClauseUpdate>,
    },
    ToggleEqualityValue {
        #[serde_as(as = "FromInto<SerializableExpr>")]
        field_expr: LogicalExprNode,
        value: SelectionValueExpr,
        item_id: SelectionValueExpr,
        #[serde(default = "default_clause_facet_scope")]
        facet_scope: CoordinationScope,
    },
    ReplaceAllFromSceneQuery {
        query: SelectionSceneQuery,
    },
    ReplaceFromSceneQueryInScope {
        query: SelectionSceneQuery,
    },
    UpsertFromSceneQuery {
        query: SelectionSceneQuery,
    },
    ToggleFromSceneQuery {
        query: SelectionSceneQuery,
    },
    DeleteClauses {
        ids: Vec<SelectionValueExpr>,
    },
    DeleteClausesInScope {
        scope: CoordinationScope,
        ids: Vec<SelectionValueExpr>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedSelectionClauseScope {
    pub sharing: CoordinationScope,
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
pub struct SelectionPredicateValue {
    pub id: String,
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
    Predicate {
        values: Vec<SelectionPredicateValue>,
        expr: LogicalExprNode,
        kind: Option<String>,
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

    pub fn clear_in_scope(scope: CoordinationScope) -> Self {
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

    pub fn replace_clauses_in_scope<I, C>(scope: CoordinationScope, clauses: I) -> Self
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

    /// Toggle one typed value in an equality selection without depending on a
    /// caller-chosen clause id.
    pub fn toggle_equality_value(
        field_expr: impl IntoExpr,
        value_expr: impl IntoExpr,
        item_id_expr: impl IntoExpr,
    ) -> Self {
        Self::toggle_equality_value_in_scope(
            CoordinationScope::Free,
            field_expr,
            value_expr,
            item_id_expr,
        )
    }

    /// Toggle one typed value in an equality selection in the requested
    /// coordination scope without depending on a caller-chosen clause id.
    pub fn toggle_equality_value_in_scope(
        scope: CoordinationScope,
        field_expr: impl IntoExpr,
        value_expr: impl IntoExpr,
        item_id_expr: impl IntoExpr,
    ) -> Self {
        Self::ToggleEqualityValue {
            field_expr: expr_node(field_expr.into_expr(), "selection equality toggle field"),
            value: SelectionValueExpr::new(value_expr),
            item_id: SelectionValueExpr::new(item_id_expr),
            facet_scope: scope,
        }
    }

    pub fn replace_all_from_scene_query(query: impl Into<SelectionSceneQuery>) -> Self {
        Self::ReplaceAllFromSceneQuery {
            query: query.into(),
        }
    }

    pub fn replace_from_scene_query_in_scope(query: impl Into<SelectionSceneQuery>) -> Self {
        Self::ReplaceFromSceneQueryInScope {
            query: query.into(),
        }
    }

    pub fn upsert_from_scene_query(query: impl Into<SelectionSceneQuery>) -> Self {
        Self::UpsertFromSceneQuery {
            query: query.into(),
        }
    }

    pub fn toggle_from_scene_query(query: impl Into<SelectionSceneQuery>) -> Self {
        Self::ToggleFromSceneQuery {
            query: query.into(),
        }
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

    pub fn delete_clause(id: impl IntoExpr) -> Self {
        Self::delete_clauses([id])
    }

    pub fn delete_clauses_in_scope<I, E>(scope: CoordinationScope, ids: I) -> Self
    where
        I: IntoIterator<Item = E>,
        E: IntoExpr,
    {
        Self::DeleteClausesInScope {
            scope,
            ids: ids.into_iter().map(SelectionValueExpr::new).collect(),
        }
    }

    pub fn delete_clause_in_scope(scope: CoordinationScope, id: impl IntoExpr) -> Self {
        Self::delete_clauses_in_scope(scope, [id])
    }

    pub(crate) fn map_exprs(
        self,
        f: &mut impl FnMut(Expr) -> Result<Expr, AvengerChartError>,
    ) -> Result<Self, AvengerChartError> {
        Ok(match self {
            Self::Clear => Self::Clear,
            Self::ClearInScope { scope } => Self::ClearInScope { scope },
            Self::ReplaceAllClauses { clauses } => Self::ReplaceAllClauses {
                clauses: map_clauses(clauses, f)?,
            },
            Self::ReplaceClausesInScope { scope, clauses } => Self::ReplaceClausesInScope {
                scope,
                clauses: map_clauses(clauses, f)?,
            },
            Self::UpsertClauses { clauses } => Self::UpsertClauses {
                clauses: map_clauses(clauses, f)?,
            },
            Self::ToggleClauses { clauses } => Self::ToggleClauses {
                clauses: map_clauses(clauses, f)?,
            },
            Self::ToggleEqualityValue {
                field_expr,
                value,
                item_id,
                facet_scope,
            } => Self::ToggleEqualityValue {
                field_expr: map_expr_node(field_expr, f, "selection equality toggle field")?,
                value: value.map_exprs(f)?,
                item_id: item_id.map_exprs(f)?,
                facet_scope,
            },
            Self::ReplaceAllFromSceneQuery { query } => Self::ReplaceAllFromSceneQuery {
                query: query.map_exprs(f)?,
            },
            Self::ReplaceFromSceneQueryInScope { query } => Self::ReplaceFromSceneQueryInScope {
                query: query.map_exprs(f)?,
            },
            Self::UpsertFromSceneQuery { query } => Self::UpsertFromSceneQuery {
                query: query.map_exprs(f)?,
            },
            Self::ToggleFromSceneQuery { query } => Self::ToggleFromSceneQuery {
                query: query.map_exprs(f)?,
            },
            Self::DeleteClauses { ids } => Self::DeleteClauses {
                ids: map_values(ids, f)?,
            },
            Self::DeleteClausesInScope { scope, ids } => Self::DeleteClausesInScope {
                scope,
                ids: map_values(ids, f)?,
            },
        })
    }
}

fn map_clauses(
    clauses: Vec<SelectionClauseUpdate>,
    f: &mut impl FnMut(Expr) -> Result<Expr, AvengerChartError>,
) -> Result<Vec<SelectionClauseUpdate>, AvengerChartError> {
    clauses
        .into_iter()
        .map(|clause| clause.map_exprs(f))
        .collect()
}

fn map_values(
    values: Vec<SelectionValueExpr>,
    f: &mut impl FnMut(Expr) -> Result<Expr, AvengerChartError>,
) -> Result<Vec<SelectionValueExpr>, AvengerChartError> {
    values.into_iter().map(|value| value.map_exprs(f)).collect()
}

impl SelectionClauseUpdate {
    pub fn facet_scope(mut self, facet_scope: CoordinationScope) -> Self {
        self.facet_scope = facet_scope;
        self
    }

    pub fn interval(id: impl IntoExpr) -> SelectionIntervalClauseBuilder {
        SelectionIntervalClauseBuilder {
            update: Self {
                id: SelectionValueExpr::new(id),
                facet_scope: CoordinationScope::Free,
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
                facet_scope: CoordinationScope::Free,
                predicate: SelectionPredicateUpdate::Equality {
                    dimensions: Vec::new(),
                },
            },
        }
    }

    pub fn equality_value(
        field_expr: impl IntoExpr,
        value_expr: impl IntoExpr,
    ) -> SelectionClauseUpdate {
        let value_expr = value_expr.into_expr();
        SelectionClauseUpdate::equality(value_expr.clone())
            .dimension(field_expr, value_expr)
            .build()
    }

    pub fn predicate(id: impl IntoExpr) -> SelectionPredicateClauseBuilder {
        SelectionPredicateClauseBuilder {
            id: SelectionValueExpr::new(id),
            facet_scope: CoordinationScope::Free,
            values: Vec::new(),
            kind: None,
        }
    }
}

pub struct SelectionIntervalClauseBuilder {
    update: SelectionClauseUpdate,
}

impl SelectionIntervalClauseBuilder {
    pub fn facet_scope(mut self, facet_scope: CoordinationScope) -> Self {
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
            SelectionPredicateUpdate::Predicate { .. } => {
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
    pub fn facet_scope(mut self, facet_scope: CoordinationScope) -> Self {
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
            SelectionPredicateUpdate::Predicate { .. } => {
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

    pub fn dimension_datum_named(
        self,
        id: impl Into<String>,
        field_name: impl Into<String>,
    ) -> Self {
        let field_name = field_name.into();
        self.dimension_named(
            id,
            datafusion::prelude::col(field_name.clone()),
            crate::event::datum(field_name),
        )
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

pub struct SelectionPredicateClauseBuilder {
    id: SelectionValueExpr,
    facet_scope: CoordinationScope,
    values: Vec<SelectionPredicateValueUpdate>,
    kind: Option<String>,
}

impl SelectionPredicateClauseBuilder {
    pub fn facet_scope(mut self, facet_scope: CoordinationScope) -> Self {
        self.facet_scope = facet_scope;
        self
    }

    pub fn kind(mut self, kind: impl Into<String>) -> Self {
        self.kind = Some(kind.into());
        self
    }

    pub fn value(mut self, id: impl Into<String>, value: impl IntoExpr) -> Self {
        self.values.push(SelectionPredicateValueUpdate {
            id: id.into(),
            value: SelectionValueExpr::new(value),
        });
        self
    }

    pub fn expr(self, expr: impl IntoExpr) -> SelectionClauseUpdate {
        SelectionClauseUpdate {
            id: self.id,
            facet_scope: self.facet_scope,
            predicate: SelectionPredicateUpdate::Predicate {
                values: self.values,
                expr: expr_node(expr.into_expr(), "selection generic predicate expression"),
                kind: self.kind,
            },
        }
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

    /// Whether a one-dimensional equality clause selects `value_expr` for
    /// `field_expr`.
    ///
    /// Unlike [`Self::predicate`], this expression describes visible
    /// membership state. An empty selection therefore evaluates to false even
    /// when the selection uses [`EmptySelectionBehavior::SelectAll`].
    pub fn contains_equality_value(
        &self,
        field_expr: impl IntoExpr,
        value_expr: impl IntoExpr,
    ) -> Expr {
        field_expr
            .into_expr()
            .eq(Expr::Placeholder(Placeholder::new_with_field(
                selection_equality_membership_field_placeholder_id(&self.id),
                None,
            )))
            .and(
                value_expr
                    .into_expr()
                    .eq(Expr::Placeholder(Placeholder::new_with_field(
                        selection_equality_membership_value_placeholder_id(&self.id),
                        None,
                    ))),
            )
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

fn default_clause_facet_scope() -> CoordinationScope {
    CoordinationScope::Free
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledSelectionSpec {
    pub id: String,
    pub empty: EmptySelectionBehavior,
    #[serde(default)]
    pub combine: SelectionCombine,
    #[serde(default)]
    pub facet_context: Vec<SelectionFacetContextSpec>,
}

fn selection_predicate_expr(id: impl AsRef<str>) -> Expr {
    Expr::Placeholder(Placeholder::new_with_field(
        selection_predicate_placeholder_id(id.as_ref()),
        Some(Arc::new(Field::new("", DataType::Boolean, true))),
    ))
}

fn selection_predicate_placeholder_id(id: &str) -> String {
    format!("$__selection_predicate_{id}")
}

pub fn selection_id_from_predicate_placeholder(placeholder_id: &str) -> Option<&str> {
    placeholder_id.strip_prefix("$__selection_predicate_")
}

const SELECTION_EQUALITY_MEMBERSHIP_FIELD_PREFIX: &str = "$__selection_equality_membership_field_";
const SELECTION_EQUALITY_MEMBERSHIP_VALUE_PREFIX: &str = "$__selection_equality_membership_value_";

fn selection_equality_membership_field_placeholder_id(selection_id: &str) -> String {
    format!("{SELECTION_EQUALITY_MEMBERSHIP_FIELD_PREFIX}{selection_id}")
}

fn selection_equality_membership_value_placeholder_id(selection_id: &str) -> String {
    format!("{SELECTION_EQUALITY_MEMBERSHIP_VALUE_PREFIX}{selection_id}")
}

/// Decode the field marker emitted by
/// [`Selection::contains_equality_value`].
///
/// This is public for the chart runtime and event-binding compiler; it is not
/// intended as a chart-authoring API.
#[doc(hidden)]
pub fn selection_id_from_equality_membership_field_placeholder(
    placeholder_id: &str,
) -> Option<&str> {
    placeholder_id.strip_prefix(SELECTION_EQUALITY_MEMBERSHIP_FIELD_PREFIX)
}

/// Decode the value marker emitted by
/// [`Selection::contains_equality_value`].
#[doc(hidden)]
pub fn selection_id_from_equality_membership_value_placeholder(
    placeholder_id: &str,
) -> Option<&str> {
    placeholder_id.strip_prefix(SELECTION_EQUALITY_MEMBERSHIP_VALUE_PREFIX)
}

/// Stable, type-preserving identity for a serialized selection field
/// expression.
///
/// The protobuf representation is the canonical interchange form already used
/// by compiled selections, and URL-safe base64 keeps it usable inside internal
/// placeholder and clause identifiers without lossy string formatting.
#[doc(hidden)]
pub fn selection_field_expr_fingerprint(field_expr: &LogicalExprNode) -> String {
    BASE64_URL_SAFE_NO_PAD.encode(field_expr.encode_to_vec())
}

pub fn clause_value(id: impl AsRef<str>) -> Expr {
    Expr::Placeholder(Placeholder::new_with_field(
        selection_clause_value_placeholder_id(id.as_ref()),
        None,
    ))
}

fn selection_clause_value_placeholder_id(id: &str) -> String {
    format!("$__selection_clause_value_{id}")
}

pub fn selection_clause_value_id_from_placeholder(placeholder_id: &str) -> Option<&str> {
    placeholder_id.strip_prefix("$__selection_clause_value_")
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

fn map_expr_node(
    node: LogicalExprNode,
    f: &mut impl FnMut(Expr) -> Result<Expr, AvengerChartError>,
    label: &str,
) -> Result<LogicalExprNode, AvengerChartError> {
    let expr = node.to_expr(&SessionContext::new())?;
    LogicalExprNode::from_expr(f(expr)?).map_err(|err| {
        AvengerChartError::InternalError(format!("Failed to serialize mapped {label}: {err}"))
    })
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
    fn equality_membership_marker_serializes_and_preserves_field_identity() {
        let field_expr = col("category");
        let expected_field = LogicalExprNode::from_expr(field_expr.clone())
            .expect("serialize expected membership field");
        let expr = Selection::new("picked").contains_equality_value(field_expr, col("__value"));
        LogicalExprNode::from_expr(expr.clone()).expect("membership expression serializes");

        let Expr::BinaryExpr(membership) = expr else {
            panic!("expected equality membership conjunction");
        };
        assert_eq!(membership.op, datafusion::logical_expr::Operator::And);
        let Expr::BinaryExpr(field_membership) = membership.left.as_ref() else {
            panic!("expected field membership marker");
        };
        let Expr::Placeholder(field_placeholder) = field_membership.right.as_ref() else {
            panic!("expected field marker placeholder");
        };
        assert_eq!(
            selection_id_from_equality_membership_field_placeholder(&field_placeholder.id),
            Some("picked")
        );
        assert_eq!(
            selection_field_expr_fingerprint(
                &LogicalExprNode::from_expr(field_membership.left.as_ref().clone())
                    .expect("serialize marker field operand")
            ),
            selection_field_expr_fingerprint(&expected_field),
        );

        let Expr::BinaryExpr(value_membership) = membership.right.as_ref() else {
            panic!("expected value membership marker");
        };
        let Expr::Placeholder(value_placeholder) = value_membership.right.as_ref() else {
            panic!("expected value marker placeholder");
        };
        assert_eq!(
            selection_id_from_equality_membership_value_placeholder(&value_placeholder.id),
            Some("picked")
        );
    }

    #[test]
    fn selection_clause_value_placeholder_serializes_and_is_recognized() {
        let expr = clause_value("cx");
        LogicalExprNode::from_expr(expr.clone()).expect("clause value placeholder serializes");
        let Expr::Placeholder(placeholder) = expr else {
            panic!("expected placeholder expression");
        };
        assert_eq!(
            selection_clause_value_id_from_placeholder(&placeholder.id),
            Some("cx")
        );
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
            .facet_scope(CoordinationScope::Level(1))
            .dimension(col("x"))
            .endpoints(lit(1.0), lit(2.0))
            .dimension_named("vertical", col("y"))
            .endpoints(lit(3.0), lit(4.0))
            .build();
        let json = serde_json::to_string(&clause).expect("serialize clause");
        let restored: SelectionClauseUpdate =
            serde_json::from_str(&json).expect("deserialize clause");
        assert_eq!(restored.facet_scope, CoordinationScope::Level(1));
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
            .facet_scope(CoordinationScope::Level(1))
            .dimension(col("category"), event::datum("category"))
            .dimension_named("region_key", col("region"), event::datum("region"))
            .build();
        let json = serde_json::to_string(&clause).expect("serialize clause");
        let restored: SelectionClauseUpdate =
            serde_json::from_str(&json).expect("deserialize clause");
        assert_eq!(restored.facet_scope, CoordinationScope::Level(1));
        let SelectionPredicateUpdate::Equality { dimensions } = restored.predicate else {
            panic!("expected equality predicate");
        };
        assert_eq!(dimensions.len(), 2);
        assert_eq!(dimensions[0].id, "category");
        assert_eq!(dimensions[1].id, "region_key");
    }

    #[test]
    fn equality_value_clause_uses_value_as_id_and_dimension_value() {
        let clause =
            SelectionClauseUpdate::equality_value(col("category"), event::datum("category"))
                .facet_scope(CoordinationScope::Shared);
        let json = serde_json::to_string(&clause).expect("serialize clause");
        let restored: SelectionClauseUpdate =
            serde_json::from_str(&json).expect("deserialize clause");
        assert_eq!(restored.facet_scope, CoordinationScope::Shared);
        let restored_id = restored.id.clone();
        let SelectionPredicateUpdate::Equality { dimensions } = restored.predicate else {
            panic!("expected equality predicate");
        };
        assert_eq!(dimensions.len(), 1);
        assert_eq!(dimensions[0].id, "category");
        assert_eq!(
            dimensions[0].value, restored_id,
            "the clause id and equality value should be the same expression"
        );
    }

    #[test]
    fn equality_value_toggle_accepts_explicit_scope() {
        let update = SelectionUpdate::toggle_equality_value_in_scope(
            CoordinationScope::Shared,
            col("category"),
            event::datum("__value"),
            event::datum("__item_id"),
        );
        let json = serde_json::to_string(&update).expect("serialize selection update");
        let restored: SelectionUpdate =
            serde_json::from_str(&json).expect("deserialize selection update");
        let SelectionUpdate::ToggleEqualityValue { facet_scope, .. } = restored else {
            panic!("expected equality-value toggle");
        };
        assert_eq!(facet_scope, CoordinationScope::Shared);
    }

    #[test]
    fn generic_predicate_clause_update_builder_serializes() {
        let clause = SelectionClauseUpdate::predicate("active")
            .facet_scope(CoordinationScope::Level(1))
            .kind("circle")
            .value("cx", event::start_coord("x"))
            .value("r2", lit(4.0))
            .expr(col("source_x").gt_eq(clause_value("cx")));
        let json = serde_json::to_string(&clause).expect("serialize clause");
        let restored: SelectionClauseUpdate =
            serde_json::from_str(&json).expect("deserialize clause");
        assert_eq!(restored.facet_scope, CoordinationScope::Level(1));
        let SelectionPredicateUpdate::Predicate { values, kind, .. } = restored.predicate else {
            panic!("expected generic predicate");
        };
        assert_eq!(kind.as_deref(), Some("circle"));
        assert_eq!(values.len(), 2);
        assert_eq!(values[0].id, "cx");
        assert_eq!(values[1].id, "r2");
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

    #[test]
    fn toggle_equality_value_update_serializes_typed_inputs() {
        let update = SelectionUpdate::toggle_equality_value(
            col("category"),
            event::datum("__value"),
            event::datum("__item_id"),
        );
        let json = serde_json::to_string(&update).expect("serialize update");
        let restored: SelectionUpdate = serde_json::from_str(&json).expect("deserialize update");
        let SelectionUpdate::ToggleEqualityValue {
            field_expr,
            value,
            item_id,
            facet_scope,
        } = restored
        else {
            panic!("expected equality value toggle");
        };
        assert_eq!(facet_scope, CoordinationScope::Free);
        assert_eq!(
            selection_field_expr_fingerprint(&field_expr),
            selection_field_expr_fingerprint(&expr_node(col("category"), "test field"))
        );
        assert_ne!(value, item_id);
    }

    #[test]
    fn scoped_delete_update_serializes() {
        let update = SelectionUpdate::delete_clauses_in_scope(
            CoordinationScope::Level(1),
            [event::datum("category"), lit("fallback")],
        );
        let json = serde_json::to_string(&update).expect("serialize update");
        let restored: SelectionUpdate = serde_json::from_str(&json).expect("deserialize update");
        let SelectionUpdate::DeleteClausesInScope { scope, ids } = restored else {
            panic!("expected scoped delete clauses");
        };
        assert_eq!(scope, CoordinationScope::Level(1));
        assert_eq!(ids.len(), 2);
    }
}
