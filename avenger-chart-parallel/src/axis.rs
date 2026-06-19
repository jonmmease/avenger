use std::any::Any;

use avenger_chart_core::{
    AvengerChartError, Axis, IntoExpr, Maybe, MaybeOptionalExpr,
    serialization::DefaultLogicalExprNodeExt,
};
use datafusion::{
    logical_expr::Expr,
    prelude::{SessionContext, lit, named_struct},
};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use crate::{ParallelDisplayState, ParallelOrderState};

/// Axis configuration for one parallel-coordinate dimension.
#[serde_as]
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ParallelAxis {
    #[serde_as(as = "MaybeOptionalExpr")]
    pub visible: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub title: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub grid: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub tick_count: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub tick_spacing: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub label_angle: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub format_number: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub title_font_family: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub title_color: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub label_font_family: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub show_title: Maybe<Option<LogicalExprNode>>,
    #[doc(hidden)]
    #[serde(default)]
    pub dimension_id: Option<String>,
    #[doc(hidden)]
    #[serde(default)]
    pub order_index: Option<usize>,
    #[doc(hidden)]
    #[serde(default)]
    pub order_state: Option<ParallelOrderState>,
    #[doc(hidden)]
    #[serde(default)]
    pub display_state: Option<ParallelDisplayState>,
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

    pub fn grid(mut self, grid: impl IntoExpr) -> Self {
        self.grid = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(grid.into_expr())
                .expect("failed to serialize parallel axis grid expression"),
        ));
        self
    }

    pub fn tick_count(mut self, count: impl IntoExpr) -> Self {
        self.tick_count = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(count.into_expr())
                .expect("failed to serialize parallel axis tick_count expression"),
        ));
        self
    }

    /// Generate numeric axis ticks from a struct expression with `start` and `step` fields.
    pub fn tick_spacing(mut self, spacing: impl IntoExpr) -> Self {
        self.tick_spacing = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(spacing.into_expr())
                .expect("failed to serialize parallel axis tick_spacing expression"),
        ));
        self
    }

    /// Generate numeric axis ticks from `start + n * step`, clipped to the scale domain.
    pub fn ticks_start_step(self, start: impl IntoExpr, step: impl IntoExpr) -> Self {
        self.tick_spacing(named_struct(vec![
            lit("start"),
            start.into_expr(),
            lit("step"),
            step.into_expr(),
        ]))
    }

    pub fn label_angle(mut self, angle: impl IntoExpr) -> Self {
        self.label_angle = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(angle.into_expr())
                .expect("failed to serialize parallel axis label_angle expression"),
        ));
        self
    }

    pub fn format(mut self, format: impl IntoExpr) -> Self {
        self.format_number = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(format.into_expr())
                .expect("failed to serialize parallel axis format expression"),
        ));
        self
    }

    pub fn title_font_family(mut self, font: impl IntoExpr) -> Self {
        self.title_font_family = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(font.into_expr())
                .expect("failed to serialize parallel axis title_font_family expression"),
        ));
        self
    }

    pub fn title_color(mut self, color: impl IntoExpr) -> Self {
        self.title_color = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(color.into_expr())
                .expect("failed to serialize parallel axis title_color expression"),
        ));
        self
    }

    pub fn label_font_family(mut self, font: impl IntoExpr) -> Self {
        self.label_font_family = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(font.into_expr())
                .expect("failed to serialize parallel axis label_font_family expression"),
        ));
        self
    }

    /// Show or hide the dimension header title only.
    pub fn show_title(mut self, show: impl IntoExpr) -> Self {
        self.show_title = Maybe::Set(Some(
            LogicalExprNode::from_default_expr(show.into_expr())
                .expect("failed to serialize parallel axis show_title expression"),
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
        if other.grid.is_set() {
            self.grid = other.grid;
        }
        if other.tick_count.is_set() {
            self.tick_count = other.tick_count;
        }
        if other.tick_spacing.is_set() {
            self.tick_spacing = other.tick_spacing;
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
        if other.title_color.is_set() {
            self.title_color = other.title_color;
        }
        if other.label_font_family.is_set() {
            self.label_font_family = other.label_font_family;
        }
        if other.show_title.is_set() {
            self.show_title = other.show_title;
        }
        if other.dimension_id.is_some() {
            self.dimension_id = other.dimension_id;
        }
        if other.order_index.is_some() {
            self.order_index = other.order_index;
        }
        if other.order_state.is_some() {
            self.order_state = other.order_state;
        }
        if other.display_state.is_some() {
            self.display_state = other.display_state;
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

    #[doc(hidden)]
    pub fn with_frame_state(
        mut self,
        order_state: Option<ParallelOrderState>,
        display_state: Option<ParallelDisplayState>,
    ) -> Self {
        self.order_state = order_state;
        self.display_state = display_state;
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
        [
            &self.visible,
            &self.title,
            &self.grid,
            &self.tick_count,
            &self.tick_spacing,
            &self.label_angle,
            &self.format_number,
            &self.title_font_family,
            &self.title_color,
            &self.label_font_family,
            &self.show_title,
        ]
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
            grid: map_maybe(&self.grid, f)?,
            tick_count: map_maybe(&self.tick_count, f)?,
            tick_spacing: map_maybe(&self.tick_spacing, f)?,
            label_angle: map_maybe(&self.label_angle, f)?,
            format_number: map_maybe(&self.format_number, f)?,
            title_font_family: map_maybe(&self.title_font_family, f)?,
            title_color: map_maybe(&self.title_color, f)?,
            label_font_family: map_maybe(&self.label_font_family, f)?,
            show_title: map_maybe(&self.show_title, f)?,
            dimension_id: self.dimension_id.clone(),
            order_index: self.order_index,
            order_state: self.order_state.clone(),
            display_state: self.display_state.clone(),
        }))
    }
}
