//! The engine's memoized boxes and rasters, and the fallbacks for labels that don't lay out.
#![cfg(feature = "raster")]

mod common;

use std::sync::Arc;

use avenger_format_datetime_d3::D3DateTimeFormatProvider;
use avenger_typst_label::{
    EngineOptions, Label, LabelEngine, LabelError, LabelLineHeight, LabelOptions,
    LabelSource, LabelWidth, MissingFontPolicy, RasterError,
};

fn engine() -> LabelEngine {
    LabelEngine::new(common::engine_options())
        .with_datetime_formatting(Arc::new(D3DateTimeFormatProvider::new()))
}

fn options(font_size: f32) -> LabelOptions {
    let mut options = LabelOptions::default();
    options.text.font_family = "Lato".into();
    options.text.font_size = font_size;
    options
}

/// A markup label at size 14.
fn markup(source: &str) -> Label<'_> {
    Label {
        source: LabelSource::Markup(source),
        options: options(14.0),
    }
}

#[test]
fn formatter_providers_invalidate_boxes_and_rasters() {
    use avenger_format::*;
    #[derive(Debug)]
    struct Provider(String);
    #[derive(Debug)]
    struct Prepared(String);
    impl NumberFormatProvider for Provider {
        fn prepare(
            &self,
            _: &str,
        ) -> Result<Arc<dyn PreparedNumberFormatter>, NumberFormatError> {
            Ok(Arc::new(Prepared(self.0.clone())))
        }
    }
    impl DateTimeFormatProvider for Provider {
        fn prepare_date(
            &self,
            _: &str,
        ) -> Result<Arc<dyn PreparedDateFormatter>, DateTimeFormatError> {
            Ok(Arc::new(Prepared(self.0.clone())))
        }
        fn prepare_naive(
            &self,
            _: &str,
        ) -> Result<Arc<dyn PreparedNaiveDateTimeFormatter>, DateTimeFormatError>
        {
            Ok(Arc::new(Prepared(self.0.clone())))
        }
        fn prepare_zoned(
            &self,
            _: &str,
        ) -> Result<Arc<dyn PreparedZonedDateTimeFormatter>, DateTimeFormatError>
        {
            Ok(Arc::new(Prepared(self.0.clone())))
        }
        fn default_calendar_patterns(&self) -> CalendarPatterns {
            unreachable!("labels format explicit patterns")
        }
    }
    impl PreparedNumberFormatter for Prepared {
        fn format(&self, _: f64) -> FormattedNumber {
            FormattedNumber::plain(self.0.clone())
        }
    }
    impl PreparedDateFormatter for Prepared {
        fn format(&self, _: chrono::NaiveDate) -> Result<String, DateTimeFormatError> {
            Ok(self.0.clone())
        }
    }
    impl PreparedNaiveDateTimeFormatter for Prepared {
        fn format(
            &self,
            _: chrono::NaiveDateTime,
        ) -> Result<String, DateTimeFormatError> {
            Ok(self.0.clone())
        }
    }
    impl PreparedZonedDateTimeFormatter for Prepared {
        fn format(
            &self,
            _: chrono::DateTime<chrono::Utc>,
        ) -> Result<String, DateTimeFormatError> {
            Ok(self.0.clone())
        }
        fn timezone(&self) -> chrono_tz::Tz {
            chrono_tz::UTC
        }
    }
    let configure = |engine: LabelEngine, value: &str| {
        engine
            .with_number_formatting(Arc::new(Provider(value.to_owned())))
            .with_datetime_formatting(Arc::new(Provider(value.to_owned())))
    };
    let first = configure(engine(), "1");
    // Made from a clone of the first engine, whose memos the labels below fill first.
    let second = configure(first.clone(), "100000000");
    let epoch = "datetime(year: 1970, month: 1, day: 1, hour: 0, minute: 0, second: 0, utc: true)";
    for source in [
        "#numfmt(42, \"custom\")".to_string(),
        format!("#datetimefmt({epoch}, \"custom\")"),
    ] {
        let label = markup(&source);
        let first_bounds = first.bounds(&label).unwrap();
        let first_raster = first.raster(&label, 1.0).unwrap();
        let second_bounds = second.bounds(&label).unwrap();
        assert!(second_bounds.width > first_bounds.width * 2.0);
        let second_raster = second.raster(&label, 1.0).unwrap();
        assert!(second_raster.image.is_some());
        assert_eq!(first_raster.bounds, first_bounds);
        assert_eq!(second_raster.bounds, second_bounds);
    }
}

#[test]
fn boxes_measure_math_emoji_bidi_and_complex_scripts() {
    let engine = engine();
    for source in ["Revenue #emoji.face $x^2$", "ABC שלום", "नमस्ते data"]
    {
        let bounds = engine.bounds(&markup(source)).unwrap();
        assert!(bounds.width > 0.0, "{source}");
        assert!(bounds.height >= 14.0, "{source}");
    }
}

#[test]
fn rasters_draw_whole_labels_with_math_and_emoji() {
    // With the system's fonts, the emoji draw in color.
    let mut options = common::engine_options();
    options.fonts.load_system_fonts = true;
    let raster = LabelEngine::new(options)
        .raster(&markup("Hi #emoji.face $x$"), 2.0)
        .unwrap();
    assert!(raster.bounds.width > 0.0);
    let image = raster.image.expect("a raster of a label with text");
    #[cfg(target_os = "macos")]
    {
        let colored = image
            .as_raw()
            .chunks_exact(4)
            .filter(|pixel| {
                let [r, g, b, a] = [pixel[0], pixel[1], pixel[2], pixel[3]];
                a > 0 && r.abs_diff(g).max(r.abs_diff(b)).max(g.abs_diff(b)) > 16
            })
            .count();
        assert!(colored > 20, "emoji should draw in color on macOS");
    }
    let _ = image;
}

#[test]
fn empty_labels_rasterize_to_no_image() {
    let label = Label {
        source: LabelSource::Text(""),
        options: LabelOptions::default(),
    };
    let raster = engine().raster(&label, 1.0).unwrap();
    assert!(raster.image.is_none());
    assert_eq!((raster.bounds.width, raster.bounds.height), (0.0, 12.0));
}

#[test]
fn several_lines_share_their_box_across_boxes_and_rasters() {
    let engine = engine();
    let mut wrapped = options(14.0);
    wrapped.width = LabelWidth::Max(80.0);
    let mut lines = options(14.0);
    lines.newline_breaks = true;
    for label in [
        Label {
            source: LabelSource::Text("Revenue by region in millions of dollars"),
            options: wrapped,
        },
        Label {
            source: LabelSource::Text("Revenue\n(millions)\nby region"),
            options: lines,
        },
        markup("Revenue \\ *ratio* $x^2$"),
    ] {
        let bounds = engine.bounds(&label).unwrap();
        // Several lines, which the memo keeps apart from one line of the same text.
        let (LabelSource::Text(source) | LabelSource::Markup(source)) = label.source;
        let one = Label {
            source: LabelSource::Text(source),
            options: options(14.0),
        };
        assert!(bounds.height > engine.bounds(&one).unwrap().height + 10.0, "{source}");
        assert_eq!(engine.raster(&label, 2.0).unwrap().bounds, bounds, "{source}");
    }
}

#[test]
fn line_boxes_add_half_the_gap_that_line_heights_leave() {
    let engine = engine();
    let mut options = options(14.0);
    options.newline_breaks = true;
    options.line_height = LabelLineHeight::Fixed(30.0);
    let lines = Label {
        source: LabelSource::Text("a\nb\nc"),
        options: options.clone(),
    };
    // The gap between plain lines' boxes is the pitch less the font size.
    let bounds = engine.bounds(&lines).unwrap();
    assert!((bounds.leading - 16.0).abs() < 1e-3, "{bounds:?}");
    // With Typst's spacing, plain lines leave a positive gap.
    options.line_height = LabelLineHeight::Auto;
    let auto = engine.bounds(&Label { options, ..lines }).unwrap();
    assert!(auto.leading > 0.0 && auto.leading < 14.0, "{auto:?}");
}

#[test]
fn math_and_plain_text_share_the_padding_rule() {
    let engine = engine();
    let height = |source| engine.bounds(&markup(source)).unwrap().height;
    // A line shorter than the font size is padded to it, with math or not.
    assert_eq!(height("Radius"), 14.0);
    assert_eq!(height("$x$"), 14.0);
    // Taller content is as tall as it is.
    assert!(height("$display(sum_(i=1)^n x_i)$") > 20.0);
}

#[test]
fn widths_cut_rich_text_alike_in_boxes_and_rasters() {
    let engine = engine();
    let cut = |source, width| {
        let mut label = markup(source);
        label.options.width = LabelWidth::Max(width);
        label.options.wrap = false;
        label.options.ellipsis = true;
        label
    };
    for source in [
        "#\"An expanded string label\"",
        "*A long bold label* $x^2$",
        "#strong[A long label]",
    ] {
        let expected = engine.bounds(&cut(source, 35.0)).unwrap();
        assert!(expected.width <= 35.0, "{source}: {expected:?}");
        let raster = engine.raster(&cut(source, 35.0), 2.0).unwrap();
        assert_eq!(raster.bounds, expected);
        // Within the width, but for the ellipsis's overhang.
        let image = raster.image.as_ref().unwrap();
        assert!(raster.x + image.width() as f32 / 2.0 <= 36.0, "{source}");
        let narrower = engine.raster(&cut(source, 25.0), 2.0).unwrap();
        assert_ne!(raster.key, narrower.key);
    }
}

#[test]
fn invalid_markup_falls_back_to_its_source_as_text() {
    let engine = engine();
    let label = markup("before $x^$ after");
    // Compiling stays strict, for callers that validate markup.
    assert!(matches!(
        engine.compile("before $x^$ after", &options(14.0)),
        Err(LabelError::Source { .. })
    ));
    let as_text = Label {
        source: LabelSource::Text("before $x^$ after"),
        options: options(14.0),
    };
    assert_eq!(engine.bounds(&label).unwrap(), engine.bounds(&as_text).unwrap());
    assert!(engine.raster(&label, 2.0).unwrap().image.is_some());
}

#[test]
fn limit_and_font_errors_survive_the_fallback() {
    let engine = engine();
    let mut limited = options(14.0);
    limited.limits.max_source_bytes = 4;
    for source in [LabelSource::Text("eight!!!"), LabelSource::Markup("eight!!!")] {
        let label = Label { source, options: limited.clone() };
        assert!(matches!(
            engine.raster(&label, 1.0),
            Err(RasterError::Label(LabelError::SourceTooLarge { .. }))
        ));
        assert!(matches!(engine.bounds(&label), Err(LabelError::SourceTooLarge { .. })));
    }

    let mut options = common::engine_options();
    options.fonts.missing_font = MissingFontPolicy::Error;
    let strict = LabelEngine::new(EngineOptions { fonts: options.fonts });
    let mut label = markup("eight!!!");
    label.options.text.font_family = "UnavailableRegressionFont123".into();
    assert!(matches!(
        strict.raster(&label, 1.0),
        Err(RasterError::Label(LabelError::MissingFont { .. }))
    ));
    assert!(matches!(strict.bounds(&label), Err(LabelError::MissingFont { .. })));
}
