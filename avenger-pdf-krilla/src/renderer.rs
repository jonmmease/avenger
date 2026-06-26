use std::path::Path;

use avenger_scenegraph::scene_graph::SceneGraph;
use krilla::{
    color::rgb,
    geom::{PathBuilder, Rect},
    num::NormalizedF32,
    page::PageSettings,
    paint::{Fill, FillRule},
    surface::Surface,
    Document, SerializeSettings,
};

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
        let width = scene_graph.width.max(3.0);
        let height = scene_graph.height.max(3.0);
        let page_settings = PageSettings::from_wh(width, height)
            .ok_or(AvengerPdfError::InvalidPageSize { width, height })?;

        let mut settings = SerializeSettings::default();
        settings.compress_content_streams = self.options.compress;
        settings.render_svg_glyph_fn = krilla_svg::render_svg_glyph;

        let mut document = Document::new_with(settings);
        let mut page = document.start_page_with(page_settings);
        let mut surface = page.surface();
        self.draw_background(&mut surface, width, height)?;
        surface.finish();
        page.finish();

        Ok(document.finish()?)
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

    fn draw_background(
        &self,
        surface: &mut Surface<'_>,
        width: f32,
        height: f32,
    ) -> Result<(), AvengerPdfError> {
        let color = match self.options.background {
            PdfBackground::White => Some([1.0, 1.0, 1.0, 1.0]),
            PdfBackground::Transparent => None,
            PdfBackground::Color(color) => Some(color),
        };
        let Some(color) = color else {
            return Ok(());
        };
        let [r, g, b, a] = color.map(|value| value.clamp(0.0, 1.0));
        if a <= 0.0 {
            return Ok(());
        }

        let rect = Rect::from_xywh(0.0, 0.0, width, height)
            .ok_or(AvengerPdfError::InvalidPageSize { width, height })?;
        let mut builder = PathBuilder::new();
        builder.push_rect(rect);
        let Some(path) = builder.finish() else {
            return Ok(());
        };

        surface.set_fill(Some(Fill {
            paint: rgb::Color::new(color_channel(r), color_channel(g), color_channel(b)).into(),
            opacity: NormalizedF32::new(a).unwrap_or(NormalizedF32::ONE),
            rule: FillRule::NonZero,
        }));
        surface.set_stroke(None);
        surface.draw_path(&path);
        Ok(())
    }
}

fn color_channel(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_scene_graph(width: f32, height: f32) -> SceneGraph {
        SceneGraph {
            width,
            height,
            origin: [0.0, 0.0],
            marks: Vec::new(),
        }
    }

    #[test]
    fn renders_empty_scene_graph_as_pdf() {
        let pdf = PdfRenderer::new()
            .render_scene_graph(&empty_scene_graph(80.0, 40.0))
            .unwrap();

        assert!(pdf.starts_with(b"%PDF-"));
    }

    #[test]
    fn honors_transparent_background() {
        let pdf = PdfRenderer::new()
            .with_options(PdfRenderOptions {
                background: PdfBackground::Transparent,
                compress: false,
                ..Default::default()
            })
            .render_scene_graph(&empty_scene_graph(20.0, 20.0))
            .unwrap();

        assert!(pdf.starts_with(b"%PDF-"));
    }

    #[test]
    fn writes_scene_graph_pdf_to_disk() {
        let tempdir = tempfile::tempdir().unwrap();
        let output = tempdir.path().join("nested").join("chart.pdf");

        PdfRenderer::new()
            .write_scene_graph_pdf(&empty_scene_graph(20.0, 20.0), &output)
            .unwrap();

        let pdf = std::fs::read(output).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
    }
}
