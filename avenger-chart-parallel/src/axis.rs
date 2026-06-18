use std::any::Any;

use avenger_chart_core::{
    AvengerChartError, Axis, IntoExpr, Maybe, MaybeOptionalExpr,
    serialization::DefaultLogicalExprNodeExt,
};
use datafusion::{logical_expr::Expr, prelude::SessionContext};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

/// Axis configuration for one parallel-coordinate dimension.
#[serde_as]
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ParallelAxis {
    #[serde_as(as = "MaybeOptionalExpr")]
    pub visible: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub title: Maybe<Option<LogicalExprNode>>,
    #[doc(hidden)]
    #[serde(default)]
    pub dimension_id: Option<String>,
    #[doc(hidden)]
    #[serde(default)]
    pub order_index: Option<usize>,
}

impl ParallelAxis {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn visible(mut self, visible: impl IntoExpr) -> Self {
        self.visible = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(visible.into_expr())
                .expect("failed to serialize parallel axis visible expression"),
        ));
        self
    }

    pub fn title(mut self, title: impl IntoExpr) -> Self {
        self.title = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(title.into_expr())
                .expect("failed to serialize parallel axis title expression"),
        ));
        self
    }

    pub fn update(mut self, other: Self) -> Self {
        if other.visible.is_set() {
            self.visible = other.visible;
        }
        if other.title.is_set() {
            self.title = other.title;
        }
        if other.dimension_id.is_some() {
            self.dimension_id = other.dimension_id;
        }
        if other.order_index.is_some() {
            self.order_index = other.order_index;
        }
        self
    }

    #[doc(hidden)]
    pub fn with_dimension_metadata(
        mut self,
        dimension_id: impl Into<String>,
        order_index: usize,
    ) -> Self {
        self.dimension_id = Some(dimension_id.into());
        self.order_index = Some(order_index);
        self
    }
}

#[typetag::serde]
impl Axis for ParallelAxis {
    fn update(&mut self, other: &dyn Axis) {
        if let Some(other) = other.as_any().downcast_ref::<ParallelAxis>() {
            *self = self.clone().update(other.clone());
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn box_clone(&self) -> Box<dyn Axis> {
        Box::new(self.clone())
    }

    fn set_default_title_expr(&mut self, title: Expr) -> Result<bool, AvengerChartError> {
        if self.title.is_set() {
            return Ok(false);
        }
        self.title = Maybe::Set(Some(LogicalExprNode::from_default_expr(title)?));
        Ok(true)
    }

    fn all_exprs(&self, ctx: &SessionContext) -> Vec<Expr> {
        [&self.visible, &self.title]
            .into_iter()
            .filter_map(|maybe| maybe.as_option().and_then(|expr| expr.as_ref()))
            .filter_map(|expr| expr.to_default_expr(ctx).ok())
            .collect()
    }

    fn map_exprs(
        &self,
        f: &mut dyn FnMut(Expr) -> Result<Expr, AvengerChartError>,
    ) -> Result<Box<dyn Axis>, AvengerChartError> {
        let map_maybe = |maybe: &Maybe<Option<LogicalExprNode>>,
                         f: &mut dyn FnMut(Expr) -> Result<Expr, AvengerChartError>|
         -> Result<Maybe<Option<LogicalExprNode>>, AvengerChartError> {
            let Some(Some(node)) = maybe.as_option() else {
                return Ok(maybe.clone());
            };
            Ok(Maybe::Set(Some(LogicalExprNode::from_default_expr(f(
                node.to_default_expr(&SessionContext::new())?,
            )?)?)))
        };

        Ok(Box::new(ParallelAxis {
            visible: map_maybe(&self.visible, f)?,
            title: map_maybe(&self.title, f)?,
            dimension_id: self.dimension_id.clone(),
            order_index: self.order_index,
        }))
    }
}
