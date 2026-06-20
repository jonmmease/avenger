use std::path::Path;

use avenger_scenegraph::{
    marks::mark::SceneMark,
    render_order::{SceneDisplayList, SceneDisplayMark},
    scene_graph::SceneGraph,
};
use avenger_svg::{SvgBackground, SvgFontEmbedding, SvgImageMode, SvgRenderOptions, SvgRenderer};
use avenger_text::{
    types::{FontStyle, FontWeight, FontWeightNameSpec},
    MissingFontPolicy,
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
        let usvg_options = self.usvg_options();
        if self.options.embed_text
            && matches!(
                self.options.font_resolution.missing_font,
                MissingFontPolicy::Error
            )
        {
            validate_scene_graph_text_fonts(scene_graph, usvg_options.fontdb.as_ref())?;
        }

        let svg = self.render_svg_for_pdf(scene_graph)?;
        let tree = svg2pdf::usvg::Tree::from_str(&svg, &usvg_options)?;
        if self.options.embed_text
            && matches!(
                self.options.font_resolution.missing_font,
                MissingFontPolicy::Error
            )
        {
            validate_text_fonts_for_embedding(&tree)?;
        }
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

fn validate_scene_graph_text_fonts(
    scene_graph: &SceneGraph,
    fontdb: &svg2pdf::usvg::fontdb::Database,
) -> Result<(), AvengerPdfError> {
    let display_list = SceneDisplayList::from_scene_graph(scene_graph);
    for item in display_list.ordered_items() {
        let SceneDisplayMark::Borrowed(SceneMark::Text(mark)) = &item.mark else {
            continue;
        };

        for (((text, font), font_weight), font_style) in mark
            .text_iter()
            .zip(mark.font_iter())
            .zip(mark.font_weight_iter())
            .zip(mark.font_style_iter())
        {
            if text.chars().all(char::is_whitespace) {
                continue;
            }

            let family = font.trim();
            if family.is_empty() || is_generic_font_family(family) {
                continue;
            }

            if !fontdb_has_scene_family(fontdb, family, font_weight, font_style) {
                return Err(AvengerPdfError::Font(format!(
                    "missing requested text font family {family}"
                )));
            }
        }
    }

    Ok(())
}

fn is_generic_font_family(family: &str) -> bool {
    matches!(
        family,
        "serif" | "sans-serif" | "cursive" | "fantasy" | "monospace"
    )
}

fn fontdb_has_scene_family(
    fontdb: &svg2pdf::usvg::fontdb::Database,
    family: &str,
    font_weight: &FontWeight,
    font_style: &FontStyle,
) -> bool {
    let families = [svg2pdf::usvg::fontdb::Family::Name(family)];
    let query = svg2pdf::usvg::fontdb::Query {
        families: &families,
        weight: svg2pdf::usvg::fontdb::Weight(font_weight_number(font_weight)),
        stretch: svg2pdf::usvg::fontdb::Stretch::Normal,
        style: scene_font_style(font_style),
    };
    fontdb.query(&query).is_some()
}

fn validate_text_fonts_for_embedding(tree: &svg2pdf::usvg::Tree) -> Result<(), AvengerPdfError> {
    let mut error = None;
    validate_group_text_fonts(tree.root(), tree.fontdb(), &mut error);
    if let Some(error) = error {
        return Err(error);
    }
    Ok(())
}

fn validate_group_text_fonts(
    group: &svg2pdf::usvg::Group,
    fontdb: &svg2pdf::usvg::fontdb::Database,
    error: &mut Option<AvengerPdfError>,
) {
    if error.is_some() {
        return;
    }

    for node in group.children() {
        match node {
            svg2pdf::usvg::Node::Group(group) => validate_group_text_fonts(group, fontdb, error),
            svg2pdf::usvg::Node::Text(text) => {
                if let Some(message) = text_font_embedding_error(text, fontdb) {
                    *error = Some(AvengerPdfError::Font(message));
                    return;
                }
            }
            _ => {}
        }

        node.subroots(|subroot| validate_group_text_fonts(subroot, fontdb, error));
        if error.is_some() {
            return;
        }
    }
}

fn text_font_embedding_error(
    text: &svg2pdf::usvg::Text,
    fontdb: &svg2pdf::usvg::fontdb::Database,
) -> Option<String> {
    if !text_has_visible_content(text) {
        return None;
    }

    if let Some(message) = missing_requested_font_error(text, fontdb) {
        return Some(message);
    }

    let mut glyph_count = 0usize;
    for span in text.layouted().iter().filter(|span| span.visible) {
        for glyph in &span.positioned_glyphs {
            glyph_count += 1;
            if glyph.id.0 == 0 {
                return Some(format!(
                    "missing glyph for '{}' in {}",
                    glyph.text,
                    describe_text_fonts(text)
                ));
            }
        }
    }

    if glyph_count == 0 {
        Some(format!(
            "no embeddable glyphs were resolved for {}",
            describe_text_fonts(text)
        ))
    } else {
        None
    }
}

fn missing_requested_font_error(
    text: &svg2pdf::usvg::Text,
    fontdb: &svg2pdf::usvg::fontdb::Database,
) -> Option<String> {
    for chunk in text.chunks() {
        for span in chunk.spans() {
            let span_text = chunk
                .text()
                .get(span.start()..span.end())
                .unwrap_or(chunk.text());
            if !span.is_visible() || !span_text.chars().any(|c| !c.is_whitespace()) {
                continue;
            }

            let named_families = span
                .font()
                .families()
                .iter()
                .filter_map(|family| match family {
                    svg2pdf::usvg::FontFamily::Named(name) => Some(name.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>();

            if named_families.is_empty()
                || named_families
                    .iter()
                    .any(|family| fontdb_has_family(fontdb, family, span.font()))
            {
                continue;
            }

            return Some(format!(
                "missing requested text font families {}",
                named_families.join(", ")
            ));
        }
    }

    None
}

fn fontdb_has_family(
    fontdb: &svg2pdf::usvg::fontdb::Database,
    family: &str,
    font: &svg2pdf::usvg::Font,
) -> bool {
    let families = [svg2pdf::usvg::fontdb::Family::Name(family)];
    let query = svg2pdf::usvg::fontdb::Query {
        families: &families,
        weight: svg2pdf::usvg::fontdb::Weight(font.weight()),
        stretch: font_stretch(font.stretch()),
        style: font_style(font.style()),
    };
    fontdb.query(&query).is_some()
}

fn font_style(style: svg2pdf::usvg::FontStyle) -> svg2pdf::usvg::fontdb::Style {
    match style {
        svg2pdf::usvg::FontStyle::Normal => svg2pdf::usvg::fontdb::Style::Normal,
        svg2pdf::usvg::FontStyle::Italic => svg2pdf::usvg::fontdb::Style::Italic,
        svg2pdf::usvg::FontStyle::Oblique => svg2pdf::usvg::fontdb::Style::Oblique,
    }
}

fn font_stretch(stretch: svg2pdf::usvg::FontStretch) -> svg2pdf::usvg::fontdb::Stretch {
    match stretch {
        svg2pdf::usvg::FontStretch::UltraCondensed => {
            svg2pdf::usvg::fontdb::Stretch::UltraCondensed
        }
        svg2pdf::usvg::FontStretch::ExtraCondensed => {
            svg2pdf::usvg::fontdb::Stretch::ExtraCondensed
        }
        svg2pdf::usvg::FontStretch::Condensed => svg2pdf::usvg::fontdb::Stretch::Condensed,
        svg2pdf::usvg::FontStretch::SemiCondensed => svg2pdf::usvg::fontdb::Stretch::SemiCondensed,
        svg2pdf::usvg::FontStretch::Normal => svg2pdf::usvg::fontdb::Stretch::Normal,
        svg2pdf::usvg::FontStretch::SemiExpanded => svg2pdf::usvg::fontdb::Stretch::SemiExpanded,
        svg2pdf::usvg::FontStretch::Expanded => svg2pdf::usvg::fontdb::Stretch::Expanded,
        svg2pdf::usvg::FontStretch::ExtraExpanded => svg2pdf::usvg::fontdb::Stretch::ExtraExpanded,
        svg2pdf::usvg::FontStretch::UltraExpanded => svg2pdf::usvg::fontdb::Stretch::UltraExpanded,
    }
}

fn font_weight_number(font_weight: &FontWeight) -> u16 {
    match font_weight {
        FontWeight::Name(FontWeightNameSpec::Normal) => 400,
        FontWeight::Name(FontWeightNameSpec::Bold) => 700,
        FontWeight::Number(weight) => *weight as u16,
    }
}

fn scene_font_style(font_style: &FontStyle) -> svg2pdf::usvg::fontdb::Style {
    match font_style {
        FontStyle::Normal => svg2pdf::usvg::fontdb::Style::Normal,
        FontStyle::Italic => svg2pdf::usvg::fontdb::Style::Italic,
    }
}

fn text_has_visible_content(text: &svg2pdf::usvg::Text) -> bool {
    text.chunks().iter().any(|chunk| {
        chunk.spans().iter().any(|span| {
            span.is_visible()
                && chunk
                    .text()
                    .get(span.start()..span.end())
                    .unwrap_or(chunk.text())
                    .chars()
                    .any(|c| !c.is_whitespace())
        })
    })
}

fn describe_text_fonts(text: &svg2pdf::usvg::Text) -> String {
    let mut families = Vec::new();
    for chunk in text.chunks() {
        for span in chunk.spans() {
            for family in span.font().families() {
                let family = family.to_string();
                if !families.contains(&family) {
                    families.push(family);
                }
            }
        }
    }

    if families.is_empty() {
        "requested text fonts".to_string()
    } else {
        format!("requested text font families {}", families.join(", "))
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
    fn writes_scenegraph_pdf_and_creates_parent_dirs() {
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
        let test_dir = std::env::temp_dir().join(format!(
            "avenger-pdf-write-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let output = test_dir.join("nested").join("chart.pdf");

        PdfRenderer::new()
            .write_scene_graph_pdf(&scene_graph, &output)
            .unwrap();

        let pdf = std::fs::read(&output).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
        std::fs::remove_dir_all(test_dir).unwrap();
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
                font: ScalarOrArray::new_scalar("Atkinson Hyperlegible Next".to_string()),
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
    fn embeds_subset_font_for_bundled_text_font() {
        let scene_graph = SceneGraph {
            width: 80.0,
            height: 20.0,
            origin: [0.0, 0.0],
            marks: vec![SceneTextMark {
                text: ScalarOrArray::new_scalar("Embed me".to_string()),
                x: ScalarOrArray::new_scalar(5.0),
                y: ScalarOrArray::new_scalar(12.0),
                font: ScalarOrArray::new_scalar("Atkinson Hyperlegible Next".to_string()),
                font_size: ScalarOrArray::new_scalar(12.0),
                ..Default::default()
            }
            .into()],
        };

        let pdf = PdfRenderer::new()
            .with_options(PdfRenderOptions {
                compress: false,
                ..Default::default()
            })
            .render_scene_graph(&scene_graph)
            .unwrap();

        assert!(pdf.starts_with(b"%PDF-"));
        assert!(pdf_contains(&pdf, b"/FontDescriptor"));
        assert!(pdf_contains(&pdf, b"/FontFile2") || pdf_contains(&pdf, b"/FontFile3"));
        assert!(pdf_contains(&pdf, b"/ToUnicode"));
    }

    #[test]
    fn errors_when_text_font_cannot_be_embedded() {
        let scene_graph = SceneGraph {
            width: 80.0,
            height: 20.0,
            origin: [0.0, 0.0],
            marks: vec![SceneTextMark {
                text: ScalarOrArray::new_scalar("Missing font".to_string()),
                x: ScalarOrArray::new_scalar(5.0),
                y: ScalarOrArray::new_scalar(12.0),
                font: ScalarOrArray::new_scalar("Definitely Missing Avenger Font".to_string()),
                font_size: ScalarOrArray::new_scalar(12.0),
                ..Default::default()
            }
            .into()],
        };

        let err = PdfRenderer::new()
            .render_scene_graph(&scene_graph)
            .unwrap_err();

        assert!(matches!(
            err,
            AvengerPdfError::Font(message)
                if message.contains("missing requested text font family")
                    && message.contains("Definitely Missing Avenger Font")
        ));
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

    fn pdf_contains(pdf: &[u8], needle: &[u8]) -> bool {
        pdf.windows(needle.len()).any(|window| window == needle)
    }
}
