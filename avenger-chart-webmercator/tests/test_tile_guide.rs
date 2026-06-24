use avenger_chart::prelude::Plot;
use avenger_chart::{
    doc::render::render_evaluated_plot_to_png,
    render::{EvaluatedPlot, PdfRenderer, SvgRenderer},
};
use avenger_chart_webmercator::{RasterTileLayer, WebMercator, WebMercatorViewport};
use avenger_resource::{ResourceKey, ResourceSource};
use avenger_scenegraph::marks::{
    image::{SceneImageMark, SceneImageSource},
    mark::SceneMark,
    text::SceneTextMark,
};
use datafusion::prelude::SessionContext;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const TINY_PNG_DATA_URI: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAG0lEQVR4nGO4o6b2XzX59X8GscVe/3+dEf0PAE8fCXZKLiUkAAAAAElFTkSuQmCC";

#[tokio::test]
async fn tile_guide_requests_resources_and_renders_image_marks() {
    let evaluated = evaluated_tile_plot().await;

    assert_eq!(evaluated.resource_requests.len(), 1);
    let request = &evaluated.resource_requests[0];
    assert_eq!(request.key, ResourceKey::new("webmercator/base/0/0/0/256"));
    assert!(matches!(request.source, ResourceSource::DataUri { .. }));

    let image_marks = collect_image_marks(evaluated.scene_graph.children());
    assert!(!image_marks.is_empty());
    for image_mark in image_marks {
        assert!(!image_mark.interactive);
        assert_eq!(image_mark.zindex, Some(-100));
        let image = image_mark.image_source_iter().next().expect("image source");
        let SceneImageSource::Resource(resource) = image else {
            panic!("expected resource-backed tile image");
        };
        assert_eq!(resource.key, ResourceKey::new("webmercator/base/0/0/0/256"));
    }

    let text_marks = collect_text_marks(evaluated.scene_graph.children());
    assert_eq!(text_marks.len(), 1);
    assert_eq!(text_marks[0].text_iter().next().unwrap(), "Example tiles");
}

#[tokio::test]
async fn tile_resources_render_to_svg_pdf_and_png_exports() {
    let evaluated = evaluated_tile_plot().await;

    let svg = SvgRenderer::new()
        .render_evaluated_plot(&evaluated)
        .expect("render svg");
    assert!(svg.contains("data:image/png;base64"));

    let pdf = PdfRenderer::new()
        .render_evaluated_plot(&evaluated)
        .expect("render pdf");
    assert!(pdf.starts_with(b"%PDF-"));

    let png_path = unique_png_path();
    render_evaluated_plot_to_png(&evaluated, &png_path)
        .await
        .expect("render png");
    let metadata = std::fs::metadata(&png_path).expect("png metadata");
    assert!(metadata.len() > 0);
    let _ = std::fs::remove_file(png_path);
}

async fn evaluated_tile_plot() -> EvaluatedPlot {
    let ctx = SessionContext::new();
    let coord = WebMercator::new()
        .viewport(
            WebMercatorViewport::new()
                .center_lon_lat(0.0, 0.0)
                .zoom(0.0),
        )
        .tiles(
            RasterTileLayer::xyz(TINY_PNG_DATA_URI)
                .id("base")
                .max_zoom(0)
                .attribution("Example tiles"),
        );

    Plot::with_coord(coord)
        .compile(&ctx)
        .await
        .expect("compile")
        .evaluate(&ctx, None)
        .await
        .expect("evaluate")
}

fn unique_png_path() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time since epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("avenger-webmercator-tile-{nanos}.png"))
}

fn collect_image_marks(marks: &[SceneMark]) -> Vec<&SceneImageMark> {
    let mut out = Vec::new();
    collect_image_marks_inner(marks, &mut out);
    out
}

fn collect_image_marks_inner<'a>(marks: &'a [SceneMark], out: &mut Vec<&'a SceneImageMark>) {
    for mark in marks {
        match mark {
            SceneMark::Image(image) => out.push(image),
            SceneMark::Group(group) => collect_image_marks_inner(&group.marks, out),
            _ => {}
        }
    }
}

fn collect_text_marks(marks: &[SceneMark]) -> Vec<&SceneTextMark> {
    let mut out = Vec::new();
    collect_text_marks_inner(marks, &mut out);
    out
}

fn collect_text_marks_inner<'a>(marks: &'a [SceneMark], out: &mut Vec<&'a SceneTextMark>) {
    for mark in marks {
        match mark {
            SceneMark::Text(text) => out.push(text),
            SceneMark::Group(group) => collect_text_marks_inner(&group.marks, out),
            _ => {}
        }
    }
}
