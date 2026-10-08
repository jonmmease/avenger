mod common;

use avenger_color::AbsoluteColor;
use avenger_typst_label::{
    CompiledLabel, Em, EngineOptions, FontStyle, FontWeight, FrameItem, LabelEngine,
    LabelError, LabelOptions, LabelParamValue, LabelParams, MathStyle, PdfItem,
    PdfOptions, SvgOptions, pdf_items, svg_items,
};
use indexmap::IndexMap;
use std::path::{Path, PathBuf};

#[cfg(feature = "raster")]
use avenger_typst_label::{RasterOptions, rasterize};

fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= 0.001,
        "expected {actual} to be within 0.001 of {expected}"
    );
}

/// The families of a label's text items.
fn families(label: &CompiledLabel) -> Vec<String> {
    let mut families: Vec<_> = label
        .frame
        .text_items()
        .into_iter()
        .map(|(_, text)| text.font.family().to_string())
        .collect();
    families.dedup();
    families
}

fn has_shape(label: &CompiledLabel) -> bool {
    let mut shape = false;
    label.frame.visit(Default::default(), &mut |_, item| {
        shape |= matches!(item, FrameItem::Shape(_));
    });
    shape
}

#[test]
fn final_public_api_compiles_measures_and_lowers_markup_label() {
    let engine = LabelEngine::new(common::engine_options());
    let mut params = LabelParams::default();
    params.insert("series_name".to_string(), LabelParamValue::Str("Revenue".to_string()));

    let source = "#strong[#series_name] $sqrt(x^2 + y^2)$";
    let label = engine
        .with_params(params.clone())
        .compile(source, &LabelOptions::default())
        .unwrap();
    let measured = engine
        .with_params(params.clone())
        .measure(source, &LabelOptions::default())
        .unwrap();

    assert!(label.flags.has_math);
    assert!(label.metrics.width > 0.0);
    assert!(label.metrics.height > 0.0);
    assert_eq!(measured, label.metrics);
    let families = families(&label);
    assert!(families.contains(&"Lato".into()), "{families:?}");
    assert!(families.contains(&"Lete Sans Math".into()), "{families:?}");
    assert!(has_shape(&label), "sqrt should emit a radical shape");

    let svg = svg_items(&label, &SvgOptions::default());
    assert_eq!(svg.size, label.frame.size);
    assert!(!svg.items.is_empty());

    let pdf = pdf_items(&label, &PdfOptions::default());
    assert_eq!(pdf.size, label.frame.size);
    assert!(pdf.semantic_text.contains("Revenue"));
    assert!(pdf.items.iter().any(|item| matches!(item, PdfItem::Text(_))));
    assert!(pdf.items.iter().any(|item| matches!(item, PdfItem::Path(_))));
}

#[test]
fn final_public_api_exposes_options_and_external_param_model() {
    let mut engine_options = EngineOptions::default();
    engine_options.fonts.load_system_fonts = false;

    assert!(!engine_options.fonts.load_system_fonts);

    let mut dict = IndexMap::new();
    dict.insert("cap".to_string(), LabelParamValue::Str("round".to_string()));
    let mut options = LabelOptions::default();
    options.text.font_family = "Lato".to_string();
    options.text.font_size = 15.0;
    options.text.fill = AbsoluteColor::from_srgb(0.1, 0.2, 0.3, 1.0);
    options.text.font_weight = FontWeight::from_number(500);
    options.text.font_style = FontStyle::Italic;
    options.text.lang = "de".parse().unwrap();
    options.text.region = Some("AT".parse().unwrap());
    options.math = MathStyle {
        font_family: "Lete Sans Math".to_string(),
        font_size: Some(Em(1.25)),
        fill: Some(AbsoluteColor::from_srgb(0.3, 0.2, 0.1, 1.0)),
        font_weight: Some(FontWeight::BOLD),
    };
    let mut params = LabelParams::default();
    params.insert("none".to_string(), LabelParamValue::None);
    params.insert("flag".to_string(), LabelParamValue::Bool(true));
    params.insert("count".to_string(), LabelParamValue::Int(7));
    params.insert("ratio".to_string(), LabelParamValue::Float(0.25));
    params.insert("name".to_string(), LabelParamValue::Str("Series".to_string()));
    params.insert(
        "array".to_string(),
        LabelParamValue::Array(vec![LabelParamValue::Int(1)]),
    );
    params.insert("stroke".to_string(), LabelParamValue::Dict(dict));

    let engine = LabelEngine::new(common::engine_options());
    let label = engine
        .with_params(params)
        .compile("#name $x^#count$", &options)
        .unwrap();
    let math = label
        .frame
        .text_items()
        .into_iter()
        .find(|(_, text)| text.font.family() == "Lete Sans Math")
        .unwrap()
        .1;
    assert_close(math.size, 15.0 * 1.25);
    assert_eq!(math.fill, AbsoluteColor::from_srgb(0.3, 0.2, 0.1, 1.0));
    // The math weight applies to math alone.
    assert_eq!(math.font.postscript_name().as_deref(), Some("LeteSansMath-Bold"));

    options.limits.max_source_bytes = 4;
    let err = engine.compile("12345", &options).unwrap_err();
    assert!(matches!(err, LabelError::SourceTooLarge { actual: 5, limit: 4 }));
}

#[test]
fn output_lowerers_consume_compiled_frame_not_source_text() {
    let engine = LabelEngine::new(common::engine_options());
    let mut label = engine
        .compile("Price \\$7 $sqrt(x)$", &LabelOptions::default())
        .unwrap();
    let size = label.frame.size;

    label.source = "this would be invalid if a lowerer parsed it: $x^$ #let".to_string();

    let svg = svg_items(&label, &SvgOptions::default());
    assert_eq!(svg.size, size);
    assert!(!svg.items.is_empty());

    let pdf = pdf_items(&label, &PdfOptions::default());
    assert_eq!(pdf.size, size);
    assert!(pdf.semantic_text.contains("Price $7"));
    assert!(!pdf.items.is_empty());
}

#[cfg(feature = "raster")]
#[test]
fn raster_lowerer_consumes_compiled_frame_not_source_text() {
    let engine = LabelEngine::new(common::engine_options());
    let mut label = engine
        .compile("Price \\$7 $sqrt(x)$", &LabelOptions::default())
        .unwrap();

    label.source = "this would be invalid if raster parsed it: $x^$ #let".to_string();

    let raster = rasterize(&label, &RasterOptions { scale: 1.0 }).unwrap();
    assert!(raster.image.width > 0);
    assert!(raster.image.height > 0);
}

#[test]
fn final_public_api_extracts_referenced_params() {
    let source = "#upper[#series] #underline(stroke: series_color)[care] \
        $y = #slope x + #intercept$ #series";
    let expected = vec![
        "series".to_string(),
        "series_color".to_string(),
        "slope".to_string(),
        "intercept".to_string(),
    ];

    assert_eq!(avenger_typst_label::referenced_params(source).unwrap(), expected);

    let engine = LabelEngine::new(common::engine_options());
    assert_eq!(engine.referenced_params(source).unwrap(), expected);
}

#[test]
fn typst_mirrored_modules_do_not_depend_on_public_label_params() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src_dir = manifest_dir.join("src");
    let mut sources = Vec::new();
    collect_typst_sources(&src_dir, &mut sources);

    let forbidden = ["LabelParams", "LabelParamValue", "render_label_param"];
    let mut matches = Vec::new();
    for path in sources {
        let text = std::fs::read_to_string(&path).unwrap();
        for (index, line) in text.lines().enumerate() {
            if forbidden.iter().any(|needle| line.contains(needle)) {
                matches.push(format!("{}:{}: {line}", path.display(), index + 1));
            }
        }
    }

    assert!(
        matches.is_empty(),
        "mirrored typst modules should use typst_library::foundations::Scope, not public label params:\n{}",
        matches.join("\n")
    );
}

fn collect_typst_sources(dir: &Path, sources: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            let is_typst_dir =
                path.file_name().and_then(|name| name.to_str()).is_some_and(|name| {
                    name == "typst_library" || name.starts_with("typst_")
                });
            if is_typst_dir {
                collect_rs_sources(&path, sources);
            }
        }
    }
}

fn collect_rs_sources(dir: &Path, sources: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_rs_sources(&path, sources);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(path);
        }
    }
}

#[test]
fn missing_font_policy_errors_warns_or_falls_back() {
    use avenger_typst_label::{LabelWarning, MissingFontPolicy};
    let mut options = LabelOptions::default();
    options.text.font_family = "UnavailableFontForPolicyTest".to_string();
    for policy in
        [MissingFontPolicy::Error, MissingFontPolicy::Warn, MissingFontPolicy::Fallback]
    {
        let mut engine_options = common::engine_options();
        engine_options.fonts.missing_font = policy;
        let engine = LabelEngine::new(engine_options);
        let result = engine.compile_text("Text", &options);
        match policy {
            MissingFontPolicy::Error => {
                assert!(matches!(result, Err(LabelError::MissingFont { .. })))
            }
            MissingFontPolicy::Warn => assert!(
                result
                    .unwrap()
                    .warnings
                    .iter()
                    .any(|warning| matches!(warning, LabelWarning::MissingFont { .. }))
            ),
            MissingFontPolicy::Fallback => assert!(result.unwrap().warnings.is_empty()),
        }
    }
}

#[test]
fn engines_and_labels_are_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<LabelEngine>();
    assert_send_sync::<CompiledLabel>();
}

#[cfg(feature = "serde")]
#[test]
fn options_round_trip_through_serde() {
    use avenger_typst_label::{LabelAlign, LabelLineHeight, LabelWidth, TextDir};

    let mut options = LabelOptions::default();
    options.text.font_family = "Lato, sans-serif".into();
    options.text.font_weight = FontWeight::from_number(500);
    options.text.font_style = FontStyle::Italic;
    options.text.fill = AbsoluteColor::from_srgb(0.1, 0.2, 0.3, 0.5);
    options.text.lang = "de".parse().unwrap();
    options.text.region = Some("CH".parse().unwrap());
    options.text.dir = TextDir::Rtl;
    options.math = MathStyle {
        font_family: "Lete Sans Math".into(),
        font_size: Some(Em(0.9)),
        fill: Some(AbsoluteColor::from_srgb(0.3, 0.2, 0.1, 1.0)),
        font_weight: Some(FontWeight::BOLD),
    };
    options.width = LabelWidth::Max(120.0);
    options.wrap = false;
    options.align = LabelAlign::Center;
    options.line_height = LabelLineHeight::Relative(1.1);
    options.max_lines = std::num::NonZeroUsize::new(2);
    options.ellipsis = true;
    options.hanging_signs = true;
    options.newline_breaks = true;
    let json = serde_json::to_value(&options).unwrap();
    // Weights are numbers, and styles, languages and regions their names.
    assert_eq!(json["text"]["font_weight"], 500);
    assert_eq!(json["text"]["font_style"], "italic");
    assert_eq!(json["text"]["lang"], "de");
    assert_eq!(json["text"]["region"], "CH");
    assert_eq!(json["width"], serde_json::json!({ "Max": 120.0 }));
    assert_eq!(json["align"], "Center");
    assert_eq!(json["line_height"], serde_json::json!({ "Relative": 1.100000023841858 }));
    assert_eq!(serde_json::from_value::<LabelOptions>(json.clone()).unwrap(), options);
    assert_eq!(
        (json["wrap"].clone(), json["max_lines"].clone(), json["ellipsis"].clone()),
        (false.into(), 2.into(), true.into())
    );

    let mut engine = EngineOptions::default();
    engine.fonts.default_math_family = Some("Lete Sans Math".into());
    let json = serde_json::to_string(&engine).unwrap();
    assert_eq!(serde_json::from_str::<EngineOptions>(&json).unwrap(), engine);
}

#[test]
fn referenced_params_are_the_names_the_library_lacks() {
    use avenger_typst_label::referenced_params;

    // Library functions, colors, argument names and math symbols are not parameters.
    let source =
        "#underline(stroke: 1.5pt + red, evade: true)[care] $alpha + frac(1, sqrt(x))$";
    assert_eq!(referenced_params(source).unwrap(), Vec::<String>::new());
    // The formatting functions' arguments are, and so are math names the library lacks.
    let source = "Peak #numfmt(value, number_format) on #datetimefmt(report_date, date_format) $rate t$";
    assert_eq!(
        referenced_params(source).unwrap(),
        ["value", "number_format", "report_date", "date_format", "rate"]
    );
}

#[test]
fn fonts_in_extra_dirs_resolve_by_family() {
    use std::io::Read;

    let dir = common::oracle::output_dir("extra_font_dirs");
    std::fs::create_dir_all(&dir).unwrap();
    let mut font = Vec::new();
    brotli::Decompressor::new(
        &include_bytes!("fixtures/fonts/NotoSansHebrew.ttf.br")[..],
        4096,
    )
    .read_to_end(&mut font)
    .unwrap();
    std::fs::write(dir.join("NotoSansHebrew.ttf"), font).unwrap();

    let mut options = EngineOptions::default();
    options.fonts.load_system_fonts = false;
    options.fonts.extra_font_dirs.push(dir);
    let engine = LabelEngine::new(options);
    let mut label_options = LabelOptions::default();
    label_options.text.font_family = "Noto Sans Hebrew".into();
    let label = engine.compile("שלום", &label_options).unwrap();
    assert_eq!(families(&label), ["Noto Sans Hebrew"]);
    assert!(label.frame.text_items()[0].1.glyphs.iter().all(|glyph| glyph.id != 0));
}
