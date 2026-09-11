//! CPU rasterization and image comparisons for export tests.
use base64::{prelude::BASE64_STANDARD, Engine};
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

pub fn assert_baseline(actual: &image::RgbaImage, baseline: &Path) {
    if std::env::var_os("AVENGER_UPDATE_EXPORT_BASELINES").is_some() {
        actual.save(baseline).unwrap();
    }
    let expected = image::open(baseline)
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
    if mean_error > 0.1 {
        let failure = std::env::temp_dir().join(format!(
            "avenger-{}",
            baseline.file_name().unwrap().to_string_lossy()
        ));
        actual.save(&failure).unwrap();
        panic!(
            "mean channel difference {mean_error:.4} exceeds 0.1; actual image: {}",
            failure.display()
        );
    }
}
