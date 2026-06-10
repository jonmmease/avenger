//! Grid layout building logic

use std::collections::HashMap;

use avenger_text::{
    measurement::{TextBounds, TextMeasurementConfig, TextMeasurer, default_text_measurer},
    types::{FontStyle, FontWeight, FontWeightNameSpec},
};
use datafusion::{common::ScalarValue, prelude::SessionContext};
use datafusion_proto::protobuf::LogicalExprNode;
use indexmap::IndexMap;
use tracing::debug;

use avenger_chart_core::{
    LegendPosition, Size2D, evaluate_f32_expr, evaluate_string_expr, maybe::Maybe,
};
use avenger_layout::{Edges as LayoutEdges, Frame, FrameAxis, FrameAxisSizing, FrameSide};

use crate::{
    error::AvengerChartError,
    guide::OverflowSpaceRequirement,
    plot::compiled::TextMeasurementCacheKey,
    plot::{PlotSubtitle, PlotTitle},
    render::EvaluationContext,
    serialization::LogicalExprNodeExt,
    theme::{Theme, ThemeContext},
};

use super::sizing::EvaluatedLayoutSpec;

/// Minimum size in pixels for creating guide overflow regions.
/// Overflow regions smaller than this are ignored to avoid unnecessary grid complexity.
pub(crate) const MIN_GUIDE_OVERFLOW_SIZE: f32 = 2.0;

/// Minimum extent of the plot content when an axis is solved from a fixed
/// envelope (canvas) without a fixed plot size, and the floor for flexible
/// legend spans.
pub(crate) const MIN_COMPONENT_SIZE: f32 = 50.0;

/// Declared chrome of one chart frame, ready to solve, plus the existence
/// flags the rect projection needs. A zero-size layer and an absent layer
/// solve identically, but only existing layers project to component rects.
#[derive(Clone, Debug)]
pub(crate) struct FrameChrome {
    pub frame: Frame,
    /// A measured title band exists (`frame.vertical.leading.bands[0]`).
    pub has_title_band: bool,
    /// A measured subtitle band exists (the band after the title, if any).
    pub has_subtitle_band: bool,
    /// Guide overflow layers large enough to exist
    /// (> [`MIN_GUIDE_OVERFLOW_SIZE`]).
    pub guide_overflow: LayoutEdges<bool>,
}

/// Spacing multipliers for title and subtitle rows
const TITLE_ROW_HEIGHT_MULTIPLIER: f32 = 1.15;
const SUBTITLE_ROW_HEIGHT_MULTIPLIER: f32 = 1.1;

/// Default font sizes for title and subtitle when not specified
const DEFAULT_TITLE_FONT_SIZE: f32 = 16.0;
const DEFAULT_SUBTITLE_FONT_SIZE: f32 = 14.0;

/// Default font family when not specified in theme or expression
const DEFAULT_FONT_FAMILY: &str = "sans-serif";

/// Dynamic grid builder for chart layouts
///
/// `GridBuilder` constructs Avenger frame grids for charts by dynamically
/// positioning components based on their semantic roles and spatial
/// requirements. The builder is order-independent - components can be added in
/// any order and will be positioned deterministically based on their types and
/// the overflow measurements.
///
/// ## Component Positioning
///
/// Components are positioned in layers moving outward from the plot area:
/// - **Innermost**: Plot area (where data marks are rendered)
/// - **Next layer**: Guide overflows (directly adjacent to plot area, in overflow regions)
/// - **Outer layers**: Legends (farther from plot, after guide overflows)
/// - **Outermost**: Titles/subtitles (top only, before any guide overflows/legends)
///
/// ## Grid Structure
///
/// The builder creates a grid with the following structure:
/// ```text
///   ↓ margin column
/// ┌────┬───────────────────────────────────────────────┬────┐
/// │    │                                               │    │ ← margin row
/// ├────┼───────────────────────────────────────────────┼────┤
/// │    │              Title (optional)                 │    │
/// │    ├───────────────────────────────────────────────┤    │
/// │    │            Subtitle (optional)                │    │
/// │    ├────────────┬─────────────────┬────────┬───────┤    │
/// │    │            │ Overflow-top    │        │       │    │
/// │    │            │ (guide)         │        │       │    │
/// │    ├────────────┼─────────────────┼────────┼───────┤    │
/// │    │ Overflow-  │                 │Overflow│Right  │    │
/// │    │ left       │   Plot Area     │-right  │Legend │    │
/// │    │(guide)     │                 │(guide) │       │    │
/// │    ├────────────┼─────────────────┼────────┼───────┤    │
/// │    │            │ Overflow-bottom │        │       │    │
/// │    │            │ (guide)         │        │       │    │
/// ├────├────────────┴─────────────────┴────────┴───────┼────┤
/// │    │                                               │    │ ← margin row
/// └────┴───────────────────────────────────────────────┴────┘
///    ↑ margin column                                     ↑ margin column
/// ```
///
/// ## Dynamic Columns and Rows
///
/// The builder dynamically adds columns and rows based on:
/// - **Overflow requirements**: Space needed for guide elements beyond plot area
/// - **Legend containers**: Each legend position gets its own column/row
/// - **Titles**: Title and subtitle each get their own row
///
/// Only overflow regions larger than `MIN_GUIDE_OVERFLOW_SIZE` are created to avoid
/// unnecessary grid complexity for minimal overflows.
///
/// ## Component Positioning Rules
///
/// Components are positioned deterministically based on their types:
/// - **Margins**: Always outermost rows/columns
/// - **Title/Subtitle**: Top rows, before any guide overflows
/// - **Guide overflows**: Adjacent to plot area, created based on overflow measurements
/// - **Plot area**: Center, flexible sizing
/// - **Legend containers**: Outside guide overflows, preserving insertion order within each position
///
/// ## Legend Containers
///
/// Legends are grouped by position into containers:
/// - Multiple legends at the same position share a container
/// - The container manages internal spacing and alignment
/// - Container width is determined by the widest legend it contains
/// - Only channel names are stored, preserving insertion order per position
/// - Containers are created automatically during build phase
///
/// ## Build Process
///
/// The `build_with_overflow()` method:
/// 1. Creates margin columns/rows from `layout_spec.margins`
/// 2. Adds title/subtitle rows with measured heights
/// 3. Adds guide overflow columns/rows based on measured requirements
/// 4. Places the plot area in the center (flexible sizing with `fr(1.0)`)
/// 5. Adds legend container columns with measured widths
/// 6. Returns a `GridLayout` with track sizing functions and component positions
///    mapping components to their grid positions
pub(crate) struct GridBuilder {
    // Simple flags for component presence
    pub has_title: bool,
    pub has_subtitle: bool,

    // Track legend channels by position, preserving insertion order
    // This is the only place where insertion order matters (for stacking)
    pub legends_by_position: IndexMap<LegendPosition, Vec<String>>,
}

/// Helper function to measure title/subtitle text height
async fn measure_text_bounds(
    text_expr: &LogicalExprNode,
    font_size_field: &Maybe<Option<LogicalExprNode>>,
    font_family_field: &Maybe<Option<LogicalExprNode>>,
    theme_context: &ThemeContext,
    theme: &Theme,
    default_font_size: f32,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    eval_ctx: Option<&EvaluationContext>,
) -> Result<TextBounds, AvengerChartError> {
    // Evaluate font_size
    let font_size = match font_size_field {
        Maybe::Set(Some(node)) => {
            let expr = node.to_expr(ctx)?;
            evaluate_f32_expr(&expr, ctx, params).await?
        }
        _ => theme.font_size(theme_context).unwrap_or(default_font_size),
    };

    // Evaluate font_family
    let font_family = match font_family_field {
        Maybe::Set(Some(node)) => {
            let expr = node.to_expr(ctx)?;
            evaluate_string_expr(&expr, ctx, params).await?
        }
        _ => theme
            .font_family(theme_context)
            .unwrap_or_else(|| DEFAULT_FONT_FAMILY.to_string()),
    };

    // Evaluate the text expression to get the actual text
    let text_expr_df = text_expr.to_expr(ctx)?;
    let text_value = evaluate_string_expr(&text_expr_df, ctx, params).await?;

    // Measure text for layout (using Normal weight/style as approximation)
    let measurer = default_text_measurer();
    let config = TextMeasurementConfig {
        text: &text_value,
        font: &font_family,
        font_size,
        font_weight: &FontWeight::Name(FontWeightNameSpec::Normal),
        font_style: &FontStyle::Normal,
    };
    let bounds = if let Some(cache) = eval_ctx.and_then(EvaluationContext::text_measurement_cache) {
        let key = TextMeasurementCacheKey::new(
            config.text,
            config.font,
            config.font_size,
            config.font_weight,
            config.font_style,
        );
        let cached = {
            cache
                .lock()
                .expect("text measurement cache lock poisoned")
                .get(&key)
        };
        if let Some(bounds) = cached {
            if let Some(eval_ctx) = eval_ctx {
                eval_ctx.record_text_measurement_cache_hit();
            }
            bounds
        } else {
            if let Some(eval_ctx) = eval_ctx {
                eval_ctx.record_text_measurement_cache_miss();
            }
            let bounds = measurer.measure_text_bounds(&config);
            cache
                .lock()
                .expect("text measurement cache lock poisoned")
                .insert(key, bounds.clone());
            bounds
        }
    } else {
        measurer.measure_text_bounds(&config)
    };

    Ok(bounds)
}

impl GridBuilder {
    pub fn new() -> Self {
        GridBuilder {
            has_title: false,
            has_subtitle: false,
            legends_by_position: IndexMap::new(),
        }
    }

    pub fn add_title(&mut self) {
        self.has_title = true;
    }

    pub fn add_subtitle(&mut self) {
        self.has_subtitle = true;
    }

    pub fn add_legend(&mut self, channel: String, position: LegendPosition) {
        // Only store channel names, preserving insertion order per position
        self.legends_by_position
            .entry(position)
            .or_default()
            .push(channel);
    }

    pub fn measure_legend_container_width(
        &self,
        channels: &[String],
        legend_sizes: &HashMap<String, Size2D>,
    ) -> f32 {
        let mut max_width: f32 = 0.0;
        for channel in channels {
            if let Some(size) = legend_sizes.get(channel) {
                max_width = max_width.max(size.width);
            }
        }
        max_width
    }

    /// Measure the height needed for a legend container
    /// For horizontal legends (Top/Bottom), legends stack horizontally so use max height
    pub fn measure_legend_container_height(
        &self,
        channels: &[String],
        legend_sizes: &HashMap<String, Size2D>,
    ) -> f32 {
        let mut max_height: f32 = 0.0;
        for channel in channels {
            if let Some(size) = legend_sizes.get(channel) {
                max_height = max_height.max(size.height);
            }
        }
        max_height
    }

    /// Measure title and subtitle band heights (line height × row multiplier)
    /// for the components registered on this builder.
    #[allow(clippy::too_many_arguments)]
    async fn measure_title_band_heights(
        &self,
        title: Option<&PlotTitle>,
        subtitle: Option<&PlotSubtitle>,
        theme: &Theme,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
        eval_ctx: Option<&EvaluationContext>,
    ) -> Result<(Option<f32>, Option<f32>), AvengerChartError> {
        let title_height = if self.has_title
            && let Some(t) = title
        {
            let title_ctx = theme.title_context_with_params(params.clone());
            let text_node: LogicalExprNode = t.text.clone();
            let bounds = measure_text_bounds(
                &text_node,
                &t.font_size,
                &t.font_family,
                &title_ctx,
                theme,
                DEFAULT_TITLE_FONT_SIZE,
                ctx,
                params,
                eval_ctx,
            )
            .await?;
            Some(bounds.line_height * TITLE_ROW_HEIGHT_MULTIPLIER)
        } else {
            None
        };

        let subtitle_height = if self.has_subtitle
            && let Some(s) = subtitle
        {
            let subtitle_ctx = theme.subtitle_context_with_params(params.clone());
            let text_node: LogicalExprNode = s.text.clone();
            let bounds = measure_text_bounds(
                &text_node,
                &s.font_size,
                &s.font_family,
                &subtitle_ctx,
                theme,
                DEFAULT_SUBTITLE_FONT_SIZE,
                ctx,
                params,
                eval_ctx,
            )
            .await?;
            Some(bounds.line_height * SUBTITLE_ROW_HEIGHT_MULTIPLIER)
        } else {
            None
        };

        Ok((title_height, subtitle_height))
    }

    /// Build the declared frame chrome for the collected components and
    /// measured overflow requirements: per-side layers ordered outside-in as
    /// margin → bands (title, subtitle; top only) → outer (legend container)
    /// → inner (guide overflow), around the plot content.
    #[allow(clippy::too_many_arguments)]
    pub async fn build_frame_chrome(
        &self,
        overflow: &OverflowSpaceRequirement,
        sizing_horizontal: FrameAxisSizing,
        sizing_vertical: FrameAxisSizing,
        title: Option<&PlotTitle>,
        subtitle: Option<&PlotSubtitle>,
        theme: &Theme,
        layout_spec: &EvaluatedLayoutSpec,
        legend_sizes: &HashMap<String, Size2D>,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
        eval_ctx: Option<&EvaluationContext>,
    ) -> Result<FrameChrome, AvengerChartError> {
        let (title_height, subtitle_height) = self
            .measure_title_band_heights(title, subtitle, theme, ctx, params, eval_ctx)
            .await?;
        debug!(
            left = overflow.left,
            right = overflow.right,
            top = overflow.top,
            bottom = overflow.bottom,
            "Frame chrome build"
        );

        let legend_container_width = |position: LegendPosition| {
            self.legends_by_position
                .get(&position)
                .map(|channels| self.measure_legend_container_width(channels, legend_sizes))
                .unwrap_or(0.0)
        };
        let legend_container_height = |position: LegendPosition| {
            self.legends_by_position
                .get(&position)
                .map(|channels| self.measure_legend_container_height(channels, legend_sizes))
                .unwrap_or(0.0)
        };

        let guide_overflow = LayoutEdges::new(
            overflow.top > MIN_GUIDE_OVERFLOW_SIZE,
            overflow.right > MIN_GUIDE_OVERFLOW_SIZE,
            overflow.bottom > MIN_GUIDE_OVERFLOW_SIZE,
            overflow.left > MIN_GUIDE_OVERFLOW_SIZE,
        );
        // Existing overflow layers are pixel-aligned by ceiling, as the
        // legacy grid did when creating overflow tracks.
        let inner = |present: bool, value: f32| if present { value.ceil() } else { 0.0 };

        let mut top_bands = Vec::new();
        if let Some(height) = title_height {
            top_bands.push(height);
        }
        if let Some(height) = subtitle_height {
            top_bands.push(height);
        }

        let margins = &layout_spec.margins;
        let frame = Frame {
            horizontal: FrameAxis {
                sizing: sizing_horizontal,
                leading: FrameSide {
                    margin: margins.left,
                    bands: Vec::new(),
                    outer: legend_container_width(LegendPosition::Left),
                    inner: inner(guide_overflow.left, overflow.left),
                },
                trailing: FrameSide {
                    margin: margins.right,
                    bands: Vec::new(),
                    outer: legend_container_width(LegendPosition::Right),
                    inner: inner(guide_overflow.right, overflow.right),
                },
                content_min: MIN_COMPONENT_SIZE,
            },
            vertical: FrameAxis {
                sizing: sizing_vertical,
                leading: FrameSide {
                    margin: margins.top,
                    bands: top_bands,
                    outer: legend_container_height(LegendPosition::Top),
                    inner: inner(guide_overflow.top, overflow.top),
                },
                trailing: FrameSide {
                    margin: margins.bottom,
                    bands: Vec::new(),
                    outer: legend_container_height(LegendPosition::Bottom),
                    inner: inner(guide_overflow.bottom, overflow.bottom),
                },
                content_min: MIN_COMPONENT_SIZE,
            },
        };

        Ok(FrameChrome {
            frame,
            has_title_band: title_height.is_some(),
            has_subtitle_band: subtitle_height.is_some(),
            guide_overflow,
        })
    }
}

#[cfg(test)]
mod tests {
    use datafusion::prelude::SessionContext;
    use indexmap::IndexMap;

    use avenger_layout::FrameAxisSizing;

    use crate::{
        guide::OverflowSpaceRequirement,
        layout::{EvaluatedLayoutSpec, EvaluatedMargins, EvaluatedSizeMode},
        theme::Theme,
    };

    use super::GridBuilder;

    #[tokio::test]
    async fn build_frame_chrome_respects_minimum_guide_threshold() {
        let builder = GridBuilder::new();
        let ctx = SessionContext::new();
        let params = IndexMap::new();
        let theme = Theme::light();
        let layout_spec = EvaluatedLayoutSpec {
            canvas: EvaluatedSizeMode::Fixed {
                width: 400.0,
                height: 300.0,
            },
            plot_area: EvaluatedSizeMode::Auto,
            margins: EvaluatedMargins {
                top: 10.0,
                right: 10.0,
                bottom: 10.0,
                left: 10.0,
            },
        };
        let sizing_horizontal = FrameAxisSizing::EnvelopeFixed { extent: 400.0 };
        let sizing_vertical = FrameAxisSizing::EnvelopeFixed { extent: 300.0 };

        let below_threshold = builder
            .build_frame_chrome(
                &OverflowSpaceRequirement {
                    left: 2.0,
                    right: 1.9,
                    top: 2.0,
                    bottom: 1.9,
                },
                sizing_horizontal,
                sizing_vertical,
                None,
                None,
                &theme,
                &layout_spec,
                &Default::default(),
                &ctx,
                &params,
                None,
            )
            .await
            .expect("build below-threshold chrome");

        assert!(!below_threshold.guide_overflow.left);
        assert!(!below_threshold.guide_overflow.right);
        assert!(!below_threshold.guide_overflow.top);
        assert!(!below_threshold.guide_overflow.bottom);
        assert_eq!(below_threshold.frame.horizontal.leading.inner, 0.0);
        assert_eq!(below_threshold.frame.vertical.leading.inner, 0.0);

        let above_threshold = builder
            .build_frame_chrome(
                &OverflowSpaceRequirement {
                    left: 2.01,
                    right: 3.2,
                    top: 2.01,
                    bottom: 3.2,
                },
                sizing_horizontal,
                sizing_vertical,
                None,
                None,
                &theme,
                &layout_spec,
                &Default::default(),
                &ctx,
                &params,
                None,
            )
            .await
            .expect("build above-threshold chrome");

        assert!(above_threshold.guide_overflow.left);
        assert!(above_threshold.guide_overflow.right);
        assert!(above_threshold.guide_overflow.top);
        assert!(above_threshold.guide_overflow.bottom);
        // Existing overflow layers pixel-align by ceiling.
        assert_eq!(above_threshold.frame.horizontal.leading.inner, 3.0);
        assert_eq!(above_threshold.frame.horizontal.trailing.inner, 4.0);
        assert_eq!(above_threshold.frame.vertical.leading.inner, 3.0);
        assert_eq!(above_threshold.frame.vertical.trailing.inner, 4.0);
        assert_eq!(above_threshold.frame.horizontal.leading.margin, 10.0);
        assert!(!above_threshold.has_title_band);
        assert!(!above_threshold.has_subtitle_band);
    }
}
