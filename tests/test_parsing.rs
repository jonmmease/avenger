use vega_wgpu_renderer::specs::mark::MarkSpec;

#[test]
fn parse_bar() {
    let scene: MarkSpec = serde_json::from_str(include_str!("specs/bar.sg.json")).unwrap();
    println!("{scene:#?}");
}

#[test]
fn parse_symbol() {
    let scene: MarkSpec = serde_json::from_str(include_str!("specs/circles.sg.json")).unwrap();
    println!("{scene:#?}");
}
