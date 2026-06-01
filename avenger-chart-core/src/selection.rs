use std::sync::Arc;

use datafusion::{
    arrow::datatypes::{DataType, Field, FieldRef, Fields},
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
    SerializableDataType, SerializableExpr, Sharing,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectionCombine {
    #[default]
    Union,
    Intersect,
}

#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelectionGeometryFieldSpec {
    pub name: String,
    #[serde_as(as = "FromInto<SerializableDataType>")]
    pub data_type: DataType,
    pub nullable: bool,
}

impl SelectionGeometryFieldSpec {
    pub fn to_field_ref(&self) -> FieldRef {
        Arc::new(Field::new(
            self.name.clone(),
            self.data_type.clone(),
            self.nullable,
        ))
    }
}

impl From<FieldRef> for SelectionGeometryFieldSpec {
    fn from(field: FieldRef) -> Self {
        Self {
            name: field.name().clone(),
            data_type: field.data_type().clone(),
            nullable: field.is_nullable(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SelectionGeometrySchema {
    #[serde(default)]
    pub fields: Vec<SelectionGeometryFieldSpec>,
}

impl SelectionGeometrySchema {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn field(mut self, field: FieldRef) -> Self {
        self.fields.push(field.into());
        self
    }

    pub fn cartesian_rect() -> Self {
        Self::new().field(cartesian_rect_field())
    }

    pub fn field_named(&self, name: &str) -> Option<&SelectionGeometryFieldSpec> {
        self.fields.iter().find(|field| field.name == name)
    }
}

pub const CARTESIAN_RECT_GEOMETRY_COLUMN: &str = "CartesianRect";

pub fn cartesian_rect_field() -> FieldRef {
    Arc::new(Field::new(
        CARTESIAN_RECT_GEOMETRY_COLUMN,
        DataType::Struct(Fields::from(vec![
            Field::new("x_min", DataType::Float64, true),
            Field::new("x_max", DataType::Float64, true),
            Field::new("y_min", DataType::Float64, true),
            Field::new("y_max", DataType::Float64, true),
        ])),
        true,
    ))
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
    #[serde(default)]
    pub combine: SelectionCombine,
    #[serde(default = "default_selection_sharing")]
    pub sharing: Sharing,
    #[serde(default)]
    pub geometry_schema: SelectionGeometrySchema,
    pub dimensions: Vec<SelectionDimensionSpec>,
    #[serde(default)]
    pub facet_context: Vec<SelectionFacetContextSpec>,
}

impl Selection {
    pub fn new(id: impl Into<String>) -> Self {
        Self::single(id)
    }

    pub fn single(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            resolution: SelectionResolution::Single,
            empty: SelectionEmpty::None,
            combine: SelectionCombine::Union,
            sharing: Sharing::Free,
            geometry_schema: SelectionGeometrySchema::default(),
            dimensions: Vec::new(),
            facet_context: Vec::new(),
        }
    }

    pub fn empty(mut self, empty: SelectionEmpty) -> Self {
        self.empty = empty;
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

    pub fn geometry(mut self, schema: SelectionGeometrySchema) -> Self {
        self.geometry_schema = schema;
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
        if self
            .geometry_schema
            .field_named(CARTESIAN_RECT_GEOMETRY_COLUMN)
            .is_none()
        {
            self.geometry_schema = self.geometry_schema.field(cartesian_rect_field());
        }
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
        if self
            .geometry_schema
            .field_named(CARTESIAN_RECT_GEOMETRY_COLUMN)
            .is_none()
        {
            self.geometry_schema = self.geometry_schema.field(cartesian_rect_field());
        }
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
        selection_predicate(&self.id)
    }

    pub fn clauses(&self) -> SelectionClauseData {
        selection_clauses(&self.id)
    }

    pub fn dimension_value_expr(&self, id: &str) -> Expr {
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
        validate_geometry_schema(&self.id, &self.geometry_schema)?;
        Ok(CompiledSelectionSpec {
            id: self.id.clone(),
            resolution: self.resolution,
            empty: self.empty,
            dimensions: self.dimensions.clone(),
            combine: self.combine,
            sharing: self.sharing,
            geometry_schema: self.geometry_schema.clone(),
            facet_context: self.facet_context.clone(),
            lowered_params: SelectionLoweredParams::with_facet_context(
                &self.id,
                self.facet_context.iter().map(|facet| facet.id.as_str()),
            ),
        })
    }
}

fn validate_geometry_schema(
    selection_id: &str,
    schema: &SelectionGeometrySchema,
) -> Result<(), AvengerChartError> {
    let mut names = std::collections::HashSet::new();
    for field in &schema.fields {
        if !names.insert(field.name.clone()) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Selection '{selection_id}' declares duplicate geometry column '{}'",
                field.name
            )));
        }
        if !matches!(&field.data_type, DataType::Struct(_)) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Selection '{selection_id}' geometry column '{}' must be an Arrow Struct",
                field.name
            )));
        }
    }
    Ok(())
}

fn default_selection_sharing() -> Sharing {
    Sharing::Free
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
    pub combine: SelectionCombine,
    #[serde(default = "default_selection_sharing")]
    pub sharing: Sharing,
    #[serde(default)]
    pub geometry_schema: SelectionGeometrySchema,
    #[serde(default)]
    pub facet_context: Vec<SelectionFacetContextSpec>,
    pub lowered_params: SelectionLoweredParams,
}

impl CompiledSelectionSpec {
    pub fn dimension_value_expr(&self, id: &str) -> Expr {
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelectionClauseData {
    pub selection_id: String,
    #[serde(default)]
    pub matching_current_facet: bool,
}

impl SelectionClauseData {
    pub fn matching_current_facet(mut self) -> Self {
        self.matching_current_facet = true;
        self
    }
}

pub fn selection_clauses(id: impl AsRef<str>) -> SelectionClauseData {
    SelectionClauseData {
        selection_id: id.as_ref().to_string(),
        matching_current_facet: false,
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

#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelectionValueExpr {
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub expr: LogicalExprNode,
}

impl SelectionValueExpr {
    pub fn new(expr: impl IntoExpr) -> Self {
        Self {
            expr: expr_node(expr.into_expr(), "selection value"),
        }
    }
}

#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SelectionPredicateUpdate {
    Range2D {
        #[serde_as(as = "FromInto<SerializableExpr>")]
        x_field: LogicalExprNode,
        #[serde_as(as = "FromInto<SerializableExpr>")]
        y_field: LogicalExprNode,
        x_range: SelectionRangeExpr,
        y_range: SelectionRangeExpr,
    },
}

impl SelectionPredicateUpdate {
    pub fn range_2d() -> SelectionRange2DUpdateBuilder {
        SelectionRange2DUpdateBuilder::default()
    }
}

#[derive(Default)]
pub struct SelectionRange2DUpdateBuilder {
    x_field: Option<LogicalExprNode>,
    y_field: Option<LogicalExprNode>,
    x_range: Option<SelectionRangeExpr>,
    y_range: Option<SelectionRangeExpr>,
}

impl SelectionRange2DUpdateBuilder {
    pub fn x_field(mut self, expr: impl IntoExpr) -> Self {
        self.x_field = Some(expr_node(expr.into_expr(), "selection x predicate field"));
        self
    }

    pub fn y_field(mut self, expr: impl IntoExpr) -> Self {
        self.y_field = Some(expr_node(expr.into_expr(), "selection y predicate field"));
        self
    }

    pub fn x_range(mut self, expr: impl IntoExpr) -> Self {
        self.x_range = Some(SelectionRangeExpr::new(expr));
        self
    }

    pub fn y_range(mut self, expr: impl IntoExpr) -> Self {
        self.y_range = Some(SelectionRangeExpr::new(expr));
        self
    }

    pub fn build(self) -> Result<SelectionPredicateUpdate, AvengerChartError> {
        Ok(SelectionPredicateUpdate::Range2D {
            x_field: self
                .x_field
                .unwrap_or_else(|| expr_node(col(":x"), "default selection x field")),
            y_field: self
                .y_field
                .unwrap_or_else(|| expr_node(col(":y"), "default selection y field")),
            x_range: self.x_range.ok_or_else(|| {
                AvengerChartError::InvalidArgument(
                    "Range2D selection predicate update requires an x range".to_string(),
                )
            })?,
            y_range: self.y_range.ok_or_else(|| {
                AvengerChartError::InvalidArgument(
                    "Range2D selection predicate update requires a y range".to_string(),
                )
            })?,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelectionGeometryUpdate {
    pub column_name: String,
    #[serde(default)]
    pub fields: IndexMap<String, SelectionValueExpr>,
}

impl SelectionGeometryUpdate {
    pub fn new(column_name: impl Into<String>) -> Self {
        Self {
            column_name: column_name.into(),
            fields: IndexMap::new(),
        }
    }

    pub fn field(mut self, name: impl Into<String>, expr: impl IntoExpr) -> Self {
        self.fields
            .insert(name.into(), SelectionValueExpr::new(expr));
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelectionClauseUpdate {
    pub clause_id: Option<SelectionValueExpr>,
    pub predicate: SelectionPredicateUpdate,
    pub geometry: Option<SelectionGeometryUpdate>,
    pub owner_facet_context_from_start: bool,
}

impl SelectionClauseUpdate {
    pub fn new(predicate: SelectionPredicateUpdate) -> Self {
        Self {
            clause_id: None,
            predicate,
            geometry: None,
            owner_facet_context_from_start: false,
        }
    }

    pub fn clause_id(mut self, expr: impl IntoExpr) -> Self {
        self.clause_id = Some(SelectionValueExpr::new(expr));
        self
    }

    pub fn geometry(mut self, geometry: SelectionGeometryUpdate) -> Self {
        self.geometry = Some(geometry);
        self
    }

    pub fn owner_facet_context_from_start(mut self) -> Self {
        self.owner_facet_context_from_start = true;
        self
    }
}

#[derive(Clone, Debug)]
pub enum SelectionPredicateSpec {
    Range2D {
        x_field: LogicalExprNode,
        y_field: LogicalExprNode,
        x_min: ScalarValue,
        x_max: ScalarValue,
        y_min: ScalarValue,
        y_max: ScalarValue,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SelectionGeometryValue {
    pub column_name: String,
    pub fields: IndexMap<String, ScalarValue>,
}

#[derive(Clone, Debug)]
pub struct SelectionClause {
    pub clause_id: String,
    pub source_scope_id: Option<String>,
    pub predicate: SelectionPredicateSpec,
    pub geometry: Option<SelectionGeometryValue>,
    pub owner_path: Vec<ScalarValue>,
    pub owner_facet_values: Vec<ScalarValue>,
}

#[derive(Clone, Debug, Default)]
pub struct SelectionState {
    pub id: String,
    pub clauses: Vec<SelectionClause>,
    pub revision: u64,
}

#[derive(Clone, Debug)]
pub enum SelectionStateUpdate {
    Clear,
    ReplaceClause(SelectionClause),
    AddClause(SelectionClause),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SelectionUpdateKind {
    Interval,
    ReplaceClause,
    AddClause,
    Clear,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelectionUpdate {
    pub kind: SelectionUpdateKind,
    pub source: Option<String>,
    #[serde(default)]
    pub clause_id: Option<SelectionValueExpr>,
    pub x_range: Option<SelectionRangeExpr>,
    pub y_range: Option<SelectionRangeExpr>,
    pub capture_facet_context: bool,
    pub meta: Option<SelectionClauseMeta>,
    pub clause: Option<SelectionClauseUpdate>,
}

impl SelectionUpdate {
    pub fn interval_xy() -> Self {
        Self {
            kind: SelectionUpdateKind::Interval,
            source: None,
            clause_id: None,
            x_range: None,
            y_range: None,
            capture_facet_context: false,
            meta: Some(SelectionClauseMeta {
                kind: "interval".to_string(),
            }),
            clause: None,
        }
    }

    pub fn add_interval_xy() -> Self {
        Self {
            kind: SelectionUpdateKind::AddClause,
            ..Self::interval_xy()
        }
    }

    pub fn clear() -> Self {
        Self {
            kind: SelectionUpdateKind::Clear,
            source: None,
            clause_id: None,
            x_range: None,
            y_range: None,
            capture_facet_context: false,
            meta: None,
            clause: None,
        }
    }

    pub fn replace_clause(clause: SelectionClauseUpdate) -> Self {
        Self {
            kind: SelectionUpdateKind::ReplaceClause,
            source: None,
            clause_id: None,
            x_range: None,
            y_range: None,
            capture_facet_context: clause.owner_facet_context_from_start,
            meta: None,
            clause: Some(clause),
        }
    }

    pub fn add_clause(clause: SelectionClauseUpdate) -> Self {
        Self {
            kind: SelectionUpdateKind::AddClause,
            source: None,
            clause_id: None,
            x_range: None,
            y_range: None,
            capture_facet_context: clause.owner_facet_context_from_start,
            meta: None,
            clause: Some(clause),
        }
    }

    pub fn source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn clause_id(mut self, expr: impl IntoExpr) -> Self {
        self.clause_id = Some(SelectionValueExpr::new(expr));
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

    pub fn as_clause_update(
        &self,
        selection: &CompiledSelectionSpec,
    ) -> Result<Option<SelectionClauseUpdate>, AvengerChartError> {
        match self.kind {
            SelectionUpdateKind::Clear => Ok(None),
            SelectionUpdateKind::ReplaceClause => self
                .clause
                .clone()
                .map(Ok)
                .unwrap_or_else(|| {
                    Err(AvengerChartError::InvalidArgument(
                        "Selection clause update is missing its clause payload".to_string(),
                    ))
                })
                .map(Some),
            SelectionUpdateKind::AddClause if self.clause.is_some() => Ok(self.clause.clone()),
            SelectionUpdateKind::Interval | SelectionUpdateKind::AddClause => {
                let Some(x_range) = &self.x_range else {
                    return Err(AvengerChartError::InvalidArgument(
                        "Interval selection update is missing an x range".to_string(),
                    ));
                };
                let Some(y_range) = &self.y_range else {
                    return Err(AvengerChartError::InvalidArgument(
                        "Interval selection update is missing a y range".to_string(),
                    ));
                };
                let predicate = SelectionPredicateUpdate::Range2D {
                    x_field: expr_node(
                        selection.dimension_value_expr("x"),
                        "selection interval x field",
                    ),
                    y_field: expr_node(
                        selection.dimension_value_expr("y"),
                        "selection interval y field",
                    ),
                    x_range: x_range.clone(),
                    y_range: y_range.clone(),
                };
                let geometry = SelectionGeometryUpdate::new(CARTESIAN_RECT_GEOMETRY_COLUMN)
                    .field(
                        "x_min",
                        interval_start(x_range.expr.to_expr(&SessionContext::new())?),
                    )
                    .field(
                        "x_max",
                        interval_end(x_range.expr.to_expr(&SessionContext::new())?),
                    )
                    .field(
                        "y_min",
                        interval_start(y_range.expr.to_expr(&SessionContext::new())?),
                    )
                    .field(
                        "y_max",
                        interval_end(y_range.expr.to_expr(&SessionContext::new())?),
                    );
                Ok(Some(SelectionClauseUpdate {
                    clause_id: self.clause_id.clone(),
                    predicate,
                    geometry: Some(geometry),
                    owner_facet_context_from_start: self.capture_facet_context,
                }))
            }
        }
    }
}

pub fn selection_predicate(id: impl AsRef<str>) -> Expr {
    Expr::Placeholder(Placeholder {
        id: selection_predicate_placeholder_id(id.as_ref()),
        data_type: Some(DataType::Boolean),
    })
}

pub fn selection_predicate_placeholder_id(id: &str) -> String {
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
        SelectionUpdateKind::Interval
        | SelectionUpdateKind::ReplaceClause
        | SelectionUpdateKind::AddClause => {
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
