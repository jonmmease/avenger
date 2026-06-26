use std::{collections::HashMap, sync::OnceLock};

use crate::{
    error::AvengerTextError,
    math::TextMathConfig,
    measurement::{FontMetrics, FontMetricsConfig, TextBounds, TextMeasurementConfig},
    path::{TextPathBuffer, TextPathExtractionConfig, TextPathExtractor, TypstTextPathExtractor},
    rasterization::{TextRasterizationBuffer, TextRasterizationConfig},
    typst_text::{TypstTextMeasurer, TypstTextRasterCacheKey, TypstTextRasterizer},
};

#[derive(Debug, Clone)]
pub struct TextEngine {
    typst: avenger_typst::AvengerTypst,
    math: TextMathConfig,
}

impl TextEngine {
    pub fn new(typst: avenger_typst::AvengerTypst, math: TextMathConfig) -> Self {
        Self { typst, math }
    }

    pub fn with_config(math: TextMathConfig) -> Result<Self, avenger_typst::TypstInitError> {
        Ok(Self::new(
            avenger_typst::AvengerTypst::new(avenger_typst::TypstEngineConfig {
                backend: avenger_typst::TypstEngineBackend::OwnedTypst,
                ..avenger_typst::TypstEngineConfig::default()
            })?,
            math,
        ))
    }

    pub fn with_default_config() -> Result<Self, avenger_typst::TypstInitError> {
        Self::with_config(TextMathConfig::default())
    }

    pub fn measure_bounds(&self, config: &TextMeasurementConfig) -> TextBounds {
        TypstTextMeasurer::new(self.typst.clone(), self.math.clone()).measure_text_bounds(config)
    }

    pub fn font_metrics(&self, config: &FontMetricsConfig) -> FontMetrics {
        TypstTextMeasurer::new(self.typst.clone(), self.math.clone()).measure_font_metrics(config)
    }

    pub fn rasterize<CacheValue>(
        &self,
        config: &TextRasterizationConfig,
        scale: f32,
        cached_entries: &HashMap<TypstTextRasterCacheKey, CacheValue>,
    ) -> Result<TextRasterizationBuffer<TypstTextRasterCacheKey>, AvengerTextError>
    where
        CacheValue: Clone,
    {
        TypstTextRasterizer::<CacheValue>::new(self.typst.clone(), self.math.clone()).rasterize(
            config,
            scale,
            cached_entries,
        )
    }

    pub fn extract_paths(
        &self,
        config: &TextPathExtractionConfig,
    ) -> Result<TextPathBuffer, AvengerTextError> {
        TypstTextPathExtractor::new(self.typst.clone(), self.math.clone())
            .extract_text_paths(config)
    }
}

pub fn default_text_engine() -> TextEngine {
    static DEFAULT_TEXT_ENGINE: OnceLock<TextEngine> = OnceLock::new();
    DEFAULT_TEXT_ENGINE
        .get_or_init(|| {
            TextEngine::with_default_config().expect("failed to initialize Typst text engine")
        })
        .clone()
}
