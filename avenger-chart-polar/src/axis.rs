use std::any::Any;

use avenger_chart_core::{
    Axis, IntoExpr, Maybe, MaybeOptionalExpr, serialization::DefaultLogicalExprNodeExt,
};
use datafusion::prelude::{Expr, lit};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

/// Type of polar axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PolarAxisType {
    #[default]
    Radial,
    Angular,
}

/// Direction for angular axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PolarDirection {
    #[default]
    Clockwise,
    CounterClockwise,
}

impl IntoExpr for PolarAxisType {
    fn into_expr(self) -> Expr {
        let s = match self {
            PolarAxisType::Radial => "radial",
            PolarAxisType::Angular => "angular",
        };
        lit(s)
    }
}

impl IntoExpr for PolarDirection {
    fn into_expr(self) -> Expr {
        let s = match self {
            PolarDirection::Clockwise => "clockwise",
            PolarDirection::CounterClockwise => "counterclockwise",
        };
        lit(s)
    }
}

/// Concrete struct for Polar axes.
///
/// Using a struct instead of a trait enables type inference in closure
/// parameters.
#[serde_as]
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PolarAxis {
    #[serde_as(as = "MaybeOptionalExpr")]
    pub visible: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub axis_type: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub title: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub grid: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub tick_count: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub format_number: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub grid_levels: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub start_angle: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub direction: Maybe<Option<LogicalExprNode>>,
}

impl PolarAxis {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn visible(mut self, visible: impl IntoExpr) -> Self {
        let expr = visible.into_expr();
        self.visible = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(expr).expect("Failed to serialize visible expr"),
        ));
        self
    }

    pub fn axis_type(mut self, axis_type: impl IntoExpr) -> Self {
        let expr = axis_type.into_expr();
        self.axis_type = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(expr).expect("Failed to serialize axis_type expr"),
        ));
        self
    }

    pub fn title(mut self, title: impl IntoExpr) -> Self {
        let expr = title.into_expr();
        self.title = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(expr).expect("Failed to serialize title expr"),
        ));
        self
    }

    pub fn grid(mut self, grid: impl IntoExpr) -> Self {
        let expr = grid.into_expr();
        self.grid = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(expr).expect("Failed to serialize grid expr"),
        ));
        self
    }

    pub fn tick_count(mut self, count: impl IntoExpr) -> Self {
        let expr = count.into_expr();
        self.tick_count = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(expr).expect("Failed to serialize tick_count expr"),
        ));
        self
    }

    pub fn format(mut self, format: impl IntoExpr) -> Self {
        let expr = format.into_expr();
        self.format_number = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(expr).expect("Failed to serialize format expr"),
        ));
        self
    }

    pub fn grid_levels(mut self, levels: impl IntoExpr) -> Self {
        let expr = levels.into_expr();
        self.grid_levels = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(expr).expect("Failed to serialize grid_levels expr"),
        ));
        self
    }

    pub fn start_angle(mut self, angle: impl IntoExpr) -> Self {
        let expr = angle.into_expr();
        self.start_angle = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(expr).expect("Failed to serialize start_angle expr"),
        ));
        self
    }

    pub fn direction(mut self, direction: impl IntoExpr) -> Self {
        let expr = direction.into_expr();
        self.direction = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(expr).expect("Failed to serialize direction expr"),
        ));
        self
    }

    /// Update this axis configuration with another, applying all set fields.
    pub fn update(mut self, other: PolarAxis) -> Self {
        if other.visible.is_set() {
            self.visible = other.visible;
        }
        if other.axis_type.is_set() {
            self.axis_type = other.axis_type;
        }
        if other.title.is_set() {
            self.title = other.title;
        }
        if other.grid.is_set() {
            self.grid = other.grid;
        }
        if other.tick_count.is_set() {
            self.tick_count = other.tick_count;
        }
        if other.format_number.is_set() {
            self.format_number = other.format_number;
        }
        if other.grid_levels.is_set() {
            self.grid_levels = other.grid_levels;
        }
        if other.start_angle.is_set() {
            self.start_angle = other.start_angle;
        }
        if other.direction.is_set() {
            self.direction = other.direction;
        }
        self
    }
}

#[typetag::serde]
impl Axis for PolarAxis {
    fn update(&mut self, other: &dyn Axis) {
        if let Some(o) = other.as_any().downcast_ref::<PolarAxis>() {
            *self = std::mem::take(self).update(o.clone());
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn box_clone(&self) -> Box<dyn Axis> {
        Box::new(self.clone())
    }
}
