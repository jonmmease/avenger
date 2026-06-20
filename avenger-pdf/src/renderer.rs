use std::path::Path;

use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_svg::{SvgBackground, SvgFontEmbedding, SvgImageMode, SvgRenderOptions, SvgRenderer};

use crate::{
    error::AvengerPdfError,
    options::{PdfBackground, PdfRenderOptions},
};

#[derive(Debug, Clone, Default)]
pub struct PdfRenderer {
    options: PdfRenderOptions,
}

impl PdfRenderer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_options(mut self, options: PdfRenderOptions) -> Self {
        self.options = options;
        self
    }

    pub fn render_scene_graph(&self, scene_graph: &SceneGraph) -> Result<Vec<u8>, AvengerPdfError> {
        let svg = self.render_svg_for_pdf(scene_graph)?;
        let usvg_options = self.usvg_options();
        let tree = svg2pdf::usvg::Tree::from_str(&svg, &usvg_options)?;
        let pdf = svg2pdf::to_pdf(
            &tree,
            svg2pdf::ConversionOptions {
                compress: self.options.compress,
                raster_scale: self.options.raster_scale,
                embed_text: self.options.embed_text,
                pdfa: false,
            },
            svg2pdf::PageOptions::default(),
        )
        .map_err(|err| AvengerPdfError::Conversion(err.to_string()))?;
        Ok(pdf)
    }

    pub fn write_scene_graph_pdf<P: AsRef<Path>>(
        &self,
        scene_graph: &SceneGraph,
        output: P,
    ) -> Result<(), AvengerPdfError> {
        let output = output.as_ref();
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(output, self.render_scene_graph(scene_graph)?)?;
        Ok(())
    }

    fn render_svg_for_pdf(&self, scene_graph: &SceneGraph) -> Result<String, AvengerPdfError> {
        Ok(SvgRenderer::new()
            .with_options(self.svg_options())
            .render_scene_graph(scene_graph)?)
    }

    fn svg_options(&self) -> SvgRenderOptions {
        SvgRenderOptions {
            background: match self.options.background {
                PdfBackground::White => SvgBackground::White,
                PdfBackground::Transparent => SvgBackground::Transparent,
                PdfBackground::Color(color) => SvgBackground::Color(color),
            },
            precision: self.options.precision,
            image_mode: SvgImageMode::EmbedPngDataUris,
            font_resolution: self.options.font_resolution.clone(),
            font_embedding: SvgFontEmbedding::None,
            include_metadata: false,
        }
    }

    fn usvg_options(&self) -> svg2pdf::usvg::Options<'static> {
        let mut options = svg2pdf::usvg::Options::default();
        load_avenger_embedded_fonts(options.fontdb_mut());

        if self.options.font_resolution.load_system_fonts {
            options.fontdb_mut().load_system_fonts();
        }

        for font_dir in &self.options.font_resolution.extra_font_dirs {
            options.fontdb_mut().load_fonts_dir(font_dir);
        }

        options
    }
}

fn load_avenger_embedded_fonts(fontdb: &mut svg2pdf::usvg::fontdb::Database) {
    for font in avenger_text::fonts::embedded_fonts() {
        fontdb.load_font_data(Vec::from(font.data));
    }
}

#[cfg(test)]
mod tests {
    use avenger_color::{ColorOrGradient, Gradient, GradientStop, RadialGradient};
    use avenger_common::value::ScalarOrArray;
    use avenger_scenegraph::{
        marks::{rect::SceneRectMark, text::SceneTextMark},
        scene_graph::SceneGraph,
    };

    use super::*;

    #[test]
    fn renders_scenegraph_to_pdf_bytes() {
        let scene_graph = SceneGraph {
            width: 20.0,
            height: 10.0,
            origin: [0.0, 0.0],
            marks: vec![SceneRectMark {
                len: 1,
                x: ScalarOrArray::new_scalar(1.0),
                y: ScalarOrArray::new_scalar(2.0),
                width: Some(ScalarOrArray::new_scalar(3.0)),
                height: Some(ScalarOrArray::new_scalar(4.0)),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([1.0, 0.0, 0.0, 1.0])),
                ..Default::default()
            }
            .into()],
        };

        let pdf = PdfRenderer::new().render_scene_graph(&scene_graph).unwrap();

        assert!(pdf.starts_with(b"%PDF-"));
    }

    #[test]
    fn internal_svg_preserves_native_text_for_pdf_conversion() {
        let scene_graph = SceneGraph {
            width: 80.0,
            height: 20.0,
            origin: [0.0, 0.0],
            marks: vec![SceneTextMark {
                text: ScalarOrArray::new_scalar("Selectable".to_string()),
                x: ScalarOrArray::new_scalar(5.0),
                y: ScalarOrArray::new_scalar(12.0),
                font: ScalarOrArray::new_scalar("Atkinson Hyperlegible".to_string()),
                font_size: ScalarOrArray::new_scalar(12.0),
                ..Default::default()
            }
            .into()],
        };

        let renderer = PdfRenderer::new();
        let svg = renderer.render_svg_for_pdf(&scene_graph).unwrap();
        let tree = svg2pdf::usvg::Tree::from_str(&svg, &renderer.usvg_options()).unwrap();
        let pdf = renderer.render_scene_graph(&scene_graph).unwrap();

        assert!(svg.contains("<text "));
        assert!(tree.has_text_nodes());
        assert!(pdf.starts_with(b"%PDF-"));
    }

    #[test]
    fn radial_gradient_pattern_svg_converts_to_pdf() {
        let scene_graph = SceneGraph {
            width: 40.0,
            height: 20.0,
            origin: [0.0, 0.0],
            marks: vec![SceneRectMark {
                len: 1,
                gradients: vec![Gradient::RadialGradient(RadialGradient {
                    x0: 0.5,
                    y0: 0.5,
                    x1: 0.5,
                    y1: 0.5,
                    r0: 0.0,
                    r1: 0.5,
                    stops: vec![
                        GradientStop {
                            offset: 0.0,
                            color: [1.0, 0.0, 0.0, 1.0],
                        },
                        GradientStop {
                            offset: 1.0,
                            color: [0.0, 0.0, 1.0, 1.0],
                        },
                    ],
                })],
                x: ScalarOrArray::new_scalar(2.0),
                y: ScalarOrArray::new_scalar(3.0),
                width: Some(ScalarOrArray::new_scalar(30.0)),
                height: Some(ScalarOrArray::new_scalar(10.0)),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::GradientIndex(0)),
                ..Default::default()
            }
            .into()],
        };

        let renderer = PdfRenderer::new();
        let svg = renderer.render_svg_for_pdf(&scene_graph).unwrap();
        let tree = svg2pdf::usvg::Tree::from_str(&svg, &renderer.usvg_options()).unwrap();
        let pdf = renderer.render_scene_graph(&scene_graph).unwrap();

        assert!(svg.contains("<pattern "));
        assert!(svg.contains("<radialGradient "));
        assert!(svg.contains(r#"fill="url(#svg-gradient-0)""#));
        assert!(tree.size().width() > 0.0);
        assert!(pdf.starts_with(b"%PDF-"));
    }
}
