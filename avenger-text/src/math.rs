use avenger_typst::{
    AvengerTypst, MathDelimiterInfo, MathDelimiterOptions, MathLimits, MathOutputRequest,
    MathRunArtifact, MathStringArtifact, MathStringOptions, MathStringRun, MathStyle,
    MathSyntaxMode, TypesetMetrics, TypstEngineConfig,
};

use crate::{
    measurement::{
        FontMetrics, FontMetricsConfig, TextBounds, TextMeasurementConfig, TextMeasurer,
    },
    types::{FontStyle, FontWeight},
};

pub(crate) const DEFAULT_MATH_LINE_LEADING_FACTOR: f32 = 0.65;

#[derive(Debug, Clone, PartialEq)]
pub enum TextMarkupMode {
    Plain,
    TypstMathDelimited(MathDelimiterOptions),
}

impl Default for TextMarkupMode {
    fn default() -> Self {
        Self::Plain
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathMarkupErrorPolicy {
    TreatInvalidMathAsLiteral,
    UseFallbackBounds,
    ErrorOnPathExtraction,
}

impl Default for MathMarkupErrorPolicy {
    fn default() -> Self {
        Self::TreatInvalidMathAsLiteral
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextMathConfig {
    pub mode: TextMarkupMode,
    pub math_style: MathStyle,
    pub syntax: MathSyntaxMode,
    pub limits: MathLimits,
    pub error_policy: MathMarkupErrorPolicy,
}

impl Default for TextMathConfig {
    fn default() -> Self {
        Self {
            mode: TextMarkupMode::Plain,
            math_style: MathStyle::default(),
            syntax: MathSyntaxMode::default(),
            limits: MathLimits::default(),
            error_policy: MathMarkupErrorPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MathAwareTextRun {
    Plain {
        text: String,
        byte_range: std::ops::Range<usize>,
    },
    Math {
        source: String,
        byte_range: std::ops::Range<usize>,
        delimiter: MathDelimiterInfo,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum MathAwareLaidOutRun {
    Plain {
        text: String,
        byte_range: std::ops::Range<usize>,
        x: f32,
        y_offset: f32,
        bounds: TextBounds,
    },
    Math {
        source: String,
        byte_range: std::ops::Range<usize>,
        delimiter: MathDelimiterInfo,
        x: f32,
        y_offset: f32,
        metrics: TypesetMetrics,
        artifact: MathRunArtifact,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct MathAwareTextLayout {
    pub bounds: TextBounds,
    pub runs: Vec<MathAwareLaidOutRun>,
}

#[derive(Debug, Clone)]
pub struct MathAwareTextMeasurer<P> {
    plain: P,
    typst: AvengerTypst,
    math: TextMathConfig,
}

impl<P> MathAwareTextMeasurer<P> {
    pub fn new(plain: P, typst: AvengerTypst, math: TextMathConfig) -> Self {
        Self { plain, typst, math }
    }

    pub fn with_default_typst(
        plain: P,
        math: TextMathConfig,
    ) -> Result<Self, avenger_typst::TypstInitError> {
        Ok(Self::new(
            plain,
            AvengerTypst::new(TypstEngineConfig::default())?,
            math,
        ))
    }

    pub fn with_vendor_typst(
        plain: P,
        math: TextMathConfig,
    ) -> Result<Self, avenger_typst::TypstInitError> {
        Ok(Self::new(
            plain,
            AvengerTypst::new(TypstEngineConfig {
                backend: avenger_typst::TypstEngineBackend::VendorTypst,
                ..TypstEngineConfig::default()
            })?,
            math,
        ))
    }

    pub fn plain(&self) -> &P {
        &self.plain
    }

    pub fn math_config(&self) -> &TextMathConfig {
        &self.math
    }
}

impl<P> TextMeasurer for MathAwareTextMeasurer<P>
where
    P: TextMeasurer,
{
    fn measure_text_bounds(&self, config: &TextMeasurementConfig) -> TextBounds {
        measure_math_aware_text(&self.plain, &self.typst, &self.math, config)
            .map(|layout| layout.bounds)
            .unwrap_or_else(|| self.plain.measure_text_bounds(config))
    }

    fn measure_font_metrics(&self, config: &FontMetricsConfig) -> FontMetrics {
        self.plain.measure_font_metrics(config)
    }
}

pub fn parse_math_aware_runs(
    typst: &AvengerTypst,
    math: &TextMathConfig,
    text: &str,
    font_size: f32,
) -> Result<Vec<MathAwareTextRun>, avenger_typst::MathTypesetError> {
    let options = math_string_options(math, font_size);
    let artifact = typst.typeset_math_string(text, &options)?;

    Ok(artifact
        .runs
        .into_iter()
        .map(|run| match run {
            MathStringRun::Plain(run) => MathAwareTextRun::Plain {
                text: run.text,
                byte_range: run.byte_range,
            },
            MathStringRun::Math(run) => MathAwareTextRun::Math {
                source: run.source,
                byte_range: run.byte_range,
                delimiter: run.delimiter,
            },
        })
        .collect())
}

pub fn measure_math_aware_text<P>(
    plain: &P,
    typst: &AvengerTypst,
    math: &TextMathConfig,
    config: &TextMeasurementConfig,
) -> Option<MathAwareTextLayout>
where
    P: TextMeasurer,
{
    if matches!(math.mode, TextMarkupMode::Plain) || config.text.is_empty() {
        return None;
    }

    let options = math_string_options(math, config.font_size);
    let artifact = match typst.typeset_math_string(config.text, &options) {
        Ok(artifact) => artifact,
        Err(_) => {
            return match math.error_policy {
                MathMarkupErrorPolicy::TreatInvalidMathAsLiteral
                | MathMarkupErrorPolicy::ErrorOnPathExtraction => None,
                MathMarkupErrorPolicy::UseFallbackBounds => Some(MathAwareTextLayout {
                    bounds: self_fallback_bounds(plain, config),
                    runs: vec![MathAwareLaidOutRun::Plain {
                        text: config.text.to_string(),
                        byte_range: 0..config.text.len(),
                        x: 0.0,
                        y_offset: 0.0,
                        bounds: self_fallback_bounds(plain, config),
                    }],
                }),
            };
        }
    };

    if !artifact
        .runs
        .iter()
        .any(|run| matches!(run, MathStringRun::Math(_)))
    {
        return None;
    }

    layout_math_string_artifact(plain, artifact, config)
}

pub(crate) fn layout_math_string_artifact<P>(
    plain: &P,
    artifact: MathStringArtifact,
    config: &TextMeasurementConfig,
) -> Option<MathAwareTextLayout>
where
    P: TextMeasurer,
{
    let mut measured_runs = Vec::new();
    let mut width = 0.0f32;
    let mut tight_ascent = 0.0f32;
    let mut tight_descent = 0.0f32;
    let mut math_ascent = 0.0f32;
    let mut math_descent = 0.0f32;
    let mut line_height = 0.0f32;
    let line_box = font_line_box(plain, config);

    for run in artifact.runs {
        match run {
            MathStringRun::Plain(run) => {
                let run_bounds = measure_plain_run(
                    plain,
                    &run.text,
                    config.font,
                    config.font_size,
                    config.font_weight,
                    config.font_style,
                );
                tight_ascent = tight_ascent.max(run_bounds.ascent);
                tight_descent = tight_descent.max(run_bounds.descent);
                line_height = line_height.max(run_bounds.line_height);
                let run_width = run_bounds.width;
                measured_runs.push(PendingRun::Plain {
                    text: run.text,
                    byte_range: run.byte_range,
                    width: run_width,
                    ascent: run_bounds.ascent,
                    bounds: run_bounds,
                });
                width += run_width;
            }
            MathStringRun::Math(run) => {
                let metrics = run.artifact.metrics;
                tight_ascent = tight_ascent.max(metrics.ascent);
                tight_descent = tight_descent.max(metrics.descent);
                math_ascent = math_ascent.max(metrics.ascent);
                math_descent = math_descent.max(metrics.descent);
                line_height = line_height.max(metrics.height);
                measured_runs.push(PendingRun::Math {
                    source: run.source,
                    byte_range: run.byte_range,
                    delimiter: run.delimiter,
                    width: metrics.width,
                    ascent: metrics.ascent,
                    metrics,
                    artifact: run.artifact,
                });
                width += metrics.width;
            }
        }
    }

    let mut ascent = tight_ascent.max(line_box.ascent);
    let mut descent = tight_descent.max(line_box.descent);
    if math_ascent > line_box.ascent || math_descent > line_box.descent {
        let leading = math_line_leading(config.font_size, line_box.leading);
        ascent += leading * 0.5;
        descent += leading * 0.5;
    }
    let height = ascent + descent;
    let bounds = TextBounds {
        width,
        height,
        ascent,
        descent,
        line_height: line_height.max(line_box.line_height).max(height),
    };

    let mut x = 0.0;
    let runs = measured_runs
        .into_iter()
        .map(|run| {
            let run_x = x;
            x += run.width();
            run.into_laid_out(run_x, ascent)
        })
        .collect();

    Some(MathAwareTextLayout { bounds, runs })
}

#[derive(Debug, Clone, Copy)]
struct LineBox {
    ascent: f32,
    descent: f32,
    line_height: f32,
    leading: f32,
}

fn font_line_box<P>(plain: &P, config: &TextMeasurementConfig) -> LineBox
where
    P: TextMeasurer,
{
    let metrics = plain.measure_font_metrics(&FontMetricsConfig {
        font: config.font,
        font_size: config.font_size,
        font_weight: config.font_weight,
        font_style: config.font_style,
    });
    let font_height = metrics.height.max(0.0);
    let line_height = metrics
        .line_height
        .max(font_height)
        .max(config.font_size.max(0.0));
    let leading = (line_height - font_height).max(0.0);

    LineBox {
        ascent: metrics.ascent.max(0.0) + leading * 0.5,
        descent: metrics.descent.max(0.0) + leading * 0.5,
        line_height,
        leading,
    }
}

fn math_line_leading(font_size: f32, font_leading: f32) -> f32 {
    font_leading.max(font_size.max(0.0) * DEFAULT_MATH_LINE_LEADING_FACTOR)
}

fn math_string_options(math: &TextMathConfig, font_size: f32) -> MathStringOptions {
    math_string_options_with_outputs(
        math,
        font_size,
        MathOutputRequest {
            paths: false,
            raster: None,
            pdf_text_layer: false,
        },
    )
}

pub(crate) fn math_string_options_with_outputs(
    math: &TextMathConfig,
    font_size: f32,
    outputs: MathOutputRequest,
) -> MathStringOptions {
    let delimiters = match &math.mode {
        TextMarkupMode::Plain => MathDelimiterOptions::default(),
        TextMarkupMode::TypstMathDelimited(delimiters) => delimiters.clone(),
    };
    let mut math_style = math.math_style.clone();
    math_style.font_size = font_size;

    MathStringOptions {
        text_style: Default::default(),
        math_style,
        outputs,
        delimiters,
        syntax: math.syntax,
        limits: math.limits,
    }
}

fn measure_plain_run<P>(
    plain: &P,
    text: &str,
    font: &str,
    font_size: f32,
    font_weight: &FontWeight,
    font_style: &FontStyle,
) -> TextBounds
where
    P: TextMeasurer,
{
    plain.measure_text_bounds(&TextMeasurementConfig {
        text,
        font,
        font_size,
        font_weight,
        font_style,
    })
}

fn self_fallback_bounds<P>(plain: &P, config: &TextMeasurementConfig) -> TextBounds
where
    P: TextMeasurer,
{
    plain.measure_text_bounds(config)
}

enum PendingRun {
    Plain {
        text: String,
        byte_range: std::ops::Range<usize>,
        width: f32,
        ascent: f32,
        bounds: TextBounds,
    },
    Math {
        source: String,
        byte_range: std::ops::Range<usize>,
        delimiter: MathDelimiterInfo,
        width: f32,
        ascent: f32,
        metrics: TypesetMetrics,
        artifact: MathRunArtifact,
    },
}

impl PendingRun {
    fn width(&self) -> f32 {
        match self {
            Self::Plain { width, .. } | Self::Math { width, .. } => *width,
        }
    }

    fn into_laid_out(self, x: f32, layout_ascent: f32) -> MathAwareLaidOutRun {
        match self {
            Self::Plain {
                text,
                byte_range,
                ascent,
                bounds,
                ..
            } => MathAwareLaidOutRun::Plain {
                text,
                byte_range,
                x,
                y_offset: layout_ascent - ascent,
                bounds,
            },
            Self::Math {
                source,
                byte_range,
                delimiter,
                ascent,
                metrics,
                artifact,
                ..
            } => MathAwareLaidOutRun::Math {
                source,
                byte_range,
                delimiter,
                x,
                y_offset: layout_ascent - ascent,
                metrics,
                artifact,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FontWeightNameSpec;
    use avenger_typst::{MathDisplayHint, MathRun};

    #[derive(Debug, Clone)]
    struct FixedMeasurer;

    impl TextMeasurer for FixedMeasurer {
        fn measure_text_bounds(&self, config: &TextMeasurementConfig) -> TextBounds {
            TextBounds {
                width: config.text.chars().count() as f32 * 10.0,
                height: 10.0,
                ascent: 7.0,
                descent: 3.0,
                line_height: 12.0,
            }
        }

        fn measure_font_metrics(&self, config: &FontMetricsConfig) -> FontMetrics {
            FontMetrics::fallback(config.font_size)
        }
    }

    fn typst() -> AvengerTypst {
        AvengerTypst::new(TypstEngineConfig::default()).unwrap()
    }

    fn math_config() -> TextMathConfig {
        TextMathConfig {
            mode: TextMarkupMode::TypstMathDelimited(MathDelimiterOptions::default()),
            ..Default::default()
        }
    }

    fn measurement_config<'a>(text: &'a str) -> TextMeasurementConfig<'a> {
        static WEIGHT: FontWeight = FontWeight::Name(FontWeightNameSpec::Normal);
        static STYLE: FontStyle = FontStyle::Normal;
        TextMeasurementConfig {
            text,
            font: "sans-serif",
            font_size: 10.0,
            font_weight: &WEIGHT,
            font_style: &STYLE,
        }
    }

    fn synthetic_math_string_artifact(metrics: TypesetMetrics) -> MathStringArtifact {
        MathStringArtifact {
            source: "$x$".to_string(),
            runs: vec![MathStringRun::Math(MathRun {
                source: "x".to_string(),
                byte_range: 1..2,
                delimiter: MathDelimiterInfo {
                    full_range: 0..3,
                    opening_range: 0..1,
                    closing_range: 2..3,
                    display_hint: MathDisplayHint::Inline,
                },
                artifact: MathRunArtifact {
                    metrics,
                    paths: None,
                    raster: None,
                    pdf_text: None,
                    font_resources: Vec::new(),
                    warnings: Vec::new(),
                },
            })],
            font_resources: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn assert_close(left: f32, right: f32) {
        assert!(
            (left - right).abs() <= 1e-4,
            "expected {left} to be close to {right}"
        );
    }

    #[test]
    fn plain_mode_returns_none_for_layout() {
        let layout = measure_math_aware_text(
            &FixedMeasurer,
            &typst(),
            &TextMathConfig::default(),
            &measurement_config("$x$"),
        );

        assert!(layout.is_none());
    }

    #[test]
    fn no_math_delimiter_returns_none_for_passthrough() {
        let layout = measure_math_aware_text(
            &FixedMeasurer,
            &typst(),
            &math_config(),
            &measurement_config("plain label"),
        );

        assert!(layout.is_none());
    }

    #[test]
    fn escaped_dollar_stays_plain_when_markup_enabled() {
        let layout = measure_math_aware_text(
            &FixedMeasurer,
            &typst(),
            &math_config(),
            &measurement_config(r"Cost is \$5 and $x$"),
        )
        .unwrap();

        assert!(matches!(
            &layout.runs[0],
            MathAwareLaidOutRun::Plain { text, .. } if text == "Cost is $5 and "
        ));
        assert!(
            matches!(&layout.runs[1], MathAwareLaidOutRun::Math { source, .. } if source == "x")
        );
    }

    #[test]
    fn mixed_layout_composes_plain_and_math_runs() {
        let layout = measure_math_aware_text(
            &FixedMeasurer,
            &typst(),
            &math_config(),
            &measurement_config("speed $v^2$"),
        )
        .unwrap();

        assert_eq!(layout.runs.len(), 2);
        assert!(layout.bounds.width > 60.0);
        assert!(layout.bounds.ascent >= 7.0);
        assert!(layout.bounds.descent >= 3.0);
        assert!(matches!(
            &layout.runs[0],
            MathAwareLaidOutRun::Plain { text, x, .. } if text == "speed " && *x == 0.0
        ));
        assert!(matches!(
            &layout.runs[1],
            MathAwareLaidOutRun::Math { source, x, .. } if source == "v^2" && *x == 60.0
        ));
    }

    #[test]
    fn math_layout_uses_plain_line_box_for_short_math() {
        let layout = layout_math_string_artifact(
            &FixedMeasurer,
            synthetic_math_string_artifact(TypesetMetrics {
                width: 8.0,
                height: 6.0,
                baseline: 4.0,
                ascent: 4.0,
                descent: 2.0,
            }),
            &measurement_config("$x$"),
        )
        .unwrap();

        assert_close(layout.bounds.height, 12.0);
        assert_close(layout.bounds.ascent, 9.0);
        assert_close(layout.bounds.descent, 3.0);
        assert!(matches!(
            &layout.runs[0],
            MathAwareLaidOutRun::Math { y_offset, .. } if *y_offset > 0.0
        ));
    }

    #[test]
    fn tall_math_layout_adds_normal_leading() {
        let layout = layout_math_string_artifact(
            &FixedMeasurer,
            synthetic_math_string_artifact(TypesetMetrics {
                width: 12.0,
                height: 20.0,
                baseline: 12.0,
                ascent: 12.0,
                descent: 8.0,
            }),
            &measurement_config("$x$"),
        )
        .unwrap();

        assert_close(layout.bounds.height, 26.5);
        assert_close(layout.bounds.ascent, 15.25);
        assert_close(layout.bounds.descent, 11.25);
        assert!(matches!(
            &layout.runs[0],
            MathAwareLaidOutRun::Math { y_offset, .. } if *y_offset == 3.25
        ));
    }

    #[test]
    fn invalid_math_can_fall_back_to_literal_plain_text() {
        let measurer = MathAwareTextMeasurer::new(FixedMeasurer, typst(), math_config());
        let bounds = measurer.measure_text_bounds(&measurement_config("$x^$"));

        assert_eq!(bounds.width, 40.0);
    }
}
