use avenger_common::{lyon::parse_svg_path, types::FillRule};
use avenger_scenegraph::marks::{
    group::Clip, path::ScenePathMark, pattern::PatternSymbol, symbol::SceneSymbolMark,
};

#[test]
fn fill_rules_round_trip_and_preserve_omitted_field_defaults() {
    let mut value = serde_json::to_value(ScenePathMark {
        fill_rule: FillRule::EvenOdd,
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        serde_json::from_value::<ScenePathMark>(value.clone())
            .unwrap()
            .fill_rule,
        FillRule::EvenOdd
    );
    value.as_object_mut().unwrap().remove("fill-rule");
    assert_eq!(
        serde_json::from_value::<ScenePathMark>(value)
            .unwrap()
            .fill_rule,
        FillRule::NonZero
    );
    let mut symbol = serde_json::to_value(SceneSymbolMark::default()).unwrap();
    symbol.as_object_mut().unwrap().remove("fill-rule");
    assert_eq!(
        serde_json::from_value::<SceneSymbolMark>(symbol)
            .unwrap()
            .fill_rule,
        FillRule::NonZero
    );
    let pattern = serde_json::json!({"shape":"circle","size":10.0});
    assert_eq!(
        serde_json::from_value::<PatternSymbol>(pattern)
            .unwrap()
            .fill_rule,
        FillRule::EvenOdd
    );
    let clip = Clip::Path {
        path: parse_svg_path("M0,0 H10 V10 Z").unwrap(),
        fill_rule: FillRule::EvenOdd,
    };
    assert_eq!(
        serde_json::from_value::<Clip>(serde_json::to_value(&clip).unwrap()).unwrap(),
        clip
    );
}
