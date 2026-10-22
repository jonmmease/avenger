//! Ticks for scales with a numeric or temporal domain, such as linear, log and time scales: how
//! many an axis of a length shows, which ones it labels, and their label text.

use arrow::{array::ArrayRef, datatypes::DataType};
use avenger_common::types::FontStyle;
use avenger_scales::scales::{time, ConfiguredScale};
use avenger_scenegraph::marks::text::text_style;
use avenger_typst_label::{LabelEngine, LabelValue};

use super::{
    format_values,
    opts::{AxisStyle, AxisTickInterval},
};
use crate::{error::AvengerGuidesError, GuideSide};

/// The gap between the labels and the title.
pub(crate) const TITLE_MARGIN: f32 = 2.0;
const DEFAULT_MAX_TICK_COUNT: f64 = 10.0;
const HORIZONTAL_MIN_TICK_SPACING_PX: f32 = 25.0;
const VERTICAL_TICK_SPACING_FONT_HEIGHT_FACTOR: f32 = 1.6;

/// The smallest space between ticks that an axis on `side` fits its tick count to: a fixed
/// spacing along a horizontal axis, and a multiple of the label font's height along a vertical
/// one.
pub(crate) fn min_tick_spacing(side: GuideSide, style: &AxisStyle, engine: &LabelEngine) -> f32 {
    if !side.is_vertical() {
        return HORIZONTAL_MIN_TICK_SPACING_PX;
    }
    let font_height = engine
        .font_metrics(&text_style(
            &style.label_font_family,
            style.label_font_size,
            style.label_font_weight,
            FontStyle::Normal,
            style.label_color,
        ))
        .map_or(style.label_font_size, |metrics| {
            metrics.ascent + metrics.descent
        });
    (font_height * VERTICAL_TICK_SPACING_FONT_HEIGHT_FACTOR).max(1.0)
}

/// The style's tick count, or one fitted to an axis of `length` with ticks at least
/// `min_spacing` apart. A fitted count applies only when it is below the scale's default.
pub(crate) fn tick_count(style: &AxisStyle, length: f32, min_spacing: f32) -> Option<f64> {
    style.tick_count.or_else(|| {
        let count = f64::from((length / min_spacing).max(2.0));
        (count < DEFAULT_MAX_TICK_COUNT).then_some(count)
    })
}

/// The ticks an axis draws, and the ones it labels.
pub(crate) fn axis_ticks(
    scale: &ConfiguredScale,
    style: &AxisStyle,
    tick_count: Option<f64>,
) -> Result<(ArrayRef, ArrayRef), AvengerGuidesError> {
    let ticks = match style.tick_start_step {
        Some(AxisTickInterval::Numeric { start, step }) => scale.start_step_ticks(start, step)?,
        Some(AxisTickInterval::Temporal {
            start_millis,
            months,
            days,
            nanos,
        }) => scale.temporal_start_step_ticks(start_millis, months, days, nanos)?,
        None => {
            let ticks = scale.ticks(tick_count)?;
            let labeled = scale.label_ticks(&ticks, tick_count);
            return Ok((ticks, labeled));
        }
    };
    Ok((ticks.clone(), ticks))
}

/// The ticks as the text engine formats them, together, so that they share digits.
pub(crate) fn continuous_tick_text(
    ticks: &ArrayRef,
    scale: &ConfiguredScale,
    tick_format: &str,
    engine: &LabelEngine,
) -> Result<Vec<LabelValue>, AvengerGuidesError> {
    check_timezone(ticks, scale, engine)?;
    format_values(ticks, tick_format, scale.tick_precision_sharing(), engine)
}

/// Zoned ticks must show in the timezone that the scale places them in.
fn check_timezone(
    ticks: &ArrayRef,
    scale: &ConfiguredScale,
    engine: &LabelEngine,
) -> Result<(), AvengerGuidesError> {
    if let DataType::Timestamp(_, Some(_)) = ticks.data_type() {
        let scale = time::timezone(&scale.config)?;
        let formatter = engine.datetime_timezone()?;
        if formatter != scale {
            return Err(AvengerGuidesError::TimezoneMismatch { scale, formatter });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::super::opts::AxisStyle;
    use super::*;
    use arrow::array::{AsArray, Date32Array, Float64Array};
    use arrow::datatypes::Float64Type;
    use avenger_common::types::TextSyntaxMode;
    use avenger_scales::scales::linear::LinearScale;
    use avenger_scales::scales::time::TimeScale;
    use avenger_scenegraph::marks::text::text_label;
    use avenger_typst_label::LabelOptions;

    use super::super::{opts::AxisConfig, tick_label_text};

    fn config(pattern: &str, tick_label: Option<&str>, style: AxisStyle) -> AxisConfig {
        AxisConfig {
            side: GuideSide::Bottom,
            tick_format: pattern.to_string(),
            tick_label: tick_label.map(str::to_string),
            style,
        }
    }

    fn values(array: &ArrayRef) -> Vec<f64> {
        array.as_primitive::<Float64Type>().values().to_vec()
    }

    /// Tick label text as the axis shows it.
    fn label_text(
        ticks: &ArrayRef,
        scale: &ConfiguredScale,
        config: &AxisConfig,
    ) -> (Vec<String>, TextSyntaxMode) {
        let ticks =
            continuous_tick_text(ticks, scale, &config.tick_format, &crate::test_engine()).unwrap();
        tick_label_text(ticks, config.tick_label.as_deref()).unwrap()
    }

    /// Start-step settings replace the scale's ticks and label every one of them.
    #[test]
    fn start_step_ticks_replace_the_scale_ticks() {
        let scale = LinearScale::configured((0.0, 0.035), (0.0, 100.0));
        let style = AxisStyle {
            tick_start_step: Some(AxisTickInterval::Numeric {
                start: 0.0,
                step: 0.01,
            }),
            ..Default::default()
        };
        let numbers = config(",", None, style);
        let (ticks, labeled) = axis_ticks(&scale, &numbers.style, None).unwrap();
        assert_eq!(values(&ticks), vec![0.0, 0.01, 0.02, 0.03]);
        assert_eq!(values(&labeled), values(&ticks));
        let (text, _) = label_text(&ticks, &scale, &numbers);
        assert_eq!(text, ["0", "0.01", "0.02", "0.03"]);

        let start = Arc::new(Date32Array::from(vec![19723])) as ArrayRef;
        let end = Arc::new(Date32Array::from(vec![19800])) as ArrayRef;
        let scale = TimeScale::configured((start, end), (0.0, 100.0));
        let style = AxisStyle {
            tick_start_step: Some(AxisTickInterval::Temporal {
                start_millis: 1_704_067_200_000,
                months: 1,
                days: 0,
                nanos: 0,
            }),
            ..Default::default()
        };
        let dates = config("%b %d", None, style);
        let (ticks, _) = axis_ticks(&scale, &dates.style, None).unwrap();
        let (text, _) = label_text(&ticks, &scale, &dates);
        assert_eq!(text, ["Jan 01", "Feb 01", "Mar 01"]);
    }

    /// Exponent ticks are Typst math.
    #[test]
    fn exponent_ticks_are_math() {
        let scale = LinearScale::configured((0.0, 2000.0), (0.0, 100.0));
        let ticks = Arc::new(Float64Array::from(vec![1200.0])) as ArrayRef;
        let (text, syntax) = label_text(&ticks, &scale, &config(".1e", None, Default::default()));
        assert_eq!(syntax, TextSyntaxMode::TypstMarkup);
        assert_eq!(text, [r#"$#"1.2" times 10^(3)$"#]);
    }

    #[test]
    fn tick_labels_bind_each_tick() {
        let scale = LinearScale::configured((0.0, 2000.0), (0.0, 100.0));
        let ticks = Arc::new(Float64Array::from(vec![1000.0, 2000.0])) as ArrayRef;
        let units = config(",.0f", Some("#tick m/s, `#tick`"), Default::default());
        let (text, syntax) = label_text(&ticks, &scale, &units);
        assert_eq!(syntax, TextSyntaxMode::TypstMarkup);
        assert_eq!(
            text,
            [r#"#("1,000") m/s, `#tick`"#, r#"#("2,000") m/s, `#tick`"#]
        );

        // Exponent ticks are math in the label.
        let ticks = Arc::new(Float64Array::from(vec![1200.0])) as ArrayRef;
        let exponent = config(".1e", Some("*#tick*"), Default::default());
        let (text, _) = label_text(&ticks, &scale, &exponent);
        assert_eq!(text, [r#"*#([$#"1.2" times 10^(3)$])*"#]);
        for text in text {
            crate::test_engine()
                .bounds(&text_label(
                    &text,
                    TextSyntaxMode::TypstMarkup,
                    LabelOptions::default(),
                ))
                .unwrap();
        }

        // Dates are bound as strings, so markup characters in them show as written.
        let start = Arc::new(Date32Array::from(vec![19723])) as ArrayRef;
        let end = Arc::new(Date32Array::from(vec![19754])) as ArrayRef;
        let scale = TimeScale::configured((start.clone(), end), (0.0, 100.0));
        let dates = config("#%d*", Some("[#tick]"), Default::default());
        let (text, _) = label_text(&start, &scale, &dates);
        assert_eq!(text, [r##"[#("#01*")]"##]);
    }
}
