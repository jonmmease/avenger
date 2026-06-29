//! Title and subtitle configuration for plots.

use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use avenger_chart_core::{
    IntoExpr,
    maybe::{Maybe, MaybeOptionalExpr},
};
use avenger_text::types::TextSyntaxMode;

use crate::{
    coords::CoordinateSystem,
    plot::Plot,
    serialization::{LogicalExprNodeExt, SerializableExpr},
};

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
    #[serde(default)]
    pub syntax_mode: TextSyntaxMode,
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
    #[serde(default)]
    pub syntax_mode: TextSyntaxMode,
}

/// Title and subtitle configuration methods for Plot
impl<C: CoordinateSystem> Plot<C> {
    /// Set a simple plot title. For advanced styling, a richer API can be added later.
    /// Accepts string literals, expressions, or column references.
    pub fn title(mut self, text: impl IntoExpr) -> Self {
        let expr = text.into_expr();
        self.title = Some(PlotTitle {
            text: LogicalExprNode::from_expr(expr).expect("Failed to serialize title expr"),
            font_size: Maybe::Unset,
            font_family: Maybe::Unset,
            span: Maybe::Unset,
            align: Maybe::Unset,
            syntax_mode: TextSyntaxMode::Plain,
        });
        self
    }

    /// Configure the title with a closure for advanced options
    pub fn configure_title<F>(mut self, text: impl IntoExpr, f: F) -> Self
    where
        F: FnOnce(PlotTitle) -> PlotTitle,
    {
        let expr = text.into_expr();
        let title = PlotTitle {
            text: LogicalExprNode::from_expr(expr).expect("Failed to serialize title expr"),
            font_size: Maybe::Unset,
            font_family: Maybe::Unset,
            span: Maybe::Unset,
            align: Maybe::Unset,
            syntax_mode: TextSyntaxMode::Plain,
        };
        self.title = Some(f(title));
        self
    }

    /// Set a simple plot subtitle. For advanced styling, a richer API can be added later.
    /// Accepts string literals, expressions, or column references.
    pub fn subtitle(mut self, text: impl IntoExpr) -> Self {
        let expr = text.into_expr();
        self.subtitle = Some(PlotSubtitle {
            text: LogicalExprNode::from_expr(expr).expect("Failed to serialize subtitle expr"),
            font_size: Maybe::Unset,
            font_family: Maybe::Unset,
            span: Maybe::Unset,
            align: Maybe::Unset,
            syntax_mode: TextSyntaxMode::Plain,
        });
        self
    }

    /// Configure the subtitle with a closure for advanced options
    pub fn configure_subtitle<F>(mut self, text: impl IntoExpr, f: F) -> Self
    where
        F: FnOnce(PlotSubtitle) -> PlotSubtitle,
    {
        let expr = text.into_expr();
        let subtitle = PlotSubtitle {
            text: LogicalExprNode::from_expr(expr).expect("Failed to serialize subtitle expr"),
            font_size: Maybe::Unset,
            font_family: Maybe::Unset,
            span: Maybe::Unset,
            align: Maybe::Unset,
            syntax_mode: TextSyntaxMode::Plain,
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

impl PlotTitle {
    /// Set the font size
    pub fn font_size(mut self, size: impl IntoExpr) -> Self {
        let expr = size.into_expr();
        self.font_size = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize font_size expr"),
        ));
        self
    }

    /// Set the font family
    pub fn font_family(mut self, family: impl IntoExpr) -> Self {
        let expr = family.into_expr();
        self.font_family = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize font_family expr"),
        ));
        self
    }

    /// Set the span (Canvas or PlotArea)
    pub fn span(mut self, span: impl IntoExpr) -> Self {
        let expr = span.into_expr();
        self.span = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize span expr"),
        ));
        self
    }

    /// Set the text alignment (Left, Center, Right)
    pub fn align(mut self, align: impl IntoExpr) -> Self {
        let expr = align.into_expr();
        self.align = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize align expr"),
        ));
        self
    }

    /// Interpret this title as Typst markup.
    pub fn typst(mut self) -> Self {
        self.syntax_mode = TextSyntaxMode::TypstMarkup;
        self
    }

    /// Interpret this title as literal plain text.
    pub fn plain_text(mut self) -> Self {
        self.syntax_mode = TextSyntaxMode::Plain;
        self
    }

    /// Set the title text syntax mode explicitly.
    pub fn syntax_mode(mut self, mode: TextSyntaxMode) -> Self {
        self.syntax_mode = mode;
        self
    }
}

impl PlotSubtitle {
    /// Set the font size
    pub fn font_size(mut self, size: impl IntoExpr) -> Self {
        let expr = size.into_expr();
        self.font_size = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize font_size expr"),
        ));
        self
    }

    /// Set the font family
    pub fn font_family(mut self, family: impl IntoExpr) -> Self {
        let expr = family.into_expr();
        self.font_family = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize font_family expr"),
        ));
        self
    }

    /// Set the span (Canvas or PlotArea)
    pub fn span(mut self, span: impl IntoExpr) -> Self {
        let expr = span.into_expr();
        self.span = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize span expr"),
        ));
        self
    }

    /// Set the text alignment (Left, Center, Right)
    pub fn align(mut self, align: impl IntoExpr) -> Self {
        let expr = align.into_expr();
        self.align = Maybe::Set(Some(
            LogicalExprNode::from_expr(expr).expect("Failed to serialize align expr"),
        ));
        self
    }

    /// Interpret this subtitle as Typst markup.
    pub fn typst(mut self) -> Self {
        self.syntax_mode = TextSyntaxMode::TypstMarkup;
        self
    }

    /// Interpret this subtitle as literal plain text.
    pub fn plain_text(mut self) -> Self {
        self.syntax_mode = TextSyntaxMode::Plain;
        self
    }

    /// Set the subtitle text syntax mode explicitly.
    pub fn syntax_mode(mut self, mode: TextSyntaxMode) -> Self {
        self.syntax_mode = mode;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartesian::Cartesian;
    use avenger_scenegraph::marks::{mark::SceneMark, text::SceneTextMark};
    use datafusion::common::ScalarValue;
    use datafusion::prelude::SessionContext;
    use std::sync::Arc;

    #[test]
    fn title_and_subtitle_default_to_plain_text() {
        let plot = Plot::<Cartesian>::new()
            .title("cost $5")
            .subtitle("#underline[raw]");

        assert_eq!(plot.get_title().unwrap().syntax_mode, TextSyntaxMode::Plain);
        assert_eq!(
            plot.get_subtitle().unwrap().syntax_mode,
            TextSyntaxMode::Plain
        );
    }

    #[test]
    fn title_and_subtitle_opt_into_typst_markup() {
        let plot = Plot::<Cartesian>::new()
            .configure_title("Price $R^2$", |t| t.typst())
            .configure_subtitle("cost \\$5", |s| s.typst());

        assert_eq!(
            plot.get_title().unwrap().syntax_mode,
            TextSyntaxMode::TypstMarkup
        );
        assert_eq!(
            plot.get_subtitle().unwrap().syntax_mode,
            TextSyntaxMode::TypstMarkup
        );
    }

    #[test]
    fn plain_text_overrides_typst_markup() {
        let plot = Plot::<Cartesian>::new().configure_title("cost $5", |t| t.typst().plain_text());

        assert_eq!(plot.get_title().unwrap().syntax_mode, TextSyntaxMode::Plain);
    }

    #[tokio::test]
    async fn plain_title_with_literal_dollar_evaluates() {
        let ctx = SessionContext::new();
        let compiled = Plot::<Cartesian>::new()
            .title("cost $5")
            .compile(&ctx)
            .await
            .unwrap();

        compiled.evaluate(&ctx, None).await.unwrap();
    }

    #[tokio::test]
    async fn typst_title_with_unmatched_dollar_errors() {
        let ctx = SessionContext::new();
        let compiled = Plot::<Cartesian>::new()
            .configure_title("cost $5", |t| t.typst())
            .compile(&ctx)
            .await
            .unwrap();

        assert!(compiled.evaluate(&ctx, None).await.is_err());
    }

    #[tokio::test]
    async fn typst_title_with_escaped_dollar_evaluates() {
        let ctx = SessionContext::new();
        let compiled = Plot::<Cartesian>::new()
            .configure_title("cost \\$5", |t| t.typst())
            .compile(&ctx)
            .await
            .unwrap();

        compiled.evaluate(&ctx, None).await.unwrap();
    }

    #[tokio::test]
    async fn typst_title_and_subtitle_scene_marks_keep_syntax_mode() {
        let ctx = SessionContext::new();
        let compiled = Plot::<Cartesian>::new()
            .configure_title("Price $R^2$", |t| t.typst())
            .configure_subtitle("cost \\$5", |s| s.typst())
            .compile(&ctx)
            .await
            .unwrap();

        let evaluated = compiled.evaluate(&ctx, None).await.unwrap();
        let mut modes = Vec::new();
        collect_text_syntax_modes(&evaluated.scene_graph.marks, &mut modes);

        assert_eq!(
            modes
                .iter()
                .filter(|mode| **mode == TextSyntaxMode::TypstMarkup)
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn typst_title_scene_mark_contains_only_referenced_params() {
        let ctx = SessionContext::new();
        let compiled = Plot::<Cartesian>::new()
            .configure_title("#series", |t| t.typst())
            .compile(&ctx)
            .await
            .unwrap();
        let mut params = indexmap::IndexMap::new();
        params.insert(
            "series".to_string(),
            ScalarValue::Utf8(Some("Revenue".to_string())),
        );
        params.insert("unused".to_string(), ScalarValue::Date32(Some(1)));

        let evaluated = compiled.evaluate(&ctx, Some(params)).await.unwrap();
        let mut text_marks = Vec::new();
        collect_text_marks(&evaluated.scene_graph.marks, &mut text_marks);
        let title = text_marks
            .iter()
            .find(|mark| mark.text_syntax == TextSyntaxMode::TypstMarkup)
            .expect("title text mark should exist");

        assert!(title.text_params.contains_key("series"));
        assert!(!title.text_params.contains_key("unused"));
    }

    #[tokio::test]
    async fn typst_title_missing_param_errors() {
        let ctx = SessionContext::new();
        let compiled = Plot::<Cartesian>::new()
            .configure_title("#series", |t| t.typst())
            .compile(&ctx)
            .await
            .unwrap();

        assert!(compiled.evaluate(&ctx, None).await.is_err());
    }

    fn collect_text_syntax_modes(marks: &[SceneMark], modes: &mut Vec<TextSyntaxMode>) {
        for mark in marks {
            match mark {
                SceneMark::Group(group) => collect_text_syntax_modes(&group.marks, modes),
                SceneMark::Text(text) => modes.push(text.text_syntax),
                _ => {}
            }
        }
    }

    fn collect_text_marks(marks: &[SceneMark], text_marks: &mut Vec<Arc<SceneTextMark>>) {
        for mark in marks {
            match mark {
                SceneMark::Group(group) => collect_text_marks(&group.marks, text_marks),
                SceneMark::Text(text) => text_marks.push(text.clone()),
                _ => {}
            }
        }
    }
}
