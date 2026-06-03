use datafusion::prelude::{Expr, col};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::{DefaultLogicalExprNodeExt, IntoExpr, SerializableExpr, Sharing};

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
    pub source_group: Option<Vec<usize>>,
    pub mark_paths: Option<Vec<Vec<usize>>>,
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

    pub fn source_group(mut self, group: Vec<usize>) -> Self {
        self.target.source_group = Some(group);
        self
    }

    pub fn mark_paths(mut self, paths: Vec<Vec<usize>>) -> Self {
        self.target.mark_paths = Some(paths);
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
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SceneQueryClauseId {
    Tuple,
    Field(String),
    Expr(#[serde_as(as = "FromInto<SerializableExpr>")] LogicalExprNode),
}

impl Default for SceneQueryClauseId {
    fn default() -> Self {
        Self::Tuple
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelectionSceneQuery {
    pub query: SceneGeometryQuery,
    #[serde(default = "default_scene_query_sharing")]
    pub sharing: Sharing,
    #[serde(default)]
    pub clause_id: SceneQueryClauseId,
}

impl SelectionSceneQuery {
    pub fn new(query: SceneGeometryQuery) -> Self {
        Self {
            query,
            sharing: Sharing::Free,
            clause_id: SceneQueryClauseId::Tuple,
        }
    }

    pub fn sharing(mut self, sharing: Sharing) -> Self {
        self.sharing = sharing;
        self
    }

    pub fn clause_id(mut self, clause_id: SceneQueryClauseId) -> Self {
        self.clause_id = clause_id;
        self
    }
}

impl From<SceneGeometryQuery> for SelectionSceneQuery {
    fn from(query: SceneGeometryQuery) -> Self {
        Self::new(query)
    }
}

fn default_scene_query_sharing() -> Sharing {
    Sharing::Free
}

fn expr_node(expr: Expr, label: &str) -> LogicalExprNode {
    LogicalExprNode::from_default_expr(expr).expect(label)
}
