use regex::Regex;
use serde_json::json;
use std::fs::DirEntry;
use std::path::Path;
use std::{fs, io};
use vl_convert_rs::VlConverter;

/// Generate test data for each Vega spec located in `sg2d-vega-test-data/vega-specs`
/// For each spec, the following three files are saved to `sg2d-vega-test-data/vega-scenegraphs`
///   1. spec_name.dims.json: This is a JSON file containing the chart's width, height, and origin
///   2. spec_name.sg.json: This is a JSON file containing the chart's scene graph
///   3. spec_name.png: This is a PNG rendering of the chart using vl-convert with resvg
fn main() {
    let mut converter = VlConverter::new();

    let specs_dir = format!("{}/vega-specs", env!("CARGO_MANIFEST_DIR"));
    let specs_path = Path::new(&specs_dir);
    visit_dirs(specs_path, &mut move |dir_entry| {
        let binding = dir_entry.path();
        let spec_category = binding
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let spec_file_name = dir_entry.file_name().to_str().unwrap().to_string();

        if let Some(spec_name) = spec_file_name.strip_suffix(".vg.json") {
            let path = format!(
                "{}/vega-specs/{spec_category}/{spec_name}.vg.json",
                env!("CARGO_MANIFEST_DIR")
            );
            let output_dir = format!(
                "{}/vega-scenegraphs/{spec_category}/",
                env!("CARGO_MANIFEST_DIR")
            );
            fs::create_dir_all(Path::new(&output_dir)).unwrap();

            // Load input Vega spec
            let spec_str = fs::read_to_string(path).unwrap();
            let vg_spec: serde_json::Value = serde_json::from_str(&spec_str).unwrap();

            // Write dimensions JSON file
            let svg =
                pollster::block_on(converter.vega_to_svg(vg_spec.clone(), Default::default()))
                    .unwrap();
            let width = get_svg_width(&svg);
            let height = get_svg_height(&svg);
            let origin = get_svg_origin(&svg);
            let dims_str = serde_json::to_string_pretty(&json!({
                "width": width,
                "height": height,
                "origin_x": origin.0,
                "origin_y": origin.1
            }))
            .unwrap();
            fs::write(format!("{output_dir}/{spec_name}.dims.json"), dims_str).unwrap();

            // Save PNG
            let png = pollster::block_on(converter.vega_to_png(
                vg_spec.clone(),
                Default::default(),
                Some(2.0),
                None,
            ))
            .unwrap();
            fs::write(format!("{output_dir}/{spec_name}.png"), png).unwrap();

            // Save Scene Graph
            let scene_graph =
                pollster::block_on(converter.vega_to_scenegraph(vg_spec, Default::default()))
                    .unwrap();
            let scene_graph_str = serde_json::to_string_pretty(&scene_graph).unwrap();
            fs::write(format!("{output_dir}/{spec_name}.sg.json"), scene_graph_str).unwrap();
        }
    })
    .unwrap();
}

/// Get the image width from the SVG
fn get_svg_width(svg: &str) -> u32 {
    let width_re = Regex::new("width=\"(\\d+)\"").unwrap();
    let captures = width_re.captures(svg).expect("Missing width");
    captures.get(1).unwrap().as_str().parse().unwrap()
}

/// Get the image height from the SVG
fn get_svg_height(svg: &str) -> u32 {
    let width_re = Regex::new("height=\"(\\d+)\"").unwrap();
    let captures = width_re.captures(svg).expect("Missing height");
    captures.get(1).unwrap().as_str().parse().unwrap()
}

/// Get the renderer origin by extracting the first `translate` transform from the SVG
fn get_svg_origin(svg: &str) -> (u32, u32) {
    let width_re = Regex::new("translate\\((\\d+),(\\d+)\\)").unwrap();
    let captures = width_re.captures(svg).expect("Missing height");
    let origin_x: u32 = captures.get(1).unwrap().as_str().parse().unwrap();
    let origin_y: u32 = captures.get(2).unwrap().as_str().parse().unwrap();
    (origin_x, origin_y)
}

fn visit_dirs(dir: &Path, cb: &mut dyn FnMut(&DirEntry)) -> io::Result<()> {
    if dir.is_dir() {
        let mut entries = fs::read_dir(dir)?
            // .map(|res| res.map(|e| e.path()))
            .collect::<Result<Vec<_>, io::Error>>()?;

        entries.sort_by_key(|d| d.path());

        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                visit_dirs(&path, cb)?;
            } else {
                cb(&entry);
            }
        }
    }
    Ok(())
}
