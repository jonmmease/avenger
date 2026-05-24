use std::any::Any;

pub use avenger_chart_core::AxisPosition;
use avenger_chart_core::{
    Axis, IntoExpr, Maybe, MaybeOptionalExpr, serialization::DefaultLogicalExprNodeExt,
};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

/// Concrete struct for Cartesian axes.
///
/// Using a struct instead of a trait enables type inference in closure
/// parameters.
#[serde_as]
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CartesianAxis {
    #[serde_as(as = "MaybeOptionalExpr")]
    pub visible: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub position: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub title: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub grid: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub tick_count: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub label_angle: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub format_number: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub title_font_family: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub label_font_family: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub show_title: Maybe<Option<LogicalExprNode>>,
}

impl CartesianAxis {
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

    pub fn position(mut self, position: impl IntoExpr) -> Self {
        let expr = position.into_expr();
        self.position = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(expr).expect("Failed to serialize position expr"),
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

    pub fn label_angle(mut self, angle: impl IntoExpr) -> Self {
        let expr = angle.into_expr();
        self.label_angle = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(expr).expect("Failed to serialize label_angle expr"),
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

    pub fn title_font_family(mut self, font: impl IntoExpr) -> Self {
        let expr = font.into_expr();
        self.title_font_family = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(expr)
                .expect("Failed to serialize title_font_family expr"),
        ));
        self
    }

    pub fn label_font_family(mut self, font: impl IntoExpr) -> Self {
        let expr = font.into_expr();
        self.label_font_family = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(expr)
                .expect("Failed to serialize label_font_family expr"),
        ));
        self
    }

    /// Show or hide the axis title only (labels unaffected).
    pub fn show_title(mut self, show: impl IntoExpr) -> Self {
        let expr = show.into_expr();
        self.show_title = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(expr).expect("Failed to serialize show_title expr"),
        ));
        self
    }

    /// Update this axis configuration with another, applying all set fields.
    pub fn update(mut self, other: CartesianAxis) -> Self {
        if other.visible.is_set() {
            self.visible = other.visible;
        }
        if other.position.is_set() {
            self.position = other.position;
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
        if other.label_angle.is_set() {
            self.label_angle = other.label_angle;
        }
        if other.format_number.is_set() {
            self.format_number = other.format_number;
        }
        if other.title_font_family.is_set() {
            self.title_font_family = other.title_font_family;
        }
        if other.label_font_family.is_set() {
            self.label_font_family = other.label_font_family;
        }
        self
    }
}

#[typetag::serde]
impl Axis for CartesianAxis {
    fn update(&mut self, other: &dyn Axis) {
        if let Some(o) = other.as_any().downcast_ref::<CartesianAxis>() {
            *self = self.clone().update(o.clone());
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn box_clone(&self) -> Box<dyn Axis> {
        Box::new(self.clone())
    }
}
