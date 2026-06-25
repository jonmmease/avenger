use avenger_chart::prelude::Plot;
use avenger_chart::{
    doc::render::render_evaluated_plot_to_png,
    layout::Margins,
    render::{EvaluatedPlot, PdfRenderer, SvgRenderer},
};
use avenger_chart_webmercator::{
    RasterTileLayer, TileLoadingPolicy, WebMercator, WebMercatorViewport,
};
use avenger_resource::{ResourceKey, ResourceRequestPurpose, ResourceSource};
use avenger_scenegraph::marks::{
    group::{Clip, SceneGroup},
    image::{SceneImageMark, SceneImageSource, SceneImageUnavailablePolicy},
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
async fn tile_guide_honors_plot_area_origin_and_clip() {
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
                .max_zoom(0),
        );

    let evaluated = Plot::with_coord(coord)
        .canvas_size(300.0, 260.0)
        .margins(Margins::uniform(20.0))
        .compile(&ctx)
        .await
        .expect("compile")
        .evaluate(&ctx, None)
        .await
        .expect("evaluate");

    let guide_group =
        find_group(evaluated.scene_graph.children(), "webmercator-guide").expect("guide group");
    assert_eq!(guide_group.origin, [20.0, 20.0]);
    assert_eq!(
        guide_group.clip,
        Clip::Rect {
            x: 0.0,
            y: 0.0,
            width: 260.0,
            height: 220.0,
        }
    );

    let data_group = first_root_child_group(evaluated.scene_graph.children()).expect("data group");
    assert_eq!(data_group.origin, [20.0, 20.0]);
    assert_eq!(
        data_group.clip,
        Clip::Rect {
            x: 0.0,
            y: 0.0,
            width: 260.0,
            height: 220.0,
        }
    );
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

#[tokio::test]
async fn smooth_zoom_tile_guide_renders_fallback_marks_and_prefetch_requests() {
    let evaluated = evaluated_smooth_tile_plot().await;
    let image_marks = collect_image_marks(evaluated.scene_graph.children());
    assert!(
        image_marks
            .iter()
            .all(|mark| mark.unavailable_policy == SceneImageUnavailablePolicy::Skip),
        "smooth tile marks should skip pending images instead of drawing placeholders"
    );

    let rendered_keys = image_marks
        .iter()
        .filter_map(|mark| {
            mark.image_source_iter().find_map(|source| match source {
                SceneImageSource::Resource(resource) => Some(resource.key.clone()),
                _ => None,
            })
        })
        .collect::<Vec<_>>();
    let prefetch_requests = evaluated
        .resource_requests
        .iter()
        .filter(|request| request.purpose == ResourceRequestPurpose::Prefetch)
        .collect::<Vec<_>>();

    assert!(!prefetch_requests.is_empty());
    assert!(
        prefetch_requests
            .iter()
            .all(|request| !rendered_keys.contains(&request.key))
    );
    assert!(
        prefetch_requests
            .iter()
            .any(|request| tile_key_zoom(&request.key) == 2),
        "expected smooth tile guide to prefetch at the target zoom for short pan gestures"
    );
    assert!(
        evaluated
            .resource_requests
            .iter()
            .any(|request| request.purpose == ResourceRequestPurpose::Required)
    );
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

async fn evaluated_smooth_tile_plot() -> EvaluatedPlot {
    let ctx = SessionContext::new();
    let coord = WebMercator::new()
        .viewport(
            WebMercatorViewport::new()
                .center_lon_lat(0.0, 0.0)
                .zoom(2.0),
        )
        .tiles(
            RasterTileLayer::xyz(TINY_PNG_DATA_URI)
                .id("base")
                .max_zoom(3)
                .loading_policy(TileLoadingPolicy::SmoothZoom {
                    fallback_below: 1,
                    fallback_above: 0,
                    prefetch_below: 1,
                    prefetch_above: 1,
                    pan_prefetch_margin_tiles: 1,
                    max_rendered_fallback_tiles: 128,
                    max_prefetch_tiles: 128,
                }),
        );

    Plot::with_coord(coord)
        .plot_size(256.0, 256.0)
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

fn tile_key_zoom(key: &ResourceKey) -> u8 {
    key.0
        .split('/')
        .nth(2)
        .expect("tile key zoom")
        .parse()
        .expect("tile key zoom number")
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

fn find_group<'a>(marks: &'a [SceneMark], name: &str) -> Option<&'a SceneGroup> {
    for mark in marks {
        let SceneMark::Group(group) = mark else {
            continue;
        };
        if group.name == name {
            return Some(group);
        }
        if let Some(group) = find_group(&group.marks, name) {
            return Some(group);
        }
    }
    None
}

fn first_root_child_group(marks: &[SceneMark]) -> Option<&SceneGroup> {
    let SceneMark::Group(root) = marks.first()? else {
        return None;
    };
    root.marks.iter().find_map(|mark| match mark {
        SceneMark::Group(group) if group.name != "webmercator-guide" => Some(group),
        _ => None,
    })
}
