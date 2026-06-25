use std::ops::Range;

use lyon_path::Path;

use crate::{
    error::AvengerTextError,
    measurement::{TextBounds, TextMeasurementConfig},
    types::{FontStyle, FontWeight},
};

#[cfg(feature = "typst-math")]
use lyon_path::geom::point;

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

#[cfg(feature = "typst-text")]
use crate::math::TextMathConfig;

#[cfg(feature = "typst-text")]
use crate::typst_text::{
    bounds_from_metrics, tight_bounds_from_metrics, typeset_line, TypstTextMeasurer,
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
pub struct TextPdfLayer {
    pub byte_range: Range<usize>,
    pub x: f32,
    pub y_offset: f32,
    pub bounds: TextBounds,
    pub layer: avenger_typst::MathPdfTextLayer,
    pub font_resources: Vec<avenger_typst::MathFontResource>,
}

#[cfg(feature = "typst-math")]
#[deprecated(note = "use TextPdfLayer")]
pub type TextMathPdfLayer = TextPdfLayer;

#[derive(Debug, Clone)]
pub struct TextPathBuffer {
    pub bounds: TextBounds,
    pub items: Vec<TextPathItem>,
    pub plain_runs: Vec<PlainTextPathRun>,
    #[cfg(feature = "typst-math")]
    pub pdf_layers: Vec<TextPdfLayer>,
}

impl TextPathBuffer {
    pub fn new(bounds: TextBounds) -> Self {
        Self {
            bounds,
            items: Vec::new(),
            plain_runs: Vec::new(),
            #[cfg(feature = "typst-math")]
            pdf_layers: Vec::new(),
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

#[cfg(feature = "typst-text")]
#[derive(Debug, Clone)]
pub struct TypstTextPathExtractor {
    typst: avenger_typst::AvengerTypst,
    math: TextMathConfig,
}

#[cfg(feature = "typst-text")]
impl TypstTextPathExtractor {
    pub fn new(typst: avenger_typst::AvengerTypst, math: TextMathConfig) -> Self {
        Self { typst, math }
    }

    pub fn with_vendor_typst(math: TextMathConfig) -> Result<Self, avenger_typst::TypstInitError> {
        Ok(Self::new(
            avenger_typst::AvengerTypst::new(avenger_typst::TypstEngineConfig {
                backend: avenger_typst::TypstEngineBackend::VendorTypst,
                ..avenger_typst::TypstEngineConfig::default()
            })?,
            math,
        ))
    }
}

#[cfg(feature = "typst-text")]
impl TextMeasurer for TypstTextPathExtractor {
    fn measure_text_bounds(&self, config: &TextMeasurementConfig) -> TextBounds {
        TypstTextMeasurer::new(self.typst.clone(), self.math.clone()).measure_text_bounds(config)
    }

    fn measure_font_metrics(
        &self,
        config: &crate::measurement::FontMetricsConfig,
    ) -> crate::measurement::FontMetrics {
        TypstTextMeasurer::new(self.typst.clone(), self.math.clone()).measure_font_metrics(config)
    }
}

#[cfg(feature = "typst-text")]
impl TextPathExtractor for TypstTextPathExtractor {
    fn extract_text_paths(
        &self,
        config: &TextPathExtractionConfig,
    ) -> Result<TextPathBuffer, AvengerTextError> {
        let text = crate::measurement::try_truncate_text_to_limit_with(
            config.text,
            config.limit,
            |candidate| {
                let measurement = TextMeasurementConfig {
                    text: candidate,
                    font: config.font,
                    font_size: config.font_size,
                    font_weight: config.font_weight,
                    font_style: config.font_style,
                };
                Ok::<f32, AvengerTextError>(self.measure_text_bounds(&measurement).width)
            },
        )?;
        let result = typeset_line(
            &self.typst,
            &self.math,
            &text,
            config.font,
            config.font_size,
            config.font_weight,
            config.font_style,
            *config.color,
            avenger_typst::TextLineOutputRequest {
                paths: true,
                raster: None,
                pdf_text_layer: config.include_pdf_text_layer,
            },
        )
        .map_err(|err| {
            AvengerTextError::InternalError(format!("Typst text path extraction failed: {err}"))
        })?;
        let tight_bounds = tight_bounds_from_metrics(result.artifact.metrics);
        let bounds = bounds_from_metrics(
            result.artifact.metrics,
            config.font_size,
            result.has_math_spans,
        );
        let y_offset = bounds.ascent - tight_bounds.ascent;
        let mut output = TextPathBuffer::new(bounds.clone());

        let paths = result.artifact.paths.ok_or_else(|| {
            AvengerTextError::InternalError(
                "Typst text path output was requested but missing".to_string(),
            )
        })?;
        let byte_range = 0..text.len();
        for item in paths.items {
            output.items.push(typst_path_item_to_text_path_item(
                item,
                byte_range.clone(),
                0.0,
                y_offset,
            ));
        }

        if let Some(layer) = result.artifact.pdf_text {
            output.pdf_layers.push(TextPdfLayer {
                byte_range,
                x: 0.0,
                y_offset,
                bounds,
                layer,
                font_resources: result.artifact.font_resources,
            });
        }

        Ok(output)
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

#[cfg(test)]
mod tests {
    use super::*;
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

    #[cfg(feature = "typst-text")]
    fn math_config() -> crate::math::TextMathConfig {
        crate::math::TextMathConfig {
            mode: crate::math::TextMarkupMode::TypstMathDelimited(Default::default()),
            ..Default::default()
        }
    }

    #[cfg(feature = "typst-text")]
    #[test]
    fn typst_text_extractor_returns_whole_line_paths_and_pdf_layer() {
        let typst = avenger_typst::AvengerTypst::new(Default::default()).unwrap();
        let extractor = TypstTextPathExtractor::new(typst, math_config());
        let text = "speed $v^2$".to_string();
        let buffer = extractor
            .extract_text_paths(&config(&text, TextPathOutputMode::AllText))
            .unwrap();

        assert!(buffer.plain_runs.is_empty());
        assert_eq!(buffer.items.len(), 1);
        assert_eq!(buffer.items[0].kind, TextPathKind::MathGlyph);
        assert_eq!(buffer.pdf_layers.len(), 1);
        assert_eq!(buffer.pdf_layers[0].layer.semantic_text, text);
    }
}
