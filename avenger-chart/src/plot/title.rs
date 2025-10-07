//! Title and subtitle configuration for plots

use crate::coords::CoordinateSystem;
use crate::plot::Plot;
use crate::serialization::SerializableExpr;
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, FromInto};

/// Alignment options for title and subtitle
#[derive(Clone, Debug, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum TitleAlign {
    /// Title/subtitle spans entire width minus padding columns
    #[default]
    FullWidth,
    /// Title/subtitle only spans the plot area column
    PlotAreaOnly,
}

/// Minimal plot title configuration
#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlotTitle {
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub text: LogicalExprNode,
    pub font_size: Option<f32>,
    pub font_family: Option<String>,
    pub align: TitleAlign,
}

/// Minimal plot subtitle configuration
#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlotSubtitle {
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub text: LogicalExprNode,
    pub font_size: Option<f32>,
    pub font_family: Option<String>,
    pub align: TitleAlign,
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
            font_size: None,
            font_family: None,
            align: TitleAlign::default(),
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
            font_size: None,
            font_family: None,
            align: TitleAlign::default(),
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
            font_size: None,
            font_family: None,
            align: TitleAlign::default(),
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
            font_size: None,
            font_family: None,
            align: TitleAlign::default(),
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
