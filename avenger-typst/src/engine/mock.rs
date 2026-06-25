use std::sync::Arc;

use crate::error::MathTypesetError;
use crate::paths::{MathPathArtifact, MathPathData, MathPathItem, MathPathKind, MathTransform};
use crate::pdf::{
    MathFontResource, MathFontResourceId, MathPdfGlyph, MathPdfGlyphRun, MathPdfTextLayer,
};
use crate::raster::{MathRasterArtifact, RgbaImageData};
use crate::style::{Color, MathFontSpec};
use crate::types::{MathFragmentOptions, MathRunArtifact, TypesetMetrics};
use crate::warnings::MathTypesetWarning;

#[derive(Debug, Clone, Default)]
pub(crate) struct MockMathEngine;

impl MockMathEngine {
    pub(crate) fn typeset_fragment(
        &self,
        source: &str,
        options: &MathFragmentOptions,
    ) -> Result<MathRunArtifact, MathTypesetError> {
        let char_count = source.chars().count().max(1) as f32;
        let font_size = options.style.font_size.max(1.0);
        let width = char_count * font_size * 0.6;
        let height = font_size * 1.2;
        let metrics = TypesetMetrics {
            width,
            height,
            baseline: font_size * 0.8,
            ascent: font_size * 0.8,
            descent: font_size * 0.4,
        };

        let font_resource = mock_font_resource(&options.style.font);
        let font_resources = if options.outputs.pdf_text_layer {
            vec![font_resource.clone()]
        } else {
            Vec::new()
        };

        let pdf_text = if options.outputs.pdf_text_layer {
            Some(mock_pdf_text_layer(
                source,
                metrics,
                options.style.font_size,
                options.style.fill,
                font_resource.id,
            ))
        } else {
            None
        };

        let paths = if options.outputs.paths {
            Some(mock_path_artifact(metrics, options.style.fill))
        } else {
            None
        };

        let raster = options.outputs.raster.map(|request| MathRasterArtifact {
            image: RgbaImageData {
                width: 1,
                height: 1,
                data: vec![0, 0, 0, 0],
            },
            scale: request.scale,
            logical_width: width,
            logical_height: height,
        });

        Ok(MathRunArtifact {
            metrics,
            paths,
            raster,
            pdf_text,
            font_resources,
            warnings: vec![MathTypesetWarning::MockBackend],
        })
    }
}

fn mock_path_artifact(metrics: TypesetMetrics, fill: Color) -> MathPathArtifact {
    MathPathArtifact {
        logical_width: metrics.width,
        logical_height: metrics.height,
        items: vec![MathPathItem {
            path: MathPathData::rect(metrics.width, metrics.height),
            kind: MathPathKind::GlyphOutline {
                glyph_run: 0,
                glyph_index: 0,
            },
            fill: Some(fill),
            stroke: None,
            transform: MathTransform::IDENTITY,
            clip: None,
        }],
    }
}

fn mock_pdf_text_layer(
    source: &str,
    metrics: TypesetMetrics,
    font_size: f32,
    fill: Color,
    font: MathFontResourceId,
) -> MathPdfTextLayer {
    let glyphs = source
        .chars()
        .enumerate()
        .map(|(index, ch)| MathPdfGlyph {
            glyph_id: ch as u32 as u16,
            unicode: ch.to_string(),
            x: index as f32 * font_size * 0.6,
            y: 0.0,
            x_advance: font_size * 0.6,
            y_advance: 0.0,
            transform: MathTransform::IDENTITY,
        })
        .collect();

    MathPdfTextLayer {
        logical_width: metrics.width,
        logical_height: metrics.height,
        semantic_text: source.to_string(),
        glyph_runs: vec![MathPdfGlyphRun {
            font,
            font_size,
            fill,
            stroke: None,
            glyphs,
        }],
    }
}

fn mock_font_resource(font: &MathFontSpec) -> MathFontResource {
    let family = match font {
        MathFontSpec::NewComputerModernMath => "New Computer Modern Math".to_string(),
        MathFontSpec::Family(family) => family.clone(),
        MathFontSpec::FontBytes(id) => format!("font-bytes-{}", id.0),
    };

    MathFontResource {
        id: MathFontResourceId(0),
        family,
        postscript_name: Some("AvengerTypstMock".to_string()),
        face_index: 0,
        units_per_em: 1000.0,
        data: Arc::<[u8]>::from([0u8; 4]),
    }
}
