use std::ops::Not;

use datafusion::{
    arrow::datatypes::DataType,
    logical_expr::cast,
    logical_expr::expr::Placeholder,
    prelude::{Expr, SessionContext, col, lit},
    scalar::ScalarValue,
};
use datafusion_proto::protobuf::LogicalExprNode;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::{
    AvengerChartError, CompiledParamSpec, DefaultLogicalExprNodeExt, IntoExpr, Param,
    SerializableExpr,
    event::{
        ChartEventParamAssignment, ChartEventSelectionAssignment, interval_end, interval_start,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectionResolution {
    Single,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectionEmpty {
    All,
    None,
}

#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelectionDimensionSpec {
    pub id: String,
    pub channel: Option<String>,
    #[serde_as(as = "Option<FromInto<SerializableExpr>>")]
    pub field_expr: Option<LogicalExprNode>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelectionClauseMeta {
    pub kind: String,
}

#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelectionFacetContextSpec {
    pub id: String,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub field_expr: LogicalExprNode,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Selection {
    pub id: String,
    pub resolution: SelectionResolution,
    pub empty: SelectionEmpty,
    pub dimensions: Vec<SelectionDimensionSpec>,
    #[serde(default)]
    pub facet_context: Vec<SelectionFacetContextSpec>,
}

impl Selection {
    pub fn single(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            resolution: SelectionResolution::Single,
            empty: SelectionEmpty::None,
            dimensions: Vec::new(),
            facet_context: Vec::new(),
        }
    }

    pub fn empty(mut self, empty: SelectionEmpty) -> Self {
        self.empty = empty;
        self
    }

    pub fn interval_xy(
        mut self,
        x_channel: impl Into<String>,
        y_channel: impl Into<String>,
    ) -> Self {
        self.dimensions = vec![
            SelectionDimensionSpec {
                id: "x".to_string(),
                channel: Some(x_channel.into()),
                field_expr: None,
            },
            SelectionDimensionSpec {
                id: "y".to_string(),
                channel: Some(y_channel.into()),
                field_expr: None,
            },
        ];
        self
    }

    pub fn interval_fields<X, Y>(mut self, x: (&str, X), y: (&str, Y)) -> Self
    where
        X: IntoExpr,
        Y: IntoExpr,
    {
        self.dimensions = vec![
            SelectionDimensionSpec {
                id: x.0.to_string(),
                channel: None,
                field_expr: Some(expr_node(x.1.into_expr(), "selection x field")),
            },
            SelectionDimensionSpec {
                id: y.0.to_string(),
                channel: None,
                field_expr: Some(expr_node(y.1.into_expr(), "selection y field")),
            },
        ];
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
        let lowered = SelectionLoweredParams::with_facet_context(
            &self.id,
            self.facet_context.iter().map(|facet| facet.id.as_str()),
        );
        selection_predicate_expr(
            param_expr(&lowered.active, DataType::Boolean).eq(lit(true)),
            self.dimension_value_expr("x"),
            self.dimension_value_expr("y"),
            param_expr(&lowered.x_min, DataType::Float64),
            param_expr(&lowered.x_max, DataType::Float64),
            param_expr(&lowered.y_min, DataType::Float64),
            param_expr(&lowered.y_max, DataType::Float64),
            param_expr(&lowered.empty_selected, DataType::Boolean),
            self.facet_context
                .iter()
                .zip(lowered.facet_values.iter())
                .map(|(facet, param_name)| {
                    let expr = facet
                        .field_expr
                        .to_expr(&SessionContext::new())
                        .expect("selection facet context field expr deserializes");
                    cast(expr, DataType::Utf8).eq(param_expr(param_name, DataType::Utf8))
                })
                .collect(),
        )
    }

    fn dimension_value_expr(&self, id: &str) -> Expr {
        self.dimensions
            .iter()
            .find(|dimension| dimension.id == id)
            .and_then(|dimension| {
                dimension
                    .field_expr
                    .as_ref()
                    .and_then(|expr| expr.to_expr(&SessionContext::new()).ok())
                    .or_else(|| {
                        dimension
                            .channel
                            .as_ref()
                            .map(|channel| col(format!(":{channel}")))
                    })
            })
            .unwrap_or_else(|| col(format!(":{id}")))
    }

    pub fn compile(&self) -> Result<CompiledSelectionSpec, AvengerChartError> {
        validate_selection_id(&self.id)?;
        for facet in &self.facet_context {
            validate_selection_id(&facet.id)?;
        }
        if self.resolution != SelectionResolution::Single {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Selection '{}' uses an unsupported resolution",
                self.id
            )));
        }
        if self.dimensions.len() != 2 {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Selection '{}' must define exactly two interval dimensions",
                self.id
            )));
        }
        Ok(CompiledSelectionSpec {
            id: self.id.clone(),
            resolution: self.resolution,
            empty: self.empty,
            dimensions: self.dimensions.clone(),
            facet_context: self.facet_context.clone(),
            lowered_params: SelectionLoweredParams::with_facet_context(
                &self.id,
                self.facet_context.iter().map(|facet| facet.id.as_str()),
            ),
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelectionLoweredParams {
    pub active: String,
    pub empty_selected: String,
    pub x_min: String,
    pub x_max: String,
    pub y_min: String,
    pub y_max: String,
    pub facet_values: Vec<String>,
}

impl SelectionLoweredParams {
    pub fn new(id: &str) -> Self {
        Self::with_facet_context(id, std::iter::empty::<&str>())
    }

    pub fn with_facet_context<'a>(id: &str, facet_ids: impl IntoIterator<Item = &'a str>) -> Self {
        Self {
            active: selection_param_name(id, "active"),
            empty_selected: selection_param_name(id, "empty_selected"),
            x_min: selection_param_name(id, "x_min"),
            x_max: selection_param_name(id, "x_max"),
            y_min: selection_param_name(id, "y_min"),
            y_max: selection_param_name(id, "y_max"),
            facet_values: facet_ids
                .into_iter()
                .map(|facet_id| selection_param_name(id, &format!("facet_{facet_id}")))
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompiledSelectionSpec {
    pub id: String,
    pub resolution: SelectionResolution,
    pub empty: SelectionEmpty,
    pub dimensions: Vec<SelectionDimensionSpec>,
    #[serde(default)]
    pub facet_context: Vec<SelectionFacetContextSpec>,
    pub lowered_params: SelectionLoweredParams,
}

impl CompiledSelectionSpec {
    pub fn hidden_param_specs(&self) -> Vec<CompiledParamSpec> {
        let mut specs = vec![
            CompiledParamSpec::shared(&Param::new(
                self.lowered_params.active.clone(),
                ScalarValue::Boolean(Some(false)),
            )),
            CompiledParamSpec::shared(&Param::new(
                self.lowered_params.empty_selected.clone(),
                ScalarValue::Boolean(Some(matches!(self.empty, SelectionEmpty::All))),
            )),
            CompiledParamSpec::shared(&Param::new(
                self.lowered_params.x_min.clone(),
                ScalarValue::Float64(None),
            )),
            CompiledParamSpec::shared(&Param::new(
                self.lowered_params.x_max.clone(),
                ScalarValue::Float64(None),
            )),
            CompiledParamSpec::shared(&Param::new(
                self.lowered_params.y_min.clone(),
                ScalarValue::Float64(None),
            )),
            CompiledParamSpec::shared(&Param::new(
                self.lowered_params.y_max.clone(),
                ScalarValue::Float64(None),
            )),
        ];
        specs.extend(self.lowered_params.facet_values.iter().map(|param| {
            CompiledParamSpec::shared(&Param::new(param.clone(), ScalarValue::Utf8(None)))
        }));
        specs
    }
}

#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelectionRangeExpr {
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub expr: LogicalExprNode,
}

impl SelectionRangeExpr {
    pub fn new(expr: impl IntoExpr) -> Self {
        Self {
            expr: expr_node(expr.into_expr(), "selection range"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SelectionUpdateKind {
    Interval,
    Clear,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelectionUpdate {
    pub kind: SelectionUpdateKind,
    pub source: Option<String>,
    pub x_range: Option<SelectionRangeExpr>,
    pub y_range: Option<SelectionRangeExpr>,
    pub capture_facet_context: bool,
    pub meta: Option<SelectionClauseMeta>,
}

impl SelectionUpdate {
    pub fn interval_xy() -> Self {
        Self {
            kind: SelectionUpdateKind::Interval,
            source: None,
            x_range: None,
            y_range: None,
            capture_facet_context: false,
            meta: Some(SelectionClauseMeta {
                kind: "interval".to_string(),
            }),
        }
    }

    pub fn clear() -> Self {
        Self {
            kind: SelectionUpdateKind::Clear,
            source: None,
            x_range: None,
            y_range: None,
            capture_facet_context: false,
            meta: None,
        }
    }

    pub fn source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn x_range(mut self, range: impl IntoExpr) -> Self {
        self.x_range = Some(SelectionRangeExpr::new(range));
        self
    }

    pub fn y_range(mut self, range: impl IntoExpr) -> Self {
        self.y_range = Some(SelectionRangeExpr::new(range));
        self
    }

    pub fn facet_context_from_start(mut self) -> Self {
        self.capture_facet_context = true;
        self
    }
}

pub fn selection_predicate(id: impl AsRef<str>) -> Expr {
    let lowered = SelectionLoweredParams::new(id.as_ref());
    let active = param_expr(&lowered.active, DataType::Boolean).eq(lit(true));
    let x_value = col(":x");
    let y_value = col(":y");
    selection_predicate_expr(
        active,
        x_value,
        y_value,
        param_expr(&lowered.x_min, DataType::Float64),
        param_expr(&lowered.x_max, DataType::Float64),
        param_expr(&lowered.y_min, DataType::Float64),
        param_expr(&lowered.y_max, DataType::Float64),
        param_expr(&lowered.empty_selected, DataType::Boolean),
        Vec::new(),
    )
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

pub fn lower_selection_assignments(
    assignment: &ChartEventSelectionAssignment,
    selection: &CompiledSelectionSpec,
    ctx: &SessionContext,
) -> Result<Vec<ChartEventParamAssignment>, AvengerChartError> {
    let params = &selection.lowered_params;
    match assignment.update.kind {
        SelectionUpdateKind::Clear => Ok(vec![ChartEventParamAssignment {
            param_name: params.active.clone(),
            expr: expr_node(lit(false), "selection clear active"),
            scope: assignment.scope,
            replace_scoped_values: false,
        }]),
        SelectionUpdateKind::Interval => {
            let Some(x_range) = &assignment.update.x_range else {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Selection update for '{}' is missing an x range",
                    assignment.selection_id
                )));
            };
            let Some(y_range) = &assignment.update.y_range else {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Selection update for '{}' is missing a y range",
                    assignment.selection_id
                )));
            };
            let x_expr = x_range.expr.to_expr(ctx)?;
            let y_expr = y_range.expr.to_expr(ctx)?;
            let mut lowered = vec![
                ChartEventParamAssignment {
                    param_name: params.active.clone(),
                    expr: expr_node(lit(true), "selection interval active"),
                    scope: assignment.scope,
                    replace_scoped_values: false,
                },
                ChartEventParamAssignment {
                    param_name: params.x_min.clone(),
                    expr: expr_node(interval_start(x_expr.clone()), "selection x min"),
                    scope: assignment.scope,
                    replace_scoped_values: false,
                },
                ChartEventParamAssignment {
                    param_name: params.x_max.clone(),
                    expr: expr_node(interval_end(x_expr), "selection x max"),
                    scope: assignment.scope,
                    replace_scoped_values: false,
                },
                ChartEventParamAssignment {
                    param_name: params.y_min.clone(),
                    expr: expr_node(interval_start(y_expr.clone()), "selection y min"),
                    scope: assignment.scope,
                    replace_scoped_values: false,
                },
                ChartEventParamAssignment {
                    param_name: params.y_max.clone(),
                    expr: expr_node(interval_end(y_expr), "selection y max"),
                    scope: assignment.scope,
                    replace_scoped_values: false,
                },
            ];
            if assignment.update.capture_facet_context {
                for (index, param_name) in params.facet_values.iter().enumerate() {
                    lowered.push(ChartEventParamAssignment {
                        param_name: param_name.clone(),
                        expr: expr_node(
                            crate::event::start_facet_value(index),
                            "selection facet context value",
                        ),
                        scope: assignment.scope,
                        replace_scoped_values: false,
                    });
                }
            }
            Ok(lowered)
        }
    }
}

fn selection_predicate_expr(
    active: Expr,
    x_value: Expr,
    y_value: Expr,
    x_min: Expr,
    x_max: Expr,
    y_min: Expr,
    y_max: Expr,
    empty_selected: Expr,
    facet_context: Vec<Expr>,
) -> Expr {
    let mut inside = x_value
        .clone()
        .gt_eq(x_min)
        .and(x_value.lt_eq(x_max))
        .and(y_value.clone().gt_eq(y_min))
        .and(y_value.lt_eq(y_max));
    for facet_expr in facet_context {
        inside = inside.and(facet_expr);
    }
    active
        .clone()
        .and(inside)
        .or(active.not().and(empty_selected))
}

fn param_expr(name: &str, data_type: DataType) -> Expr {
    Expr::Placeholder(Placeholder {
        id: format!("${name}"),
        data_type: Some(data_type),
    })
}

fn selection_param_name(id: &str, suffix: &str) -> String {
    format!("__selection_{id}__{suffix}")
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
    use datafusion::prelude::SessionContext;

    use super::*;
    use crate::{
        event::{self, ChartEventAssignmentScope},
        serialization::DefaultLogicalExprNodeExt,
    };

    #[test]
    fn single_interval_selection_compiles_hidden_params() {
        let selection = Selection::single("brush")
            .empty(SelectionEmpty::All)
            .interval_xy("x", "y");
        let compiled = selection.compile().expect("compile selection");
        assert_eq!(compiled.id, "brush");
        assert_eq!(compiled.dimensions.len(), 2);
        let params = compiled.hidden_param_specs();
        assert_eq!(params.len(), 6);
        assert_eq!(params[0].name, "__selection_brush__active");
        assert_eq!(
            params[1].default,
            ScalarValue::Boolean(Some(true)),
            "empty/all is represented by the lowered empty-selected param"
        );
    }

    #[test]
    fn lower_interval_selection_update_to_param_assignments() {
        let ctx = SessionContext::new();
        let selection = Selection::single("brush").interval_xy("x", "y");
        let compiled = selection.compile().expect("compile selection");
        let assignment = ChartEventSelectionAssignment {
            selection_id: "brush".to_string(),
            update: SelectionUpdate::interval_xy()
                .x_range(event::interval(lit(1.0), lit(4.0)))
                .y_range(event::interval(lit(2.0), lit(5.0))),
            scope: ChartEventAssignmentScope::Start,
        };
        let lowered = lower_selection_assignments(&assignment, &compiled, &ctx).expect("lower");
        assert_eq!(lowered.len(), 5);
        assert!(
            lowered
                .iter()
                .all(|a| a.scope == ChartEventAssignmentScope::Start)
        );
        assert_eq!(lowered[0].param_name, "__selection_brush__active");
        assert_eq!(lowered[1].param_name, "__selection_brush__x_min");
        assert_eq!(lowered[2].param_name, "__selection_brush__x_max");
        for assignment in lowered {
            assignment
                .expr
                .to_expr(&ctx)
                .expect("lowered selection assignment expr deserializes");
        }
    }

    #[test]
    fn selection_predicate_serializes() {
        let expr = selection_predicate("brush");
        LogicalExprNode::from_expr(expr).expect("selection predicate serializes");
    }

    #[test]
    fn facet_context_selection_compiles_hidden_params_and_predicate() {
        let selection = Selection::single("brush")
            .interval_xy("x", "y")
            .facet_context_field("group_name", col("group_name"));
        let compiled = selection.compile().expect("compile selection");
        let params = compiled.hidden_param_specs();
        assert_eq!(params.len(), 7);
        assert_eq!(
            params.last().unwrap().name,
            "__selection_brush__facet_group_name"
        );
        LogicalExprNode::from_expr(selection.predicate())
            .expect("facet-context selection predicate serializes");
    }
}
