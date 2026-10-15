//! CPU rasterization and image comparisons for export tests.
use base64::{prelude::BASE64_STANDARD, Engine};
use resvg::{tiny_skia, usvg};
use std::{collections::HashMap, path::Path, sync::Arc};

pub fn svg_to_png(svg: &str, scale: f32) -> image::RgbaImage {
    let mut db = usvg::fontdb::Database::new();
    let mut aliases = HashMap::new();
    // resvg does not load CSS webfonts, so register the document's embedded faces.
    for rule in svg.split("@font-face {").skip(1) {
        let family = rule
            .split("font-family: \"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let uri = rule
            .split("src: url(\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let (header, data) = uri.split_once(',').unwrap();
        let data = BASE64_STANDARD.decode(data).unwrap();
        let data = if header.contains("woff2") {
            font_subset::FontReader::new(&data)
                .unwrap()
                .read()
                .unwrap()
                .to_opentype()
        } else {
            data
        };
        let ids = db.load_font_source(usvg::fontdb::Source::Binary(Arc::new(data)));
        aliases.insert(family.to_string(), ids[0]);
    }
    let options = usvg::Options {
        fontdb: Arc::new(db),
        font_resolver: usvg::FontResolver {
            select_font: Box::new(move |font, _db| {
                font.families().iter().find_map(|family| match family {
                    usvg::FontFamily::Named(name) => aliases.get(name).copied(),
                    _ => None,
                })
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    let tree = usvg::Tree::from_str(svg, &options).unwrap();
    let size = tree.size().to_int_size().scale_by(scale).unwrap();
    let mut pixmap = tiny_skia::Pixmap::new(size.width(), size.height()).unwrap();
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let png = pixmap.encode_png().unwrap();
    image::load_from_memory(&png).unwrap().to_rgba8()
}

/// Asserts that `actual` matches `tests/baselines/{name}.png` in the crate at `crate_dir`, and
/// writes it to the ignored `tests/output/{name}.png`, so that an intended change is accepted by
/// copying it. Callers pass `env!("CARGO_MANIFEST_DIR")`.
pub fn assert_baseline(crate_dir: &str, actual: &image::RgbaImage, name: &str) {
    let tests = Path::new(crate_dir).join("tests");
    let output = tests.join("output").join(format!("{name}.png"));
    std::fs::create_dir_all(output.parent().unwrap()).unwrap();
    actual.save(&output).unwrap();
    let baseline = tests.join("baselines").join(format!("{name}.png"));
    let expected = image::open(&baseline)
        .expect("export baseline must exist")
        .to_rgba8();
    assert_eq!(actual.dimensions(), expected.dimensions());
    let error: f64 = actual
        .as_raw()
        .iter()
        .zip(expected.as_raw())
        .map(|(a, b)| a.abs_diff(*b) as f64)
        .sum();
    let mean_error = error / actual.as_raw().len() as f64;
    assert!(
        mean_error <= 0.1,
        "mean channel difference {mean_error:.4} exceeds 0.1; to accept {}, copy it to {}",
        output.display(),
        baseline.display()
    );
}
