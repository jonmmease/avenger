//! Shared title and child-plot furnishing specifications.

use avenger_text::types::TextSyntaxMode;
use datafusion::prelude::{Expr, SessionContext, lit};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::{
    AvengerChartError, DefaultLogicalExprNodeExt, IntoExpr, RepeatContext, SerializableExpr,
    maybe::{Maybe, MaybeOptionalExpr},
    resolve_repeat_placeholders,
};

/// Serializable rich text specification shared by chart headings and cell captions.
#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TitleSpec {
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub text: LogicalExprNode,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub font_size: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub font_family: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub span: Maybe<Option<LogicalExprNode>>,
    #[serde_as(as = "MaybeOptionalExpr")]
    pub align: Maybe<Option<LogicalExprNode>>,
    #[serde(default)]
    pub syntax_mode: TextSyntaxMode,
}

impl TitleSpec {
    pub fn new(text: impl IntoExpr) -> Self {
        Self {
            text: expr_node(text, "title text"),
            font_size: Maybe::Unset,
            font_family: Maybe::Unset,
            span: Maybe::Unset,
            align: Maybe::Unset,
            syntax_mode: TextSyntaxMode::Plain,
        }
    }

    pub fn font_size(mut self, size: impl IntoExpr) -> Self {
        self.font_size = Maybe::Set(Some(expr_node(size, "title font size")));
        self
    }

    pub fn font_family(mut self, family: impl IntoExpr) -> Self {
        self.font_family = Maybe::Set(Some(expr_node(family, "title font family")));
        self
    }

    pub fn span(mut self, span: impl IntoExpr) -> Self {
        self.span = Maybe::Set(Some(expr_node(span, "title span")));
        self
    }

    pub fn align(mut self, align: impl IntoExpr) -> Self {
        self.align = Maybe::Set(Some(expr_node(align, "title alignment")));
        self
    }

    pub fn typst(mut self) -> Self {
        self.syntax_mode = TextSyntaxMode::TypstMarkup;
        self
    }

    pub fn plain_text(mut self) -> Self {
        self.syntax_mode = TextSyntaxMode::Plain;
        self
    }

    pub fn syntax_mode(mut self, mode: TextSyntaxMode) -> Self {
        self.syntax_mode = mode;
        self
    }

    pub fn resolve_repeat(&self, context: &RepeatContext) -> Result<Self, AvengerChartError> {
        Ok(Self {
            text: resolve_expr_node(&self.text, context)?,
            font_size: resolve_maybe_expr(&self.font_size, context)?,
            font_family: resolve_maybe_expr(&self.font_family, context)?,
            span: resolve_maybe_expr(&self.span, context)?,
            align: resolve_maybe_expr(&self.align, context)?,
            syntax_mode: self.syntax_mode,
        })
    }
}

/// Independently optional, expression-backed child plot dimensions.
#[serde_as]
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChildPlotSizeSpec {
    #[serde_as(as = "Option<FromInto<SerializableExpr>>")]
    pub width: Option<LogicalExprNode>,
    #[serde_as(as = "Option<FromInto<SerializableExpr>>")]
    pub height: Option<LogicalExprNode>,
}

impl ChildPlotSizeSpec {
    pub fn width(mut self, width: impl IntoExpr) -> Self {
        self.width = Some(expr_node(width, "child plot width"));
        self
    }

    pub fn height(mut self, height: impl IntoExpr) -> Self {
        self.height = Some(expr_node(height, "child plot height"));
        self
    }

    pub fn resolve_repeat(&self, context: &RepeatContext) -> Result<Self, AvengerChartError> {
        Ok(Self {
            width: self
                .width
                .as_ref()
                .map(|expr| resolve_expr_node(expr, context))
                .transpose()?,
            height: self
                .height
                .as_ref()
                .map(|expr| resolve_expr_node(expr, context))
                .transpose()?,
        })
    }
}

/// Position-aware furnishings supplied by a Subplot or RepeatCell wrapper.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChildPlotFurnishings {
    pub caption: Option<TitleSpec>,
    pub size: ChildPlotSizeSpec,
}

impl ChildPlotFurnishings {
    pub fn resolve_repeat(&self, context: &RepeatContext) -> Result<Self, AvengerChartError> {
        Ok(Self {
            caption: self
                .caption
                .as_ref()
                .map(|caption| caption.resolve_repeat(context))
                .transpose()?,
            size: self.size.resolve_repeat(context)?,
        })
    }
}

fn expr_node(value: impl IntoExpr, label: &str) -> LogicalExprNode {
    LogicalExprNode::from_default_expr(value.into_expr())
        .unwrap_or_else(|err| panic!("Failed to serialize {label} expression: {err}"))
}

fn resolve_expr_node(
    node: &LogicalExprNode,
    context: &RepeatContext,
) -> Result<LogicalExprNode, AvengerChartError> {
    let session_context = SessionContext::new();
    LogicalExprNode::from_default_expr(resolve_repeat_placeholders(
        node.to_default_expr(&session_context)?,
        context,
    )?)
}

fn resolve_maybe_expr(
    value: &Maybe<Option<LogicalExprNode>>,
    context: &RepeatContext,
) -> Result<Maybe<Option<LogicalExprNode>>, AvengerChartError> {
    match value {
        Maybe::Unset => Ok(Maybe::Unset),
        Maybe::Set(None) => Ok(Maybe::Set(None)),
        Maybe::Set(Some(node)) => Ok(Maybe::Set(Some(resolve_expr_node(node, context)?))),
    }
}

/// Controls the width that the title/subtitle spans.
#[derive(Clone, Debug, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum TitleSpan {
    /// Title/subtitle spans entire canvas width.
    #[default]
    Canvas,
    /// Title/subtitle only spans the plot area width.
    PlotArea,
}

/// Controls the text alignment within the title/subtitle span.
#[derive(Clone, Debug, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum TitleAlign {
    /// Text aligned to the left.
    #[default]
    Left,
    /// Text aligned to the center.
    Center,
    /// Text aligned to the right.
    Right,
}

impl TitleSpan {
    /// Convert TitleSpan to a string literal.
    pub fn to_str(&self) -> &'static str {
        match self {
            TitleSpan::Canvas => "canvas",
            TitleSpan::PlotArea => "plot_area",
        }
    }
}

impl TitleAlign {
    /// Convert TitleAlign to a string literal.
    pub fn to_str(&self) -> &'static str {
        match self {
            TitleAlign::Left => "left",
            TitleAlign::Center => "center",
            TitleAlign::Right => "right",
        }
    }
}

impl IntoExpr for TitleSpan {
    fn into_expr(self) -> Expr {
        lit(self.to_str())
    }
}

impl IntoExpr for TitleAlign {
    fn into_expr(self) -> Expr {
        lit(self.to_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RepeatVariable, repeat};

    fn repeat_context() -> RepeatContext {
        let session_context = SessionContext::new();
        RepeatContext::new()
            .with_row(
                RepeatVariable::new("row_a", lit(1_i64))
                    .title("Row title")
                    .resolve(&session_context)
                    .unwrap(),
                0,
                1,
            )
            .with_column(
                RepeatVariable::new("col_b", lit(2_i64))
                    .title("Column title")
                    .resolve(&session_context)
                    .unwrap(),
                0,
                1,
            )
    }

    #[test]
    fn furnishings_survive_bincode() {
        let value = ChildPlotFurnishings {
            caption: Some(
                TitleSpec::new("Caption")
                    .font_size(13.0)
                    .font_family("Inter")
                    .span(TitleSpan::PlotArea)
                    .align(TitleAlign::Center)
                    .typst(),
            ),
            size: ChildPlotSizeSpec::default().width(320.0).height(180.0),
        };
        let decoded: ChildPlotFurnishings =
            bincode::deserialize(&bincode::serialize(&value).unwrap()).unwrap();
        assert!(decoded.caption.unwrap().syntax_mode == TextSyntaxMode::TypstMarkup);
        assert!(decoded.size.width.is_some());
        assert!(decoded.size.height.is_some());
    }

    #[test]
    fn furnishings_resolve_repeat_in_caption_and_each_size_axis() {
        let value = ChildPlotFurnishings {
            caption: Some(
                TitleSpec::new(repeat::row_title())
                    .font_size(repeat::column().into_data_expr() + lit(10_i64))
                    .font_family(repeat::column_title())
                    .span(repeat::row_id())
                    .align(repeat::column_id()),
            ),
            size: ChildPlotSizeSpec::default()
                .width(repeat::row().into_data_expr() + lit(100_i64))
                .height(repeat::column().into_data_expr() + lit(200_i64)),
        };
        let resolved = value.resolve_repeat(&repeat_context()).unwrap();
        let session_context = SessionContext::new();
        let caption = resolved.caption.unwrap();
        let text = caption.text.to_default_expr(&session_context).unwrap();
        assert_eq!(text, lit("Row title"));
        let width = resolved
            .size
            .width
            .unwrap()
            .to_default_expr(&session_context)
            .unwrap();
        let height = resolved
            .size
            .height
            .unwrap()
            .to_default_expr(&session_context)
            .unwrap();
        assert_eq!(width, lit(1_i64) + lit(100_i64));
        assert_eq!(height, lit(2_i64) + lit(200_i64));
    }
}
