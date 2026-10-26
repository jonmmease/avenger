//! Drawing and text measurement helpers for built-in widgets.

use avenger_color::ColorOrGradient;
use avenger_common::types::{TextAlign, TextBaseline, TextSyntaxMode};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::marks::text::SceneTextMark;
use avenger_typst_label::{LabelError, TextBounds};

use crate::style::WidgetStyle;
use crate::widget::{Look, WidgetContext};

/// A box as `[x, y, width, height]`.
pub(crate) type Bounds = [f32; 4];

/// Label source text and its syntax mode.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Text {
    pub(crate) source: String,
    pub(crate) syntax: TextSyntaxMode,
}

impl Text {
    pub(crate) fn plain(text: impl Into<String>) -> Self {
        Self {
            source: text.into(),
            syntax: TextSyntaxMode::Plain,
        }
    }

    pub(crate) fn markup(source: String) -> Self {
        Self {
            source,
            syntax: TextSyntaxMode::TypstMarkup,
        }
    }

    /// The text in the style's font, with its top left corner at `at`.
    pub(crate) fn mark(
        &self,
        name: &str,
        at: [f32; 2],
        color: [f32; 4],
        style: &WidgetStyle,
    ) -> SceneTextMark {
        SceneTextMark {
            name: name.to_string(),
            len: 1,
            text: self.source.clone().into(),
            text_syntax: self.syntax,
            x: at[0].into(),
            y: at[1].into(),
            align: TextAlign::Left.into(),
            baseline: TextBaseline::Top.into(),
            color: color.into(),
            font: style.font_family.clone().into(),
            font_size: style.font_size.into(),
            ..Default::default()
        }
    }

    /// The text's box in the style's font.
    pub(crate) fn bounds(&self, cx: &WidgetContext<'_>) -> Result<TextBounds, LabelError> {
        let mark = self.mark("", [0.0; 2], [0.0; 4], cx.style);
        let label = mark.labels().next().expect("a text mark with one label");
        cx.engine.bounds(&label.label)
    }

    /// The text's width and height, or zeros if typesetting fails. The widget reports the error
    /// when drawing.
    pub(crate) fn size(&self, cx: &WidgetContext<'_>) -> [f32; 2] {
        self.bounds(cx)
            .map_or([0.0; 2], |bounds| [bounds.width, bounds.height])
    }
}

/// A box filled with `fill` and outlined with `border`.
pub(crate) fn rect(
    name: &str,
    [x, y, width, height]: Bounds,
    fill: [f32; 4],
    border: [f32; 4],
    border_width: f32,
    corner_radius: f32,
) -> SceneRectMark {
    SceneRectMark {
        name: name.to_string(),
        x: x.into(),
        y: y.into(),
        width: Some(width.into()),
        height: Some(height.into()),
        fill: ColorOrGradient::Color(fill).into(),
        stroke: ColorOrGradient::Color(border).into(),
        stroke_width: border_width.into(),
        corner_radius: corner_radius.into(),
        ..Default::default()
    }
}

/// A focus ring around `bounds`, whose corners have `corner_radius`.
pub(crate) fn focus_ring(bounds: Bounds, corner_radius: f32, style: &WidgetStyle) -> SceneMark {
    let [x, y, width, height] = bounds;
    let out = style.focus_offset + style.focus_width / 2.0;
    let ring = rect(
        "focus_ring",
        [x - out, y - out, width + 2.0 * out, height + 2.0 * out],
        [0.0; 4],
        style.focus_color,
        style.focus_width,
        corner_radius + out,
    );
    SceneRectMark {
        interactive: Some(false),
        ..ring
    }
    .into()
}

/// An invisible hit target over `bounds`. Draw it after a control label so a press activates
/// the control instead of selecting text through an overlying text selection scene.
pub(crate) fn cover([x, y, width, height]: Bounds) -> SceneMark {
    SceneRectMark {
        name: "cover".to_string(),
        interactive: Some(true),
        x: x.into(),
        y: y.into(),
        width: Some(width.into()),
        height: Some(height.into()),
        ..Default::default()
    }
    .into()
}

/// The color of a widget's label, which changes only when the widget is disabled.
pub(crate) fn label_color(style: &WidgetStyle, look: Look) -> [f32; 4] {
    let enabled = Look {
        enabled: look.enabled,
        ..Look::default()
    };
    style.neutral.paint(enabled).foreground
}

/// Whether `bounds` contains `point`.
pub(crate) fn holds([x, y, width, height]: Bounds, [px, py]: [f32; 2]) -> bool {
    (x..=x + width).contains(&px) && (y..=y + height).contains(&py)
}
