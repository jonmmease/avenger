#[cfg(test)]
mod tests {
    use avenger_vega::scene_graph::VegaSceneGraph;
    use std::fs;

    #[test]
    fn try_it() {
        let category = "rule";
        let spec_name = "wide_rule_axes";
        let specs_dir = format!(
            "{}/../avenger-vega-test-data/vega-scenegraphs/{category}",
            env!("CARGO_MANIFEST_DIR")
        );

        // Read scene graph spec
        let scene_spec_str =
            fs::read_to_string(format!("{specs_dir}/{spec_name}.sg.json")).unwrap();
        let scene_spec: VegaSceneGraph = serde_json::from_str(&scene_spec_str).unwrap();

        println!("{:#?}", scene_spec);

        // Convert to scene graph

        let sg = scene_spec.to_scene_graph().unwrap();
        println!("{sg:#?}");
    }
}
