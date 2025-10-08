//! Title and subtitle configuration for plots

use crate::coords::CoordinateSystem;
use crate::maybe::{Maybe, MaybeOptionalExpr};
use crate::plot::Plot;
use crate::serialization::{LogicalExprNodeExt, SerializableExpr};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, FromInto};

/// Controls the width that the title/subtitle spans
#[derive(Clone, Debug, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum TitleSpan {
    /// Title/subtitle spans entire canvas width
    #[default]
    Canvas,
    /// Title/subtitle only spans the plot area width
    PlotArea,
}

/// Controls the text alignment within the title/subtitle span
#[derive(Clone, Debug, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum TitleAlign {
    /// Text aligned to the left
    Left,
    /// Text aligned to the center
    #[default]
    Center,
    /// Text aligned to the right
    Right,
}

/// Minimal plot title configuration
#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlotTitle {
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
}

/// Minimal plot subtitle configuration
#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlotSubtitle {
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
}

/// Title and subtitle configuration methods for Plot
impl<C: CoordinateSystem> Plot<C> {
    /// Set a simple plot title. For advanced styling, a richer API can be added later.
    /// Accepts string literals, expressions, or column references.
    pub fn title(mut self, text: impl super::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = text.into_expr();
        self.title = Some(PlotTitle {
            text: LogicalExprNode::from_expr(expr).expect("Failed to serialize title expr"),
            font_size: Maybe::Unset,
            font_family: Maybe::Unset,
            span: Maybe::Unset,
            align: Maybe::Unset,
        });
        self
    }

    /// Configure the title with a closure for advanced options
    pub fn configure_title<F>(mut self, text: impl super::plot::IntoExpr, f: F) -> Self
    where
        F: FnOnce(PlotTitle) -> PlotTitle,
    {
        use crate::serialization::LogicalExprNodeExt;
        let expr = text.into_expr();
        let title = PlotTitle {
            text: LogicalExprNode::from_expr(expr).expect("Failed to serialize title expr"),
            font_size: Maybe::Unset,
            font_family: Maybe::Unset,
            span: Maybe::Unset,
            align: Maybe::Unset,
        };
        self.title = Some(f(title));
        self
    }

    /// Set a simple plot subtitle. For advanced styling, a richer API can be added later.
    /// Accepts string literals, expressions, or column references.
    pub fn subtitle(mut self, text: impl super::plot::IntoExpr) -> Self {
        use crate::serialization::LogicalExprNodeExt;
        let expr = text.into_expr();
        self.subtitle = Some(PlotSubtitle {
            text: LogicalExprNode::from_expr(expr).expect("Failed to serialize subtitle expr"),
            font_size: Maybe::Unset,
            font_family: Maybe::Unset,
            span: Maybe::Unset,
            align: Maybe::Unset,
        });
        self
    }

    /// Configure the subtitle with a closure for advanced options
    pub fn configure_subtitle<F>(mut self, text: impl super::plot::IntoExpr, f: F) -> Self
    where
        F: FnOnce(PlotSubtitle) -> PlotSubtitle,
    {
        use crate::serialization::LogicalExprNodeExt;
        let expr = text.into_expr();
        let subtitle = PlotSubtitle {
            text: LogicalExprNode::from_expr(expr).expect("Failed to serialize subtitle expr"),
            font_size: Maybe::Unset,
            font_family: Maybe::Unset,
            span: Maybe::Unset,
            align: Maybe::Unset,
        };
        self.subtitle = Some(f(subtitle));
        self
    }

    /// Access the configured title
    pub fn get_title(&self) -> Option<&PlotTitle> {
        self.title.as_ref()
    }

    /// Access the configured subtitle
    pub fn get_subtitle(&self) -> Option<&PlotSubtitle> {
        self.subtitle.as_ref()
    }
}

impl TitleSpan {
    /// Convert TitleSpan to a string literal
    pub fn to_str(&self) -> &'static str {
        match self {
            TitleSpan::Canvas => "canvas",
            TitleSpan::PlotArea => "plot_area",
        }
    }
}

impl TitleAlign {
    /// Convert TitleAlign to a string literal
    pub fn to_str(&self) -> &'static str {
        match self {
            TitleAlign::Left => "left",
            TitleAlign::Center => "center",
            TitleAlign::Right => "right",
        }
    }
}

impl PlotTitle {
    /// Set the font size
    pub fn font_size(mut self, size: impl super::plot::IntoExpr) -> Self {
        let expr = size.into_expr();
        self.font_size = Maybe::Set(Some(LogicalExprNode::from_expr(expr).expect("Failed to serialize font_size expr")));
        self
    }

    /// Set the font family
    pub fn font_family(mut self, family: impl super::plot::IntoExpr) -> Self {
        let expr = family.into_expr();
        self.font_family = Maybe::Set(Some(LogicalExprNode::from_expr(expr).expect("Failed to serialize font_family expr")));
        self
    }

    /// Set the span (Canvas or PlotArea)
    pub fn span(mut self, span: impl super::plot::IntoExpr) -> Self {
        let expr = span.into_expr();
        self.span = Maybe::Set(Some(LogicalExprNode::from_expr(expr).expect("Failed to serialize span expr")));
        self
    }

    /// Set the text alignment (Left, Center, Right)
    pub fn align(mut self, align: impl super::plot::IntoExpr) -> Self {
        let expr = align.into_expr();
        self.align = Maybe::Set(Some(LogicalExprNode::from_expr(expr).expect("Failed to serialize align expr")));
        self
    }
}

impl PlotSubtitle {
    /// Set the font size
    pub fn font_size(mut self, size: impl super::plot::IntoExpr) -> Self {
        let expr = size.into_expr();
        self.font_size = Maybe::Set(Some(LogicalExprNode::from_expr(expr).expect("Failed to serialize font_size expr")));
        self
    }

    /// Set the font family
    pub fn font_family(mut self, family: impl super::plot::IntoExpr) -> Self {
        let expr = family.into_expr();
        self.font_family = Maybe::Set(Some(LogicalExprNode::from_expr(expr).expect("Failed to serialize font_family expr")));
        self
    }

    /// Set the span (Canvas or PlotArea)
    pub fn span(mut self, span: impl super::plot::IntoExpr) -> Self {
        let expr = span.into_expr();
        self.span = Maybe::Set(Some(LogicalExprNode::from_expr(expr).expect("Failed to serialize span expr")));
        self
    }

    /// Set the text alignment (Left, Center, Right)
    pub fn align(mut self, align: impl super::plot::IntoExpr) -> Self {
        let expr = align.into_expr();
        self.align = Maybe::Set(Some(LogicalExprNode::from_expr(expr).expect("Failed to serialize align expr")));
        self
    }
}
