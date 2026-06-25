use std::{collections::HashMap, hash::Hash, marker::PhantomData};

use ordered_float::OrderedFloat;

use crate::{
    error::AvengerTextError,
    math::{
        layout_math_string_artifact, math_string_options_with_outputs, MathAwareLaidOutRun,
        TextMarkupMode, TextMathConfig,
    },
    measurement::TextMeasurer,
    rasterization::{
        GlyphBBox, GlyphData, GlyphPosition, TextRasterizationBuffer, TextRasterizationConfig,
        TextRasterizer,
    },
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MathAwareRasterCacheKey<PlainKey> {
    Plain(PlainKey),
    Math(TypstMathRasterCacheKey),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypstMathRasterCacheKey {
    pub source: String,
    pub style: String,
    pub fill: [u8; 4],
    pub scale: OrderedFloat<f32>,
    pub syntax: String,
}

#[derive(Debug, Clone)]
pub struct MathAwareTextRasterizer<P, CacheValue> {
    plain: P,
    typst: avenger_typst::AvengerTypst,
    math: TextMathConfig,
    _cache_value: PhantomData<CacheValue>,
}

impl<P, CacheValue> MathAwareTextRasterizer<P, CacheValue> {
    pub fn new(plain: P, typst: avenger_typst::AvengerTypst, math: TextMathConfig) -> Self {
        Self {
            plain,
            typst,
            math,
            _cache_value: PhantomData,
        }
    }

    pub fn plain(&self) -> &P {
        &self.plain
    }

    pub fn math_config(&self) -> &TextMathConfig {
        &self.math
    }
}

impl<P, CacheValue> TextRasterizer for MathAwareTextRasterizer<P, CacheValue>
where
    P: TextMeasurer + TextRasterizer<CacheValue = CacheValue>,
    P::CacheKey: Hash + Eq + Clone + 'static,
    CacheValue: Clone + 'static,
{
    type CacheKey = MathAwareRasterCacheKey<P::CacheKey>;
    type CacheValue = CacheValue;

    fn rasterize(
        &self,
        config: &TextRasterizationConfig,
        scale: f32,
        cached_glyphs: &HashMap<Self::CacheKey, Self::CacheValue>,
    ) -> Result<TextRasterizationBuffer<Self::CacheKey>, AvengerTextError> {
        if matches!(self.math.mode, TextMarkupMode::Plain) || config.text.is_empty() {
            return self.rasterize_plain_text(config, scale, cached_glyphs);
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
                paths: false,
                raster: Some(avenger_typst::RasterRequest { scale }),
                pdf_text_layer: false,
            },
        );

        let artifact = match self.typst.typeset_math_string(config.text, &options) {
            Ok(artifact) => artifact,
            Err(_) => return self.rasterize_plain_text(config, scale, cached_glyphs),
        };

        if !artifact
            .runs
            .iter()
            .any(|run| matches!(run, avenger_typst::MathStringRun::Math(_)))
        {
            return self.rasterize_plain_text(config, scale, cached_glyphs);
        }

        let Some(layout) =
            layout_math_string_artifact(&self.plain, artifact, &config.to_measurement_config())
        else {
            return self.rasterize_plain_text(config, scale, cached_glyphs);
        };

        let plain_cached = plain_cached_glyphs(cached_glyphs);
        let fill = color_key(config.color);
        let mut glyphs = Vec::new();

        for run in layout.runs {
            match run {
                MathAwareLaidOutRun::Plain { text, x, .. } => {
                    let run_buffer = self.plain.rasterize(
                        &plain_run_config(config, &text),
                        scale,
                        &plain_cached,
                    )?;
                    append_plain_glyphs(&mut glyphs, run_buffer, x, scale);
                }
                MathAwareLaidOutRun::Math {
                    source,
                    x,
                    y_offset,
                    artifact,
                    ..
                } => {
                    let Some(raster) = artifact.raster else {
                        return Err(AvengerTextError::InternalError(
                            "Typst math raster output was requested but missing".to_string(),
                        ));
                    };
                    let cache_key = MathAwareRasterCacheKey::Math(TypstMathRasterCacheKey {
                        source,
                        style: format!("{:?}", math.math_style),
                        fill,
                        scale: OrderedFloat(scale),
                        syntax: format!("{:?}", math.syntax),
                    });
                    let image = if cached_glyphs.contains_key(&cache_key) {
                        None
                    } else {
                        Some(
                            image::RgbaImage::from_vec(
                                raster.image.width,
                                raster.image.height,
                                raster.image.data,
                            )
                            .ok_or_else(|| {
                                AvengerTextError::ImageAllocationError(
                                    "Typst math raster image dimensions did not match data"
                                        .to_string(),
                                )
                            })?,
                        )
                    };
                    let glyph_position = GlyphPosition {
                        x: x + raster.origin_x,
                        y: y_offset + raster.origin_y - layout.bounds.ascent,
                        physical_x: ((x + raster.origin_x) * scale).round(),
                        physical_y: ((y_offset + raster.origin_y - layout.bounds.ascent) * scale)
                            .round(),
                    };
                    glyphs.push((
                        GlyphData {
                            cache_key,
                            image,
                            path: None,
                            bbox: GlyphBBox {
                                top: 0,
                                left: 0,
                                width: raster.image.width,
                                height: raster.image.height,
                            },
                        },
                        glyph_position,
                    ));
                }
            }
        }

        Ok(TextRasterizationBuffer {
            glyphs,
            text_bounds: layout.bounds,
        })
    }
}

impl<P, CacheValue> MathAwareTextRasterizer<P, CacheValue>
where
    P: TextMeasurer + TextRasterizer<CacheValue = CacheValue>,
    P::CacheKey: Hash + Eq + Clone + 'static,
    CacheValue: Clone + 'static,
{
    fn rasterize_plain_text(
        &self,
        config: &TextRasterizationConfig,
        scale: f32,
        cached_glyphs: &HashMap<MathAwareRasterCacheKey<P::CacheKey>, CacheValue>,
    ) -> Result<TextRasterizationBuffer<MathAwareRasterCacheKey<P::CacheKey>>, AvengerTextError>
    {
        let plain_cached = plain_cached_glyphs(cached_glyphs);
        let buffer = self.plain.rasterize(config, scale, &plain_cached)?;
        let text_bounds = buffer.text_bounds.clone();
        let mut glyphs = Vec::new();
        append_plain_glyphs(&mut glyphs, buffer, 0.0, scale);
        Ok(TextRasterizationBuffer {
            glyphs,
            text_bounds,
        })
    }
}

fn plain_cached_glyphs<PlainKey, CacheValue>(
    cached_glyphs: &HashMap<MathAwareRasterCacheKey<PlainKey>, CacheValue>,
) -> HashMap<PlainKey, CacheValue>
where
    PlainKey: Hash + Eq + Clone,
    CacheValue: Clone,
{
    cached_glyphs
        .iter()
        .filter_map(|(key, value)| match key {
            MathAwareRasterCacheKey::Plain(plain) => Some((plain.clone(), value.clone())),
            MathAwareRasterCacheKey::Math(_) => None,
        })
        .collect()
}

fn append_plain_glyphs<PlainKey>(
    glyphs: &mut Vec<(GlyphData<MathAwareRasterCacheKey<PlainKey>>, GlyphPosition)>,
    buffer: TextRasterizationBuffer<PlainKey>,
    x_offset: f32,
    scale: f32,
) where
    PlainKey: Hash + Eq + Clone,
{
    for (glyph, mut position) in buffer.glyphs {
        position.x += x_offset;
        position.physical_x += (x_offset * scale).round();
        glyphs.push((
            GlyphData {
                cache_key: MathAwareRasterCacheKey::Plain(glyph.cache_key),
                image: glyph.image,
                path: glyph.path,
                bbox: glyph.bbox,
            },
            position,
        ));
    }
}

fn plain_run_config<'a>(
    config: &'a TextRasterizationConfig<'_>,
    text: &'a String,
) -> TextRasterizationConfig<'a> {
    TextRasterizationConfig {
        text,
        color: config.color,
        font: config.font,
        font_size: config.font_size,
        font_weight: config.font_weight,
        font_style: config.font_style,
        limit: f32::INFINITY,
    }
}

fn color_key(color: &[f32; 4]) -> [u8; 4] {
    [
        channel(color[0]),
        channel(color[1]),
        channel(color[2]),
        channel(color[3]),
    ]
}

fn channel(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        measurement::{FontMetrics, TextBounds},
        types::{FontStyle, FontWeight, FontWeightNameSpec},
    };

    #[derive(Debug, Clone)]
    struct FixedRasterizer;

    impl TextRasterizer for FixedRasterizer {
        type CacheKey = String;
        type CacheValue = ();

        fn rasterize(
            &self,
            config: &TextRasterizationConfig,
            _scale: f32,
            cached_glyphs: &HashMap<Self::CacheKey, Self::CacheValue>,
        ) -> Result<TextRasterizationBuffer<Self::CacheKey>, AvengerTextError> {
            let width = config.text.chars().count() as f32 * 10.0;
            let cache_key = config.text.clone();
            let image = if cached_glyphs.contains_key(&cache_key) {
                None
            } else {
                Some(image::RgbaImage::from_pixel(
                    1,
                    1,
                    image::Rgba([0, 0, 0, 255]),
                ))
            };

            Ok(TextRasterizationBuffer {
                text_bounds: TextBounds {
                    width,
                    height: 10.0,
                    ascent: 7.0,
                    descent: 3.0,
                    line_height: 12.0,
                },
                glyphs: vec![(
                    GlyphData {
                        cache_key,
                        image,
                        path: None,
                        bbox: GlyphBBox {
                            top: 0,
                            left: 0,
                            width: 1,
                            height: 1,
                        },
                    },
                    GlyphPosition {
                        x: 0.0,
                        y: -7.0,
                        physical_x: 0.0,
                        physical_y: -7.0,
                    },
                )],
            })
        }
    }

    impl crate::measurement::TextMeasurer for FixedRasterizer {
        fn measure_text_bounds(
            &self,
            config: &crate::measurement::TextMeasurementConfig,
        ) -> TextBounds {
            TextBounds {
                width: config.text.chars().count() as f32 * 10.0,
                height: 10.0,
                ascent: 7.0,
                descent: 3.0,
                line_height: 12.0,
            }
        }

        fn measure_font_metrics(
            &self,
            config: &crate::measurement::FontMetricsConfig,
        ) -> FontMetrics {
            FontMetrics::fallback(config.font_size)
        }
    }

    fn rasterizer() -> MathAwareTextRasterizer<FixedRasterizer, ()> {
        MathAwareTextRasterizer::new(
            FixedRasterizer,
            avenger_typst::AvengerTypst::new(Default::default()).unwrap(),
            TextMathConfig {
                mode: TextMarkupMode::TypstMathDelimited(Default::default()),
                ..Default::default()
            },
        )
    }

    fn config(text: &String) -> TextRasterizationConfig<'_> {
        static COLOR: [f32; 4] = [0.1, 0.2, 0.3, 1.0];
        static FONT: String = String::new();
        static WEIGHT: FontWeight = FontWeight::Name(FontWeightNameSpec::Normal);
        static STYLE: FontStyle = FontStyle::Normal;

        TextRasterizationConfig {
            text,
            color: &COLOR,
            font: &FONT,
            font_size: 10.0,
            font_weight: &WEIGHT,
            font_style: &STYLE,
            limit: f32::INFINITY,
        }
    }

    #[test]
    fn plain_labels_use_plain_cache_key_variant() {
        let text = "plain".to_string();
        let buffer = rasterizer()
            .rasterize(&config(&text), 1.0, &HashMap::new())
            .unwrap();

        assert_eq!(buffer.glyphs.len(), 1);
        assert!(matches!(
            &buffer.glyphs[0].0.cache_key,
            MathAwareRasterCacheKey::Plain(key) if key == "plain"
        ));
    }

    #[test]
    fn mixed_labels_include_math_pseudo_glyph() {
        let text = "speed $v^2$".to_string();
        let buffer = rasterizer()
            .rasterize(&config(&text), 2.0, &HashMap::new())
            .unwrap();

        assert_eq!(buffer.glyphs.len(), 2);
        assert!(matches!(
            &buffer.glyphs[0].0.cache_key,
            MathAwareRasterCacheKey::Plain(key) if key == "speed "
        ));
        assert!(matches!(
            &buffer.glyphs[1].0.cache_key,
            MathAwareRasterCacheKey::Math(key) if key.source == "v^2"
        ));
        assert_eq!(buffer.glyphs[1].0.bbox.width, 1);
        assert_eq!(buffer.glyphs[1].0.bbox.height, 1);
    }

    #[test]
    fn cached_math_pseudo_glyph_omits_image() {
        let text = "$x$".to_string();
        let first = rasterizer()
            .rasterize(&config(&text), 1.0, &HashMap::new())
            .unwrap();
        let cache_key = first.glyphs[0].0.cache_key.clone();
        let cached = HashMap::from([(cache_key, ())]);

        let second = rasterizer()
            .rasterize(&config(&text), 1.0, &cached)
            .unwrap();

        assert!(second.glyphs[0].0.image.is_none());
    }
}
