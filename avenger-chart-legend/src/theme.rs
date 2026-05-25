//! Legend theme/default application helpers.

use avenger_chart_core::{Legend, LegendPosition, Maybe, Theme};
use datafusion::common::ScalarValue;
use indexmap::IndexMap;

fn color_array_to_hex(color: [f32; 4]) -> String {
    let r = (color[0] * 255.0) as u8;
    let g = (color[1] * 255.0) as u8;
    let b = (color[2] * 255.0) as u8;
    let a = (color[3] * 255.0) as u8;

    if a < 255 {
        format!("#{:02x}{:02x}{:02x}{:02x}", r, g, b, a)
    } else {
        format!("#{:02x}{:02x}{:02x}", r, g, b)
    }
}

fn apply_theme_to_legend<T, U, F, G>(
    legend: &mut Legend,
    field_check: F,
    theme_query: G,
    setter: impl FnOnce(Legend, T) -> Legend,
) where
    F: FnOnce(&Legend) -> &Maybe<Option<U>>,
    G: FnOnce() -> Option<T>,
{
    if matches!(field_check(legend), Maybe::Unset)
        && let Some(value) = theme_query()
    {
        *legend = setter(legend.clone(), value);
    }
}

/// Create a default legend and apply theme defaults that should be present on
/// automatically generated legends before user legend config merges over them.
#[doc(hidden)]
pub fn themed_default_legend(
    title: String,
    position: LegendPosition,
    legend_type: Option<&str>,
    theme: &Theme,
    params: &IndexMap<String, ScalarValue>,
) -> Legend {
    let mut legend = Legend::new().title(title).position(position);
    let legend_ctx = theme.legend_context_with_params(legend_type, params.clone());
    let bg_ctx = legend_ctx.child("background");
    let base_font_size = theme.get_base_font_size(&legend_ctx.params);

    if let Some(value) = theme.query(&bg_ctx, "padding")
        && let Some(padding) = value.as_font_size(&legend_ctx.params, base_font_size)
    {
        legend = legend.background_padding(padding);
    }

    if let Some(value) = theme.query(&bg_ctx, "corner-radius")
        && let Some(radius) = value.as_font_size(&legend_ctx.params, base_font_size)
    {
        legend = legend.background_corner_radius(radius);
    }

    if let Some(fill) = theme.fill_color(&bg_ctx) {
        legend = legend.background_fill(color_array_to_hex(fill));
    }
    if let Some(stroke) = theme.stroke_color(&bg_ctx) {
        legend = legend.background_stroke(color_array_to_hex(stroke));
    }
    if let Some(value) = theme.query(&bg_ctx, "stroke-width")
        && let Some(stroke_width) = value.as_font_size(&legend_ctx.params, base_font_size)
    {
        legend = legend.background_stroke_width(stroke_width);
    }

    if let Some(color) = theme.text_color(&legend_ctx.child("title")) {
        legend = legend.title_color(color_array_to_hex(color));
    }
    if let Some(color) = theme.text_color(&legend_ctx.child("label")) {
        legend = legend.label_color(color_array_to_hex(color));
    }
    if let Some(font_family) = theme.font_family(&legend_ctx.child("title")) {
        legend = legend.title_font_family(font_family);
    }
    if let Some(size) = theme.font_size(&legend_ctx.child("title")) {
        legend = legend.title_font_size(size);
    }
    if let Some(weight) = theme.font_weight(&legend_ctx.child("title")) {
        legend = legend.title_font_weight(weight);
    }
    if let Some(font_family) = theme.font_family(&legend_ctx.child("label")) {
        legend = legend.label_font_family(font_family);
    }
    if let Some(size) = theme.font_size(&legend_ctx.child("label")) {
        legend = legend.label_font_size(size);
    }
    if let Some(weight) = theme.font_weight(&legend_ctx.child("label")) {
        legend = legend.label_font_weight(weight);
    }
    if let Some(font_family) = theme.font_family(&legend_ctx.child("tick")) {
        legend = legend.tick_font_family(font_family);
    }
    if let Some(size) = theme.font_size(&legend_ctx.child("tick")) {
        legend = legend.tick_font_size(size);
    }
    if let Some(weight) = theme.font_weight(&legend_ctx.child("tick")) {
        legend = legend.tick_font_weight(weight);
    }
    if let Some(color) = theme.text_color(&legend_ctx.child("tick")) {
        legend = legend.tick_color(color_array_to_hex(color));
    }

    legend
}

/// Apply theme defaults to fields that are currently unset on a legend.
#[doc(hidden)]
pub fn apply_legend_theme_defaults(
    legend: &mut Legend,
    legend_type: Option<&str>,
    theme: &Theme,
    params: &IndexMap<String, ScalarValue>,
) {
    let legend_ctx = theme.legend_context_with_params(legend_type, params.clone());

    apply_theme_to_legend(
        legend,
        |l| &l.title_color,
        || {
            theme
                .text_color(&legend_ctx.child("title"))
                .map(color_array_to_hex)
        },
        |l, v| l.title_color(v),
    );
    apply_theme_to_legend(
        legend,
        |l| &l.title_font_family,
        || theme.font_family(&legend_ctx.child("title")),
        |l, v| l.title_font_family(v),
    );
    apply_theme_to_legend(
        legend,
        |l| &l.title_font_size,
        || theme.font_size(&legend_ctx.child("title")),
        |l, v| l.title_font_size(v),
    );
    apply_theme_to_legend(
        legend,
        |l| &l.title_font_weight,
        || theme.font_weight(&legend_ctx.child("title")),
        |l, v| l.title_font_weight(v),
    );

    apply_theme_to_legend(
        legend,
        |l| &l.label_color,
        || {
            theme
                .text_color(&legend_ctx.child("label"))
                .map(color_array_to_hex)
        },
        |l, v| l.label_color(v),
    );
    apply_theme_to_legend(
        legend,
        |l| &l.label_font_family,
        || theme.font_family(&legend_ctx.child("label")),
        |l, v| l.label_font_family(v),
    );
    apply_theme_to_legend(
        legend,
        |l| &l.label_font_size,
        || theme.font_size(&legend_ctx.child("label")),
        |l, v| l.label_font_size(v),
    );
    apply_theme_to_legend(
        legend,
        |l| &l.label_font_weight,
        || theme.font_weight(&legend_ctx.child("label")),
        |l, v| l.label_font_weight(v),
    );

    apply_theme_to_legend(
        legend,
        |l| &l.tick_color,
        || {
            theme
                .text_color(&legend_ctx.child("tick"))
                .map(color_array_to_hex)
        },
        |l, v| l.tick_color(v),
    );
    apply_theme_to_legend(
        legend,
        |l| &l.tick_font_family,
        || theme.font_family(&legend_ctx.child("tick")),
        |l, v| l.tick_font_family(v),
    );
    apply_theme_to_legend(
        legend,
        |l| &l.tick_font_size,
        || theme.font_size(&legend_ctx.child("tick")),
        |l, v| l.tick_font_size(v),
    );
    apply_theme_to_legend(
        legend,
        |l| &l.tick_font_weight,
        || theme.font_weight(&legend_ctx.child("tick")),
        |l, v| l.tick_font_weight(v),
    );

    apply_theme_to_legend(
        legend,
        |l| &l.background_fill,
        || {
            theme
                .fill_color(&legend_ctx.child("background"))
                .map(color_array_to_hex)
        },
        |l, v| l.background_fill(v),
    );
    apply_theme_to_legend(
        legend,
        |l| &l.background_stroke,
        || {
            theme
                .stroke_color(&legend_ctx.child("background"))
                .map(color_array_to_hex)
        },
        |l, v| l.background_stroke(v),
    );
    apply_theme_to_legend(
        legend,
        |l| &l.background_stroke_width,
        || {
            let bg_ctx = legend_ctx.child("background");
            let base_font_size = theme.get_base_font_size(&legend_ctx.params);
            theme
                .query(&bg_ctx, "stroke-width")
                .and_then(|value| value.as_font_size(&legend_ctx.params, base_font_size))
        },
        |l, v| l.background_stroke_width(v),
    );
    apply_theme_to_legend(
        legend,
        |l| &l.background_padding,
        || {
            let bg_ctx = legend_ctx.child("background");
            let base_font_size = theme.get_base_font_size(&legend_ctx.params);
            theme
                .query(&bg_ctx, "padding")
                .and_then(|value| value.as_font_size(&legend_ctx.params, base_font_size))
        },
        |l, v| l.background_padding(v),
    );
    apply_theme_to_legend(
        legend,
        |l| &l.background_corner_radius,
        || {
            let bg_ctx = legend_ctx.child("background");
            let base_font_size = theme.get_base_font_size(&legend_ctx.params);
            theme
                .query(&bg_ctx, "corner-radius")
                .and_then(|value| value.as_font_size(&legend_ctx.params, base_font_size))
        },
        |l, v| l.background_corner_radius(v),
    );

    if matches!(legend.position, Maybe::Unset) {
        let has_media_queries = theme.has_media_queries_for_property(&legend_ctx, "position");

        if !has_media_queries
            && let Some(theme_value) = theme.query(&legend_ctx, "position")
            && let Some(position_str) = theme_value.as_string()
        {
            let position = match position_str.to_lowercase().as_str() {
                "top" => Some(LegendPosition::Top),
                "bottom" => Some(LegendPosition::Bottom),
                "left" => Some(LegendPosition::Left),
                "right" => Some(LegendPosition::Right),
                _ => None,
            };
            if let Some(pos) = position {
                *legend = legend.clone().position(pos);
            }
        }
    }
}
