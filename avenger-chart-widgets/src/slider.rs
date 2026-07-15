//! Horizontal numeric Slider widget.

use avenger_chart::{
    event::{self, ChartEventBinding, ChartEventStream, ChartEventType},
    pixel_frame::{
        PixelFrameRectPositionChannels, PixelFrameSymbolPositionChannels,
        PixelFrameTextPositionChannels,
    },
    prelude::{
        AvengerChartError, ChannelExpr, ChartWidget, CoordinationScope, CursorStyle, Param,
        PixelFrame, Rect, Symbol, Text, TextSyntaxMode, ToolBehaviorExpansion, ToolParamSharing,
        WidgetAxisMeasureSpec, WidgetExpansion, WidgetExpansionContext, WidgetMeasureExpr,
        WidgetMeasureSpec, WidgetPresentationBindings, WidgetStyleProperty, WidgetTextMeasureAxis,
    },
};
use avenger_format_number::{
    NumberFormatContext, NumberFormatOverrides, ResolvedNumberLocale, format_number,
};
use datafusion::{
    arrow::datatypes::DataType,
    common::ScalarValue,
    functions::math::expr_fn::round,
    logical_expr::{Expr, expr_fn::cast, lit},
    prelude::when,
};
use indexmap::IndexMap;

/// A horizontal numeric control with min-anchored step quantization.
#[derive(Clone, Debug)]
pub struct Slider {
    id: String,
    min: f64,
    max: f64,
    step: f64,
    title: String,
    format: String,
    value: Param,
    throttle_ms: Option<u64>,
}

impl Slider {
    /// Create a slider whose initial value is the lower bound.
    pub fn new(id: impl Into<String>, min: f64, max: f64) -> Self {
        let id = id.into();
        Self {
            value: {
                let __avenger_param_name = format!("{id}__value");
                let __avenger_param_default: datafusion::common::ScalarValue = (min).into();
                Param::typed(
                    __avenger_param_name,
                    __avenger_param_default.data_type(),
                    __avenger_param_default,
                )
                .expect("a parameter default must match its selected physical type")
            },
            id,
            min,
            max,
            step: 1.0,
            title: String::new(),
            format: String::new(),
            throttle_ms: None,
        }
    }

    /// Set the positive value-unit step used for pointer quantization.
    pub fn step(mut self, step: f64) -> Self {
        self.step = step;
        self
    }

    /// Set the initial value. It is clamped and normalized to the step grid
    /// when the widget is expanded.
    pub fn default(mut self, value: f64) -> Self {
        self.value.default = ScalarValue::Float64(Some(value));
        self
    }

    /// Set the visible caption shown above the track.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Set the d3-compatible value-label number format.
    pub fn format(mut self, format: impl Into<String>) -> Self {
        self.format = format.into();
        self
    }

    /// Limit drag updates to at most one per interval.
    pub fn throttle_ms(mut self, throttle_ms: u64) -> Self {
        self.throttle_ms = Some(throttle_ms);
        self
    }

    /// Use an externally named Float64 value parameter.
    pub fn value_param(mut self, param: Param) -> Self {
        self.value = param;
        self
    }

    /// The value parameter registered by this widget.
    pub fn param(&self) -> &Param {
        &self.value
    }

    /// An expression for the current slider value.
    pub fn value(&self) -> Expr {
        self.value.expr()
    }

    fn resolved_value_param(&self) -> Result<Param, AvengerChartError> {
        if !self.min.is_finite() || !self.max.is_finite() || self.max <= self.min {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Slider '{}' requires finite bounds with max greater than min",
                self.id
            )));
        }
        if !self.step.is_finite() || self.step <= 0.0 {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Slider '{}' step must be finite and greater than zero",
                self.id
            )));
        }
        let ScalarValue::Float64(Some(default)) = &self.value.default else {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Slider '{}' value parameter '{}' must have a non-null Float64 default",
                self.id, self.value.name
            )));
        };
        if !default.is_finite() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Slider '{}' default must be finite",
                self.id
            )));
        }
        let default = (self.min + ((*default - self.min) / self.step).round() * self.step)
            .clamp(self.min, self.max);
        let locale = ResolvedNumberLocale::en_us();
        format_number(
            default,
            Some(&self.format),
            NumberFormatOverrides::default(),
            NumberFormatContext::new(&locale),
        )
        .map_err(|err| {
            AvengerChartError::InvalidArgument(format!(
                "Slider '{}' has an invalid number format '{}': {err}",
                self.id, self.format
            ))
        })?;
        let value = {
            let __avenger_param_name = self.value.name.clone();
            let __avenger_param_default: datafusion::common::ScalarValue =
                (ScalarValue::Float64(Some(default))).into();
            Param::typed(
                __avenger_param_name,
                __avenger_param_default.data_type(),
                __avenger_param_default,
            )
            .expect("a parameter default must match its selected physical type")
        };
        let value_markup = value_markup(&value, &self.format);
        avenger_chart_core::scalar_params_for_label_source(
            &value_markup,
            TextSyntaxMode::TypstMarkup,
            &IndexMap::from([(value.name.clone(), value.default.clone())]),
        )
        .map_err(|err| {
            AvengerChartError::InvalidArgument(format!(
                "Slider '{}' value parameter '{}' cannot be used in its formatted label: {err}",
                self.id, value.name
            ))
        })?;
        Ok(value)
    }
}

impl ChartWidget for Slider {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> &'static str {
        "slider"
    }

    fn expand(
        &self,
        ctx: WidgetExpansionContext<'_>,
    ) -> Result<WidgetExpansion, AvengerChartError> {
        let value = self.resolved_value_param()?;
        let width = ctx.frame_width();
        let height = ctx.frame_height();
        let padding = ctx.host_style(WidgetStyleProperty::PaddingInline);
        let handle_size = ctx.part_style("handle", WidgetStyleProperty::SliderHandleSize);
        let track_height = ctx.part_style("track", WidgetStyleProperty::SliderTrackHeight);
        let nominal_x0 = padding + handle_size.clone() / lit(2.0_f32);
        let has_track = width.clone().gt(nominal_x0.clone() * lit(2.0_f32));
        let track_x0 =
            when(has_track.clone(), nominal_x0).otherwise(width.clone() / lit(2.0_f32))?;
        let track_width = when(has_track, width.clone() - track_x0.clone() * lit(2.0_f32))
            .otherwise(lit(0.0_f32))?;
        let ratio = (value.expr() - lit(self.min)) / lit(self.max - self.min);
        let handle_x = track_x0.clone() + ratio * track_width.clone();
        let handle_y = height.clone() - handle_size.clone() / lit(2.0_f32);
        let track_y = handle_y.clone() - track_height.clone() / lit(2.0_f32);

        let track = Rect::<PixelFrame>::new()
            .id("track")
            .x(track_x0.clone())
            .x2(track_x0.clone() + track_width.clone())
            .y(track_y.clone())
            .y2(track_y.clone() + track_height.clone());
        let fill = Rect::<PixelFrame>::new()
            .id("fill")
            .x(track_x0.clone())
            .x2(handle_x.clone())
            .y(track_y.clone())
            .y2(track_y + track_height);
        let handle = Symbol::<PixelFrame>::new()
            .id("handle")
            .x(handle_x.clone())
            .y(handle_y.clone())
            .size(ChannelExpr::value(
                handle_size.clone() * handle_size.clone(),
            ))
            .shape("circle");
        let focus_gap = ctx.part_style("focus-ring", WidgetStyleProperty::FocusGap);
        let focus_size = handle_size + focus_gap.clone() * lit(2.0_f32);
        let focus = Symbol::<PixelFrame>::new()
            .id("focus-ring")
            .x(handle_x)
            .y(handle_y)
            .size(ChannelExpr::value(focus_size.clone() * focus_size))
            .shape("circle")
            .fill("transparent");
        let label_y = ctx.part_style("label", WidgetStyleProperty::FontSize) / lit(2.0_f32);
        let label = Text::<PixelFrame>::new()
            .id("label")
            .x(0.0)
            .y(label_y.clone())
            .text(self.title.as_str())
            .align("left")
            .baseline("middle");
        let value_markup = value_markup(&value, &self.format);
        let value_label = Text::<PixelFrame>::new()
            .id("value-label")
            .x(width)
            .y(label_y)
            .text(value_markup.as_str())
            .align("right")
            .baseline("middle")
            .typst();

        let shared = ToolParamSharing::Explicit(CoordinationScope::Shared);
        let targets = [
            format!("{}.track", self.id),
            format!("{}.fill", self.id),
            format!("{}.handle", self.id),
        ];
        let (pointer_value, positive_track) =
            pointer_value_expr(&ctx, &value, self.min, self.max, self.step)?;
        let start = ChartEventStream::on(ChartEventType::MouseDown)
            .marks(targets.clone())
            .filter(event::button().eq(lit("left")));
        let end = ChartEventStream::on(ChartEventType::MouseUp);
        let mut drag = ChartEventBinding::on(ChartEventType::CursorMoved)
            .between(start, end)
            .filter(positive_track.clone())
            .set_param_at_start_scope(&value, pointer_value.clone())
            .preview()
            .settle_exact();
        if let Some(throttle_ms) = self.throttle_ms {
            drag = drag.throttle_ms(throttle_ms);
        }

        let behavior = ToolBehaviorExpansion::new(ctx.behavior_instance_id.clone())
            .param(value.clone(), shared)
            .mark(track)
            .mark(fill)
            .mark(handle)
            .mark(label)
            .mark(value_label)
            .mark(focus)
            .event_binding(
                ChartEventBinding::on(ChartEventType::MouseDown)
                    .marks(targets.clone())
                    .filter(event::button().eq(lit("left")))
                    .filter(positive_track)
                    .set_param(&value, pointer_value)
                    .preview()
                    .settle_exact(),
            )
            .event_binding(drag)
            .event_binding(
                ChartEventBinding::on(ChartEventType::MarkMouseEnter)
                    .marks(targets.clone())
                    .set_cursor(event::cursor(CursorStyle::Pointer)),
            )
            .event_binding(
                ChartEventBinding::on(ChartEventType::MarkMouseLeave)
                    .marks(targets)
                    .set_cursor(event::cursor(CursorStyle::Default)),
            );

        Ok(WidgetExpansion {
            instance_id: ctx.instance_id,
            behavior,
            items: None,
            measure: WidgetMeasureSpec {
                width: WidgetAxisMeasureSpec::StyledFill {
                    preferred: WidgetMeasureExpr::Max(vec![
                        WidgetMeasureExpr::StyleLength {
                            part: None,
                            property: WidgetStyleProperty::MinWidth,
                        },
                        WidgetMeasureExpr::StyleLength {
                            part: Some("track".to_string()),
                            property: WidgetStyleProperty::SliderMinWidth,
                        },
                        WidgetMeasureExpr::Add(vec![
                            WidgetMeasureExpr::TextExtent {
                                part: "label".to_string(),
                                axis: WidgetTextMeasureAxis::Width,
                                data_encoded: false,
                            },
                            WidgetMeasureExpr::StyleLength {
                                part: Some("value-label".to_string()),
                                property: WidgetStyleProperty::SliderValuePadding,
                            },
                            WidgetMeasureExpr::TextExtent {
                                part: "value-label".to_string(),
                                axis: WidgetTextMeasureAxis::Width,
                                data_encoded: false,
                            },
                        ]),
                    ]),
                    min: WidgetMeasureExpr::Max(vec![
                        WidgetMeasureExpr::StyleLength {
                            part: None,
                            property: WidgetStyleProperty::MinWidth,
                        },
                        WidgetMeasureExpr::StyleLength {
                            part: Some("track".to_string()),
                            property: WidgetStyleProperty::SliderMinWidth,
                        },
                    ]),
                    max_px: None,
                    stretch: 1.0,
                },
                height: WidgetAxisMeasureSpec::Content {
                    expr: WidgetMeasureExpr::StyleLength {
                        part: None,
                        property: WidgetStyleProperty::Height,
                    },
                    min_px: 1.0,
                    max_px: None,
                },
            },
            presentation: WidgetPresentationBindings::default().orientation("horizontal"),
        })
    }
}

fn value_markup(value: &Param, format: &str) -> String {
    format!(
        "#numfmt({}, {})",
        value.name,
        serde_json::to_string(format).expect("format string is serializable")
    )
}

fn pointer_value_expr(
    ctx: &WidgetExpansionContext<'_>,
    current: &Param,
    min: f64,
    max: f64,
    step: f64,
) -> Result<(Expr, Expr), AvengerChartError> {
    let frame_width = event::frame_width();
    let padding = cast(
        ctx.host_style(WidgetStyleProperty::PaddingInline),
        DataType::Float64,
    );
    let handle_size = cast(
        ctx.part_style("handle", WidgetStyleProperty::SliderHandleSize),
        DataType::Float64,
    );
    let nominal_x0 = padding + handle_size / lit(2.0_f64);
    let positive_track = frame_width.clone().gt(nominal_x0.clone() * lit(2.0_f64));
    let track_x0 =
        when(positive_track.clone(), nominal_x0).otherwise(frame_width.clone() / lit(2.0_f64))?;
    let track_width = when(
        positive_track.clone(),
        frame_width - track_x0.clone() * lit(2.0_f64),
    )
    .otherwise(lit(0.0_f64))?;
    let raw = lit(min) + ((event::frame_x() - track_x0) / track_width) * lit(max - min);
    let quantized = lit(min) + round(vec![(raw - lit(min)) / lit(step)]) * lit(step);
    let capped = when(quantized.clone().gt(lit(max)), lit(max)).otherwise(quantized)?;
    let clamped = when(capped.clone().lt(lit(min)), lit(min)).otherwise(capped)?;
    let guarded = when(positive_track.clone(), clamped).otherwise(current.expr())?;
    Ok((guarded, positive_track))
}
