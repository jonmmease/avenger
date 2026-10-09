use avenger_common::types::TextSyntaxMode;
use avenger_common::value::ScalarOrArray;
use avenger_geometry::{marks::MarkGeometryUtils, rtree::SceneGraphRTree};
use avenger_scenegraph::{
    marks::{group::SceneGroup, text::SceneTextMark},
    scene_graph::SceneGraph,
};
use avenger_typst_label::{
    bind, bundled_font_options, bundled_label_engine, EngineOptions, FontOptions, LabelEngine,
    LabelValue, LabelValues, LabelWidth,
};
use geo::BoundingRect;

#[test]
fn geometry_uses_registered_fonts_bound_values_locales_and_the_same_width_limit() {
    use avenger_format_number_d3::D3NumberFormatProvider;
    let number_format = std::sync::Arc::new(
        D3NumberFormatProvider::new()
            .with_locale("wide")
            .with_custom_locale(
                "wide",
                serde_json::from_str(r#"{"decimal":"decimal","thousands":"group","grouping":[3]}"#)
                    .unwrap(),
            ),
    );
    let values = LabelValues::from([
        (
            "label".into(),
            LabelValue::Str("A very long resolved label".to_string()),
        ),
        ("value".into(), LabelValue::Float(1234.5)),
    ]);
    let engine = LabelEngine::new(EngineOptions {
        fonts: FontOptions {
            load_system_fonts: false,
            default_sans_serif_family: Some("DejaVu Sans Mono".to_string()),
            ..bundled_font_options()
        },
    })
    .with_number_formatting(number_format.clone());
    let mut mark = SceneTextMark {
        text: ScalarOrArray::new_scalar(bind("#label #numfmt(value, \",.2f\")", &values).unwrap()),
        text_syntax: TextSyntaxMode::TypstMarkup,
        font: "sans-serif".to_string().into(),
        font_size: 20.0.into(),
        wrap: false,
        ellipsis: true,
        ..Default::default()
    };
    for width in [LabelWidth::Auto, LabelWidth::Max(75.0)] {
        mark.width = width.into();
        let label = mark.labels().next().unwrap().label;
        let expected = engine.bounds(&label).unwrap();
        let bounds = mark.bounding_box(&engine);
        let geometry = mark
            .geometry_iter(vec![0], [0.0, 0.0], &engine)
            .next()
            .unwrap();
        assert!(
            (geometry.geometry.bounding_rect().unwrap().width() - expected.width).abs() < 0.001
        );
        if width == LabelWidth::Auto {
            // The registered default family sets the width.
            let other_fonts = bundled_label_engine().with_number_formatting(number_format.clone());
            assert!((expected.width - other_fonts.bounds(&label).unwrap().width).abs() > 1.0);
        }
        let scene = SceneGraph {
            marks: vec![SceneGroup {
                marks: vec![mark.clone().into()],
                ..Default::default()
            }
            .into()],
            width: 500.0,
            height: 100.0,
            origin: [0.0, 0.0],
        };
        let tree = SceneGraphRTree::from_scene_graph(&scene, &engine);
        assert_eq!(tree.envelope(), &bounds);
        // Picking finds the label within its width only.
        assert!(tree.pick_top_mark_at_point(&[10.0, -5.0]).is_some());
        assert_eq!(
            tree.pick_top_mark_at_point(&[100.0, -5.0]).is_some(),
            width == LabelWidth::Auto
        );
    }
}
