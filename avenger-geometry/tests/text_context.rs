use avenger_common::value::ScalarOrArray;
use avenger_geometry::{marks::MarkGeometryUtils, rtree::SceneGraphRTree};
use avenger_scenegraph::{
    marks::{group::SceneGroup, text::SceneTextMark},
    scene_graph::SceneGraph,
};
use avenger_text::{
    measurement::TextMeasurementConfig,
    types::{FontStyle, FontWeight, TextSyntaxMode},
    FontResolutionOptions, LabelParamValue, TextEngine,
};
use geo::BoundingRect;

#[test]
fn geometry_uses_registered_fonts_parameters_locales_and_the_same_width_limit() {
    let engine = TextEngine::with_font_resolution(&FontResolutionOptions {
        load_system_fonts: false,
        default_sans_serif_family: Some("DejaVu Sans Mono".to_string()),
        ..avenger_text::default_font_resolution()
    })
    .unwrap();
    let mut mark = SceneTextMark {
        text: ScalarOrArray::new_scalar("#label #numfmt(value, \",.2f\")".to_string()),
        text_syntax: TextSyntaxMode::TypstMarkup,
        font: "sans-serif".to_string().into(),
        font_size: 20.0.into(),
        number_locale: Some("wide".to_string()),
        ..Default::default()
    };
    mark.text_params.insert(
        "label".into(),
        LabelParamValue::Str("A very long resolved label".to_string()),
    );
    mark.text_params
        .insert("value".into(), LabelParamValue::Float(1234.5));
    mark.number_locale_specs.insert(
        "wide".into(),
        avenger_text::NumberLocaleSpec {
            decimal: Some("decimal".into()),
            group: Some("group".into()),
            ..Default::default()
        },
    );
    let source = mark.text.as_vec(1, None)[0].clone();
    for limit in [f32::INFINITY, 75.0] {
        mark.limit = limit.into();
        let config = TextMeasurementConfig {
            text: &source,
            font: "sans-serif",
            font_size: 20.0,
            font_weight: FontWeight::default(),
            font_style: FontStyle::Normal,
            syntax_mode: mark.text_syntax,
            params: &mark.text_params,
            number_locale: mark.number_locale.as_deref(),
            number_locale_specs: Some(&mark.number_locale_specs),
            datetime_locale: None,
            datetime_timezone: None,
            datetime_locale_specs: None,
        };
        let expected = engine.measure_bounds_with_limit(&config, limit).unwrap();
        let bounds = mark.bounding_box_with_text_engine(&engine);
        let geometry = mark
            .geometry_iter_with_text_engine(vec![0], [0.0, 0.0], &engine)
            .next()
            .unwrap();
        assert!(
            (geometry.geometry.bounding_rect().unwrap().width() - expected.width).abs() < 0.001
        );
        if limit.is_infinite() {
            assert!(
                (expected.width
                    - avenger_text::default_text_engine()
                        .measure_bounds(&config)
                        .unwrap()
                        .width)
                    .abs()
                    > 1.0
            );
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
        let tree = SceneGraphRTree::from_scene_graph_with_text_engine(&scene, &engine);
        assert_eq!(tree.envelope(), &bounds);
    }
}
