use datafusion::prelude::{Expr, SessionContext, col};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::{
    AvengerChartError, CoordinationScope, DefaultLogicalExprNodeExt, IntoExpr, SerializableExpr,
    validate_mark_target_path, validate_structural_id,
};

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneQueryDatumField {
    pub id: String,
    pub datum_field: String,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub field_expr: LogicalExprNode,
}

impl SceneQueryDatumField {
    pub fn new(field: impl Into<String>) -> Self {
        let field = field.into();
        Self {
            id: field.clone(),
            datum_field: field.clone(),
            field_expr: expr_node(col(field), "scene query datum field expression"),
        }
    }

    pub fn datum(mut self, field: impl Into<String>) -> Self {
        self.datum_field = field.into();
        self
    }

    pub fn field_expr(mut self, expr: impl IntoExpr) -> Self {
        self.field_expr = expr_node(expr.into_expr(), "scene query datum field expression");
        self
    }

    pub(crate) fn map_exprs(
        self,
        f: &mut impl FnMut(Expr) -> Result<Expr, AvengerChartError>,
    ) -> Result<Self, AvengerChartError> {
        Ok(Self {
            id: self.id,
            datum_field: self.datum_field,
            field_expr: map_expr_node(self.field_expr, f, "scene query datum field")?,
        })
    }
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SceneGeometryQueryGeometry {
    Rect {
        #[serde_as(as = "FromInto<SerializableExpr>")]
        x0: LogicalExprNode,
        #[serde_as(as = "FromInto<SerializableExpr>")]
        y0: LogicalExprNode,
        #[serde_as(as = "FromInto<SerializableExpr>")]
        x1: LogicalExprNode,
        #[serde_as(as = "FromInto<SerializableExpr>")]
        y1: LogicalExprNode,
    },
    Circle {
        #[serde_as(as = "FromInto<SerializableExpr>")]
        cx: LogicalExprNode,
        #[serde_as(as = "FromInto<SerializableExpr>")]
        cy: LogicalExprNode,
        #[serde_as(as = "FromInto<SerializableExpr>")]
        radius: LogicalExprNode,
    },
    Polygon {
        #[serde_as(as = "FromInto<SerializableExpr>")]
        points: LogicalExprNode,
    },
}

impl SceneGeometryQueryGeometry {
    pub(crate) fn map_exprs(
        self,
        f: &mut impl FnMut(Expr) -> Result<Expr, AvengerChartError>,
    ) -> Result<Self, AvengerChartError> {
        Ok(match self {
            Self::Rect { x0, y0, x1, y1 } => Self::Rect {
                x0: map_expr_node(x0, f, "scene rectangle query x0")?,
                y0: map_expr_node(y0, f, "scene rectangle query y0")?,
                x1: map_expr_node(x1, f, "scene rectangle query x1")?,
                y1: map_expr_node(y1, f, "scene rectangle query y1")?,
            },
            Self::Circle { cx, cy, radius } => Self::Circle {
                cx: map_expr_node(cx, f, "scene circle query cx")?,
                cy: map_expr_node(cy, f, "scene circle query cy")?,
                radius: map_expr_node(radius, f, "scene circle query radius")?,
            },
            Self::Polygon { points } => Self::Polygon {
                points: map_expr_node(points, f, "scene polygon query points")?,
            },
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SceneGeometryCoordinateSpace {
    #[default]
    Scene,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SceneGeometryHitPolicy {
    EnvelopeIntersects,
    GeometryIntersects,
    GeometryContained,
    #[default]
    AnchorInside,
    CentroidInside,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneGeometryTarget {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    mark_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    subplot_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resolved_source_group: Option<Vec<usize>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    resolved_mark_ids: Vec<crate::MarkId>,
}

impl SceneGeometryTarget {
    pub fn mark_ids(&self) -> &[String] {
        &self.mark_ids
    }

    pub fn subplot_ids(&self) -> &[String] {
        &self.subplot_ids
    }

    #[doc(hidden)]
    pub fn resolved_source_group(&self) -> Option<&[usize]> {
        self.resolved_source_group.as_deref()
    }

    #[doc(hidden)]
    pub fn resolved_mark_ids(&self) -> &[crate::MarkId] {
        &self.resolved_mark_ids
    }

    #[doc(hidden)]
    pub fn with_resolved_source_group(mut self, group: Vec<usize>) -> Self {
        self.resolved_source_group = Some(group);
        self
    }

    #[doc(hidden)]
    pub fn with_resolved_mark_ids(mut self, ids: Vec<crate::MarkId>) -> Self {
        self.resolved_mark_ids = ids;
        self
    }

    pub fn validate(&self) -> Result<(), AvengerChartError> {
        for id in &self.mark_ids {
            validate_mark_target_path("mark", id)?;
        }
        for id in &self.subplot_ids {
            validate_structural_id("subplot target", id)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneGeometryQuery {
    pub geometry: SceneGeometryQueryGeometry,
    #[serde(default)]
    pub coordinate_space: SceneGeometryCoordinateSpace,
    #[serde(default)]
    pub hit_policy: SceneGeometryHitPolicy,
    #[serde(default)]
    pub target: SceneGeometryTarget,
    #[serde(default)]
    pub datum_fields: Vec<SceneQueryDatumField>,
    #[serde(default)]
    pub unique_by: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_hits: Option<usize>,
}

impl SceneGeometryQuery {
    pub fn rect(
        x0: impl IntoExpr,
        y0: impl IntoExpr,
        x1: impl IntoExpr,
        y1: impl IntoExpr,
    ) -> Self {
        Self {
            geometry: SceneGeometryQueryGeometry::Rect {
                x0: expr_node(x0.into_expr(), "scene rectangle query x0"),
                y0: expr_node(y0.into_expr(), "scene rectangle query y0"),
                x1: expr_node(x1.into_expr(), "scene rectangle query x1"),
                y1: expr_node(y1.into_expr(), "scene rectangle query y1"),
            },
            coordinate_space: SceneGeometryCoordinateSpace::Scene,
            hit_policy: SceneGeometryHitPolicy::AnchorInside,
            target: SceneGeometryTarget::default(),
            datum_fields: Vec::new(),
            unique_by: Vec::new(),
            max_hits: None,
        }
    }

    pub fn circle(cx: impl IntoExpr, cy: impl IntoExpr, radius: impl IntoExpr) -> Self {
        Self {
            geometry: SceneGeometryQueryGeometry::Circle {
                cx: expr_node(cx.into_expr(), "scene circle query cx"),
                cy: expr_node(cy.into_expr(), "scene circle query cy"),
                radius: expr_node(radius.into_expr(), "scene circle query radius"),
            },
            coordinate_space: SceneGeometryCoordinateSpace::Scene,
            hit_policy: SceneGeometryHitPolicy::AnchorInside,
            target: SceneGeometryTarget::default(),
            datum_fields: Vec::new(),
            unique_by: Vec::new(),
            max_hits: None,
        }
    }

    pub fn polygon(points: impl IntoExpr) -> Self {
        Self {
            geometry: SceneGeometryQueryGeometry::Polygon {
                points: expr_node(points.into_expr(), "scene polygon query points"),
            },
            coordinate_space: SceneGeometryCoordinateSpace::Scene,
            hit_policy: SceneGeometryHitPolicy::AnchorInside,
            target: SceneGeometryTarget::default(),
            datum_fields: Vec::new(),
            unique_by: Vec::new(),
            max_hits: None,
        }
    }

    pub fn hit_policy(mut self, hit_policy: SceneGeometryHitPolicy) -> Self {
        self.hit_policy = hit_policy;
        self
    }

    pub fn mark(mut self, id: impl Into<String>) -> Self {
        self.target.mark_ids = vec![id.into()];
        self
    }

    pub fn marks<I, S>(mut self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.target.mark_ids = ids.into_iter().map(Into::into).collect();
        self
    }

    pub fn within_subplot(mut self, id: impl Into<String>) -> Self {
        self.target.subplot_ids = vec![id.into()];
        self
    }

    pub fn within_subplots<I, S>(mut self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.target.subplot_ids = ids.into_iter().map(Into::into).collect();
        self
    }

    pub fn datum_field(mut self, field: SceneQueryDatumField) -> Self {
        if self.unique_by.is_empty() {
            self.unique_by.push(field.id.clone());
        }
        self.datum_fields.push(field);
        self
    }

    pub fn datum_fields<I>(mut self, fields: I) -> Self
    where
        I: IntoIterator<Item = SceneQueryDatumField>,
    {
        for field in fields {
            self = self.datum_field(field);
        }
        self
    }

    pub fn unique_by<I, S>(mut self, fields: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.unique_by = fields.into_iter().map(Into::into).collect();
        self
    }

    pub fn max_hits(mut self, max_hits: usize) -> Self {
        self.max_hits = Some(max_hits);
        self
    }

    pub(crate) fn map_exprs(
        self,
        f: &mut impl FnMut(Expr) -> Result<Expr, AvengerChartError>,
    ) -> Result<Self, AvengerChartError> {
        Ok(Self {
            geometry: self.geometry.map_exprs(f)?,
            coordinate_space: self.coordinate_space,
            hit_policy: self.hit_policy,
            target: self.target,
            datum_fields: self
                .datum_fields
                .into_iter()
                .map(|field| field.map_exprs(f))
                .collect::<Result<_, AvengerChartError>>()?,
            unique_by: self.unique_by,
            max_hits: self.max_hits,
        })
    }
}

#[serde_as]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum SceneQueryClauseId {
    #[default]
    Tuple,
    Field(String),
    Expr(#[serde_as(as = "FromInto<SerializableExpr>")] LogicalExprNode),
}

impl SceneQueryClauseId {
    pub fn field(field: impl Into<String>) -> Self {
        Self::Field(field.into())
    }

    pub fn expr(expr: impl IntoExpr) -> Self {
        Self::Expr(expr_node(expr.into_expr(), "scene query clause id"))
    }

    pub(crate) fn map_exprs(
        self,
        f: &mut impl FnMut(Expr) -> Result<Expr, AvengerChartError>,
    ) -> Result<Self, AvengerChartError> {
        Ok(match self {
            Self::Tuple => Self::Tuple,
            Self::Field(field) => Self::Field(field),
            Self::Expr(expr) => Self::Expr(map_expr_node(expr, f, "scene query clause id")?),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelectionSceneQuery {
    pub query: SceneGeometryQuery,
    #[serde(default = "default_scene_query_sharing")]
    pub sharing: CoordinationScope,
    #[serde(default)]
    pub clause_id: SceneQueryClauseId,
}

impl SelectionSceneQuery {
    pub fn new(query: SceneGeometryQuery) -> Self {
        Self {
            query,
            sharing: CoordinationScope::Free,
            clause_id: SceneQueryClauseId::Tuple,
        }
    }

    pub fn sharing(mut self, sharing: CoordinationScope) -> Self {
        self.sharing = sharing;
        self
    }

    pub fn clause_id(mut self, clause_id: SceneQueryClauseId) -> Self {
        self.clause_id = clause_id;
        self
    }

    pub(crate) fn map_exprs(
        self,
        f: &mut impl FnMut(Expr) -> Result<Expr, AvengerChartError>,
    ) -> Result<Self, AvengerChartError> {
        Ok(Self {
            query: self.query.map_exprs(f)?,
            sharing: self.sharing,
            clause_id: self.clause_id.map_exprs(f)?,
        })
    }
}

impl From<SceneGeometryQuery> for SelectionSceneQuery {
    fn from(query: SceneGeometryQuery) -> Self {
        Self::new(query)
    }
}

fn default_scene_query_sharing() -> CoordinationScope {
    CoordinationScope::Free
}

fn expr_node(expr: Expr, label: &str) -> LogicalExprNode {
    LogicalExprNode::from_default_expr(expr).expect(label)
}

fn map_expr_node(
    node: LogicalExprNode,
    f: &mut impl FnMut(Expr) -> Result<Expr, AvengerChartError>,
    label: &str,
) -> Result<LogicalExprNode, AvengerChartError> {
    let expr = node.to_expr(&SessionContext::new())?;
    LogicalExprNode::from_default_expr(f(expr)?).map_err(|err| {
        AvengerChartError::InternalError(format!("Failed to serialize mapped {label}: {err}"))
    })
}
