use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::{marks::text::SceneTextMark, scene_graph::SceneGraph};
use avenger_text::types::{TextBaseline, TextSyntaxMode};
use geo::BoundingRect;

#[test]
fn picking_respects_rich_label_width_and_explicit_font_context() {
    let engine =
        avenger_text::TextEngine::with_font_resolution(&avenger_text::FontResolutionOptions {
            default_sans_serif_family: Some("DejaVu Sans Mono".into()),
            ..avenger_text::default_font_resolution()
        })
        .unwrap();
    let scene = SceneGraph {
        width: 500.0,
        height: 100.0,
        origin: [0.0, 0.0],
        marks: vec![SceneTextMark {
            name: "label".into(),
            text: "*Long annotation* with $sqrt(x^2 + y^2)$"
                .to_string()
                .into(),
            text_syntax: TextSyntaxMode::TypstMarkup,
            font: "sans-serif".to_string().into(),
            font_size: 24.0.into(),
            baseline: TextBaseline::Top.into(),
            x: 20.0.into(),
            y: 20.0.into(),
            limit: 120.0.into(),
            ..Default::default()
        }
        .into()],
    };
    let tree = SceneGraphRTree::from_scene_graph_with_text_engine(&scene, &engine);
    let bounds = tree
        .iter()
        .next()
        .unwrap()
        .geometry
        .bounding_rect()
        .unwrap();
    assert!((bounds.width() - 120.0).abs() < 0.001);
    assert!(tree.pick_top_mark_at_point(&[40.0, 30.0]).is_some());
    assert!(tree.pick_top_mark_at_point(&[160.0, 30.0]).is_none());
}
