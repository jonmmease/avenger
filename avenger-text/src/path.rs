use std::ops::Range;

use lyon_path::Path;

use crate::{
    error::AvengerTextError,
    measurement::{TextBounds, TextMeasurementConfig},
    types::{FontStyle, FontWeight},
};

#[cfg(feature = "typst-math")]
use lyon_path::{geom::point, Event};

#[cfg(any(feature = "cosmic-text", feature = "typst-math"))]
use crate::measurement::TextMeasurer;

#[cfg(feature = "cosmic-text")]
use std::sync::{Arc, Mutex};

#[cfg(feature = "cosmic-text")]
use crate::FontResolutionOptions;

#[cfg(feature = "cosmic-text")]
use cosmic_text::{FontSystem, SwashCache};

#[cfg(feature = "cosmic-text")]
use crate::measurement::cosmic::{make_cosmic_text_buffer, measure_text_buffer, FONT_SYSTEM};

#[cfg(feature = "cosmic-text")]
use crate::rasterization::cosmic::import_path_commands_with_offset;

#[cfg(feature = "typst-math")]
use crate::math::{
    layout_math_string_artifact, math_string_options_with_outputs, MathAwareLaidOutRun,
    MathMarkupErrorPolicy, TextMarkupMode, TextMathConfig,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextPathOutputMode {
    MathOnly,
    AllText,
}

impl Default for TextPathOutputMode {
    fn default() -> Self {
        Self::MathOnly
    }
}

#[derive(Debug, Clone)]
pub struct TextPathExtractionConfig<'a> {
    pub text: &'a String,
    pub color: &'a [f32; 4],
    pub font: &'a String,
    pub font_size: f32,
    pub font_weight: &'a FontWeight,
    pub font_style: &'a FontStyle,
    pub limit: f32,
    pub output_mode: TextPathOutputMode,
    pub include_pdf_text_layer: bool,
}

impl<'a> TextPathExtractionConfig<'a> {
    pub fn to_measurement_config(&self) -> TextMeasurementConfig<'a> {
        TextMeasurementConfig {
            text: self.text,
            font: self.font,
            font_size: self.font_size,
            font_weight: self.font_weight,
            font_style: self.font_style,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextPathStroke {
    pub color: [f32; 4],
    pub width: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TextPathKind {
    PlainGlyph,
    MathGlyph,
    MathShape,
}

#[derive(Debug, Clone)]
pub struct TextPathItem {
    pub path: Path,
    pub fill: Option<[f32; 4]>,
    pub stroke: Option<TextPathStroke>,
    pub byte_range: Range<usize>,
    pub kind: TextPathKind,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlainTextPathRun {
    pub text: String,
    pub byte_range: Range<usize>,
    pub x: f32,
    pub y_offset: f32,
    pub bounds: TextBounds,
}

#[cfg(feature = "typst-math")]
#[derive(Debug, Clone, PartialEq)]
pub struct TextMathPdfLayer {
    pub byte_range: Range<usize>,
    pub x: f32,
    pub y_offset: f32,
    pub layer: avenger_typst::MathPdfTextLayer,
    pub font_resources: Vec<avenger_typst::MathFontResource>,
}

#[derive(Debug, Clone)]
pub struct TextPathBuffer {
    pub bounds: TextBounds,
    pub items: Vec<TextPathItem>,
    pub plain_runs: Vec<PlainTextPathRun>,
    #[cfg(feature = "typst-math")]
    pub math_pdf_layers: Vec<TextMathPdfLayer>,
}

impl TextPathBuffer {
    pub fn new(bounds: TextBounds) -> Self {
        Self {
            bounds,
            items: Vec::new(),
            plain_runs: Vec::new(),
            #[cfg(feature = "typst-math")]
            math_pdf_layers: Vec::new(),
        }
    }
}

pub trait TextPathExtractor: Send + Sync {
    fn extract_text_paths(
        &self,
        config: &TextPathExtractionConfig,
    ) -> Result<TextPathBuffer, AvengerTextError>;
}

#[cfg(feature = "cosmic-text")]
#[derive(Clone)]
pub struct CosmicTextPathExtractor {
    resources: Arc<CosmicPathResources>,
}

#[cfg(feature = "cosmic-text")]
enum CosmicPathResources {
    Global,
    Local {
        font_system: Mutex<FontSystem>,
        swash_cache: Mutex<SwashCache>,
    },
}

#[cfg(feature = "cosmic-text")]
impl Default for CosmicTextPathExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "cosmic-text")]
impl CosmicTextPathExtractor {
    pub fn new() -> Self {
        Self {
            resources: Arc::new(CosmicPathResources::Global),
        }
    }

    pub fn with_font_resolution(options: FontResolutionOptions) -> Self {
        Self {
            resources: Arc::new(CosmicPathResources::Local {
                font_system: Mutex::new(crate::fonts::build_cosmic_font_system(&options)),
                swash_cache: Mutex::new(SwashCache::new()),
            }),
        }
    }
}

#[cfg(feature = "cosmic-text")]
impl TextPathExtractor for CosmicTextPathExtractor {
    fn extract_text_paths(
        &self,
        config: &TextPathExtractionConfig,
    ) -> Result<TextPathBuffer, AvengerTextError> {
        match self.resources.as_ref() {
            CosmicPathResources::Global => {
                let mut font_system = FONT_SYSTEM
                    .lock()
                    .expect("Failed to acquire lock on FONT_SYSTEM");
                let mut swash_cache = crate::measurement::cosmic::SWASH_CACHE
                    .lock()
                    .expect("Failed to acquire lock on SWASH_CACHE");
                extract_cosmic_paths_with_resources(config, &mut font_system, &mut swash_cache)
            }
            CosmicPathResources::Local {
                font_system,
                swash_cache,
            } => {
                let mut font_system = font_system
                    .lock()
                    .expect("Failed to acquire local FontSystem lock");
                let mut swash_cache = swash_cache
                    .lock()
                    .expect("Failed to acquire local SwashCache lock");
                extract_cosmic_paths_with_resources(config, &mut font_system, &mut swash_cache)
            }
        }
    }
}

#[cfg(feature = "cosmic-text")]
impl TextMeasurer for CosmicTextPathExtractor {
    fn measure_text_bounds(&self, config: &TextMeasurementConfig) -> TextBounds {
        match self.resources.as_ref() {
            CosmicPathResources::Global => {
                let mut font_system = FONT_SYSTEM
                    .lock()
                    .expect("Failed to acquire lock on FONT_SYSTEM");
                let buffer = make_cosmic_text_buffer(config, &mut font_system);
                measure_text_buffer(&buffer)
            }
            CosmicPathResources::Local { font_system, .. } => {
                let mut font_system = font_system
                    .lock()
                    .expect("Failed to acquire local FontSystem lock");
                let buffer = make_cosmic_text_buffer(config, &mut font_system);
                measure_text_buffer(&buffer)
            }
        }
    }

    fn measure_font_metrics(
        &self,
        config: &crate::measurement::FontMetricsConfig,
    ) -> crate::measurement::FontMetrics {
        crate::measurement::FontMetrics::fallback(config.font_size)
    }
}

#[cfg(feature = "cosmic-text")]
fn extract_cosmic_paths_with_resources(
    config: &TextPathExtractionConfig,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
) -> Result<TextPathBuffer, AvengerTextError> {
    let text = crate::measurement::truncate_text_to_limit_with(config.text, config.limit, |text| {
        let measurement = TextMeasurementConfig {
            text,
            font: config.font,
            font_size: config.font_size,
            font_weight: config.font_weight,
            font_style: config.font_style,
        };
        let buffer = make_cosmic_text_buffer(&measurement, font_system);
        measure_text_buffer(&buffer).width
    });
    let measurement = TextMeasurementConfig {
        text: &text,
        font: config.font,
        font_size: config.font_size,
        font_weight: config.font_weight,
        font_style: config.font_style,
    };
    let buffer = make_cosmic_text_buffer(&measurement, font_system);
    let bounds = measure_text_buffer(&buffer);
    let mut output = TextPathBuffer::new(bounds.clone());
    output.plain_runs.push(PlainTextPathRun {
        text: text.clone(),
        byte_range: 0..config.text.len(),
        x: 0.0,
        y_offset: 0.0,
        bounds: bounds.clone(),
    });

    let fill = Some(*config.color);
    for run in buffer.layout_runs() {
        for glyph in run.glyphs.iter() {
            let physical_glyph = glyph.physical((0.0, 0.0), 1.0);
            let Some(commands) =
                swash_cache.get_outline_commands(font_system, physical_glyph.cache_key)
            else {
                continue;
            };
            let x = glyph.x + glyph.font_size * glyph.x_offset;
            let y = bounds.ascent + glyph.y - glyph.font_size * glyph.y_offset;
            output.items.push(TextPathItem {
                path: import_path_commands_with_offset(&commands, x, y),
                fill,
                stroke: None,
                byte_range: 0..config.text.len(),
                kind: TextPathKind::PlainGlyph,
            });
        }
    }

    Ok(output)
}

#[cfg(feature = "typst-math")]
#[derive(Debug, Clone)]
pub struct MathAwareTextPathExtractor<P> {
    plain: P,
    typst: avenger_typst::AvengerTypst,
    math: TextMathConfig,
}

#[cfg(feature = "typst-math")]
impl<P> MathAwareTextPathExtractor<P> {
    pub fn new(plain: P, typst: avenger_typst::AvengerTypst, math: TextMathConfig) -> Self {
        Self { plain, typst, math }
    }

    pub fn with_vendor_typst(
        plain: P,
        math: TextMathConfig,
    ) -> Result<Self, avenger_typst::TypstInitError> {
        Ok(Self::new(
            plain,
            avenger_typst::AvengerTypst::new(avenger_typst::TypstEngineConfig {
                backend: avenger_typst::TypstEngineBackend::VendorTypst,
                ..avenger_typst::TypstEngineConfig::default()
            })?,
            math,
        ))
    }
}

#[cfg(feature = "typst-math")]
impl<P> TextPathExtractor for MathAwareTextPathExtractor<P>
where
    P: TextMeasurer + TextPathExtractor,
{
    fn extract_text_paths(
        &self,
        config: &TextPathExtractionConfig,
    ) -> Result<TextPathBuffer, AvengerTextError> {
        if matches!(self.math.mode, TextMarkupMode::Plain) || config.text.is_empty() {
            return self.plain.extract_text_paths(config);
        }

        let mut math = self.math.clone();
        math.math_style.fill = avenger_typst::Color::rgba(
            config.color[0],
            config.color[1],
            config.color[2],
            config.color[3],
        );
        let options = math_string_options_with_outputs(
            &math,
            config.font_size,
            avenger_typst::MathOutputRequest {
                paths: true,
                raster: None,
                pdf_text_layer: config.include_pdf_text_layer,
            },
        );

        let artifact = match self.typst.typeset_math_string(config.text, &options) {
            Ok(artifact) => artifact,
            Err(err) => {
                return match math.error_policy {
                    MathMarkupErrorPolicy::TreatInvalidMathAsLiteral
                    | MathMarkupErrorPolicy::UseFallbackBounds => {
                        self.plain.extract_text_paths(config)
                    }
                    MathMarkupErrorPolicy::ErrorOnPathExtraction => {
                        Err(AvengerTextError::InternalError(format!(
                            "Typst math path extraction failed: {err}"
                        )))
                    }
                };
            }
        };

        if !artifact
            .runs
            .iter()
            .any(|run| matches!(run, avenger_typst::MathStringRun::Math(_)))
        {
            return self.plain.extract_text_paths(config);
        }

        let Some(layout) =
            layout_math_string_artifact(&self.plain, artifact, &config.to_measurement_config())
        else {
            return self.plain.extract_text_paths(config);
        };

        let mut output = TextPathBuffer::new(layout.bounds.clone());
        for run in layout.runs {
            match run {
                MathAwareLaidOutRun::Plain {
                    text,
                    byte_range,
                    x,
                    y_offset,
                    bounds,
                } => {
                    output.plain_runs.push(PlainTextPathRun {
                        text: text.clone(),
                        byte_range,
                        x,
                        y_offset,
                        bounds,
                    });
                    if config.output_mode == TextPathOutputMode::AllText {
                        let run_config = plain_run_config(config, &text);
                        let mut plain_paths = self.plain.extract_text_paths(&run_config)?;
                        for item in &mut plain_paths.items {
                            translate_path_item(item, x, y_offset);
                        }
                        output.items.extend(plain_paths.items);
                    }
                }
                MathAwareLaidOutRun::Math {
                    byte_range,
                    x,
                    y_offset,
                    artifact,
                    ..
                } => {
                    let Some(paths) = artifact.paths else {
                        return Err(AvengerTextError::InternalError(
                            "Typst math path output was requested but missing".to_string(),
                        ));
                    };
                    for item in paths.items {
                        output.items.push(typst_path_item_to_text_path_item(
                            item,
                            byte_range.clone(),
                            x,
                            y_offset,
                        ));
                    }
                    #[cfg(feature = "typst-math")]
                    if let Some(layer) = artifact.pdf_text {
                        output.math_pdf_layers.push(TextMathPdfLayer {
                            byte_range,
                            x,
                            y_offset,
                            layer,
                            font_resources: artifact.font_resources,
                        });
                    }
                }
            }
        }

        Ok(output)
    }
}

#[cfg(feature = "typst-math")]
fn plain_run_config<'a>(
    config: &'a TextPathExtractionConfig<'a>,
    text: &'a String,
) -> TextPathExtractionConfig<'a> {
    TextPathExtractionConfig {
        text,
        color: config.color,
        font: config.font,
        font_size: config.font_size,
        font_weight: config.font_weight,
        font_style: config.font_style,
        limit: f32::INFINITY,
        output_mode: TextPathOutputMode::AllText,
        include_pdf_text_layer: false,
    }
}

#[cfg(feature = "typst-math")]
fn typst_path_item_to_text_path_item(
    item: avenger_typst::MathPathItem,
    byte_range: Range<usize>,
    x_offset: f32,
    y_offset: f32,
) -> TextPathItem {
    let kind = match item.kind {
        avenger_typst::MathPathKind::GlyphOutline { .. } => TextPathKind::MathGlyph,
        avenger_typst::MathPathKind::MathShape => TextPathKind::MathShape,
    };
    TextPathItem {
        path: math_path_data_to_lyon_path(&item.path, item.transform, x_offset, y_offset),
        fill: item.fill.map(rgba_from_typst_color),
        stroke: item.stroke.map(|stroke| TextPathStroke {
            color: rgba_from_typst_color(stroke.color),
            width: stroke.width,
        }),
        byte_range,
        kind,
    }
}

#[cfg(feature = "typst-math")]
fn math_path_data_to_lyon_path(
    path: &avenger_typst::MathPathData,
    transform: avenger_typst::MathTransform,
    x_offset: f32,
    y_offset: f32,
) -> Path {
    let mut builder = Path::builder();
    for command in &path.commands {
        match *command {
            avenger_typst::MathPathCommand::MoveTo { x, y } => {
                builder.begin(transform_math_point(transform, x, y, x_offset, y_offset));
            }
            avenger_typst::MathPathCommand::LineTo { x, y } => {
                builder.line_to(transform_math_point(transform, x, y, x_offset, y_offset));
            }
            avenger_typst::MathPathCommand::QuadTo { x1, y1, x, y } => {
                builder.quadratic_bezier_to(
                    transform_math_point(transform, x1, y1, x_offset, y_offset),
                    transform_math_point(transform, x, y, x_offset, y_offset),
                );
            }
            avenger_typst::MathPathCommand::CubicTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                builder.cubic_bezier_to(
                    transform_math_point(transform, x1, y1, x_offset, y_offset),
                    transform_math_point(transform, x2, y2, x_offset, y_offset),
                    transform_math_point(transform, x, y, x_offset, y_offset),
                );
            }
            avenger_typst::MathPathCommand::Close => builder.close(),
        }
    }
    builder.build()
}

#[cfg(feature = "typst-math")]
fn transform_math_point(
    transform: avenger_typst::MathTransform,
    x: f32,
    y: f32,
    x_offset: f32,
    y_offset: f32,
) -> lyon_path::math::Point {
    point(
        x_offset + transform.xx * x + transform.xy * y + transform.dx,
        y_offset + transform.yx * x + transform.yy * y + transform.dy,
    )
}

#[cfg(feature = "typst-math")]
fn rgba_from_typst_color(color: avenger_typst::Color) -> [f32; 4] {
    [color.r, color.g, color.b, color.a]
}

#[cfg(feature = "typst-math")]
fn translate_path_item(item: &mut TextPathItem, x: f32, y: f32) {
    item.path = translate_path(&item.path, x, y);
}

#[cfg(feature = "typst-math")]
fn translate_path(path: &Path, x: f32, y: f32) -> Path {
    let mut builder = Path::builder();
    for event in path.iter() {
        match event {
            Event::Begin { at } => {
                builder.begin(point(at.x + x, at.y + y));
            }
            Event::Line { to, .. } => {
                builder.line_to(point(to.x + x, to.y + y));
            }
            Event::Quadratic { ctrl, to, .. } => {
                builder
                    .quadratic_bezier_to(point(ctrl.x + x, ctrl.y + y), point(to.x + x, to.y + y));
            }
            Event::Cubic {
                ctrl1, ctrl2, to, ..
            } => {
                builder.cubic_bezier_to(
                    point(ctrl1.x + x, ctrl1.y + y),
                    point(ctrl2.x + x, ctrl2.y + y),
                    point(to.x + x, to.y + y),
                );
            }
            Event::End { close, .. } => {
                if close {
                    builder.close();
                } else {
                    builder.end(false);
                }
            }
        }
    }
    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::measurement::{FontMetrics, FontMetricsConfig};
    use crate::types::{FontStyle, FontWeight, FontWeightNameSpec};

    fn config(text: &String, output_mode: TextPathOutputMode) -> TextPathExtractionConfig<'_> {
        static COLOR: [f32; 4] = [0.1, 0.2, 0.3, 1.0];
        static FONT: String = String::new();
        static WEIGHT: FontWeight = FontWeight::Name(FontWeightNameSpec::Normal);
        static STYLE: FontStyle = FontStyle::Normal;

        TextPathExtractionConfig {
            text,
            color: &COLOR,
            font: &FONT,
            font_size: 10.0,
            font_weight: &WEIGHT,
            font_style: &STYLE,
            limit: f32::INFINITY,
            output_mode,
            include_pdf_text_layer: true,
        }
    }

    #[cfg(feature = "cosmic-text")]
    #[test]
    fn cosmic_extractor_returns_plain_glyph_paths() {
        let extractor = CosmicTextPathExtractor::with_font_resolution(Default::default());
        let text = "plain".to_string();
        let buffer = extractor
            .extract_text_paths(&config(&text, TextPathOutputMode::AllText))
            .unwrap();

        assert!(buffer.bounds.width > 0.0);
        assert!(!buffer.items.is_empty());
        assert_eq!(buffer.plain_runs.len(), 1);
        assert!(buffer
            .items
            .iter()
            .all(|item| item.kind == TextPathKind::PlainGlyph));
    }

    #[cfg(feature = "typst-math")]
    #[derive(Debug, Clone)]
    struct FixedPlainExtractor;

    #[cfg(feature = "typst-math")]
    impl TextMeasurer for FixedPlainExtractor {
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

    #[cfg(feature = "typst-math")]
    impl TextPathExtractor for FixedPlainExtractor {
        fn extract_text_paths(
            &self,
            config: &TextPathExtractionConfig,
        ) -> Result<TextPathBuffer, AvengerTextError> {
            let bounds = self.measure_text_bounds(&config.to_measurement_config());
            let mut buffer = TextPathBuffer::new(bounds.clone());
            buffer.plain_runs.push(PlainTextPathRun {
                text: config.text.to_string(),
                byte_range: 0..config.text.len(),
                x: 0.0,
                y_offset: 0.0,
                bounds,
            });
            let mut builder = Path::builder();
            builder.begin(point(0.0, 0.0));
            builder.line_to(point(1.0, 0.0));
            builder.line_to(point(1.0, 1.0));
            builder.close();
            buffer.items.push(TextPathItem {
                path: builder.build(),
                fill: Some(*config.color),
                stroke: None,
                byte_range: 0..config.text.len(),
                kind: TextPathKind::PlainGlyph,
            });
            Ok(buffer)
        }
    }

    #[cfg(feature = "typst-math")]
    fn math_config() -> TextMathConfig {
        TextMathConfig {
            mode: TextMarkupMode::TypstMathDelimited(Default::default()),
            ..Default::default()
        }
    }

    #[cfg(feature = "typst-math")]
    #[test]
    fn math_aware_extractor_returns_math_paths_and_plain_runs() {
        let typst = avenger_typst::AvengerTypst::new(Default::default()).unwrap();
        let extractor = MathAwareTextPathExtractor::new(FixedPlainExtractor, typst, math_config());
        let text = "speed $v^2$".to_string();
        let buffer = extractor
            .extract_text_paths(&config(&text, TextPathOutputMode::MathOnly))
            .unwrap();

        assert_eq!(buffer.plain_runs.len(), 1);
        assert_eq!(buffer.items.len(), 1);
        assert_eq!(buffer.items[0].kind, TextPathKind::MathGlyph);
        assert_eq!(buffer.math_pdf_layers.len(), 1);
    }

    #[cfg(feature = "typst-math")]
    #[test]
    fn math_aware_extractor_can_include_plain_paths() {
        let typst = avenger_typst::AvengerTypst::new(Default::default()).unwrap();
        let extractor = MathAwareTextPathExtractor::new(FixedPlainExtractor, typst, math_config());
        let text = "speed $v^2$".to_string();
        let buffer = extractor
            .extract_text_paths(&config(&text, TextPathOutputMode::AllText))
            .unwrap();

        assert!(buffer
            .items
            .iter()
            .any(|item| item.kind == TextPathKind::PlainGlyph));
        assert!(buffer
            .items
            .iter()
            .any(|item| item.kind == TextPathKind::MathGlyph));
    }
}
