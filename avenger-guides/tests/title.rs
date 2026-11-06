//! Titles measure their text with the label engine and stand on any side of a chart.

use std::sync::Arc;

use avenger_color::ColorOrGradient;
use avenger_common::types::{TextAlign, TextSyntaxMode};
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_guides::{
    error::AvengerGuidesError,
    title::{Title, TitleAlign, TitleConfig},
    GuideSide,
};
use avenger_render_fixtures::raster::{assert_baseline, svg_to_png};
use avenger_scenegraph::{
    marks::{
        group::SceneGroup,
        mark::SceneMark,
        rect::SceneRectMark,
        text::{text_label, text_style},
    },
    scene_graph::SceneGraph,
};
use avenger_svg::SvgRenderer;
use avenger_typst_label::{LabelEngine, LabelOptions};

const TITLE: &str = "Penguins of the Palmer Archipelago";
const SUBTITLE: &str = "Body mass by flipper length";

const SIDES: [GuideSide; 4] = [
    GuideSide::Top,
    GuideSide::Bottom,
    GuideSide::Left,
    GuideSide::Right,
];

fn engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
}

fn config(side: GuideSide, align: TitleAlign) -> TitleConfig {
    TitleConfig {
        side,
        align,
        ..Default::default()
    }
}

/// The width and height of `text` in the configuration's title style.
fn measure(text: &str, config: &TitleConfig, subtitle: bool) -> (f32, f32) {
    let style = if subtitle {
        &config.subtitle
    } else {
        &config.title
    };
    let options = LabelOptions {
        text: text_style(
            &style.font_family,
            style.font_size,
            style.font_weight,
            style.font_style,
            style.color,
        ),
        ..Default::default()
    };
    let bounds = engine()
        .bounds(&text_label(text, style.syntax, options))
        .unwrap();
    (bounds.width, bounds.height)
}

/// Each text mark's text, position, alignment and angle.
fn texts(group: &SceneGroup) -> Vec<(String, [f32; 2], TextAlign, f32)> {
    group
        .marks
        .iter()
        .map(|mark| {
            let SceneMark::Text(text) = mark else {
                panic!("a text mark");
            };
            let first = |values: Vec<f32>| values[0];
            (
                text.text.as_vec(1, None).remove(0),
                [first(text.x.as_vec(1, None)), first(text.y.as_vec(1, None))],
                text.align.as_vec(1, None)[0],
                first(text.angle.as_vec(1, None)),
            )
        })
        .collect()
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn the_thickness_is_the_title_the_gap_and_the_subtitle_at_every_length() {
    let config = TitleConfig::default();
    let (title_width, title_height) = measure(TITLE, &config, false);
    let (_, subtitle_height) = measure(SUBTITLE, &config, true);
    let alone = Title::new(TITLE, None, &config, &engine()).unwrap();
    let both = Title::new(TITLE, Some(SUBTITLE), &config, &engine()).unwrap();
    for length in [600.0, 300.0, 100.0] {
        assert!(close(alone.extent(length).thickness, title_height));
        assert!(close(
            both.extent(length).thickness,
            title_height + config.gap + subtitle_height
        ));
    }
    // A title that fits holds from the shortest length that fits its longest line.
    let fits = alone.extent(600.0);
    assert_eq!(fits.overhang, [0.0; 2]);
    assert_eq!(*fits.holds.end(), f32::INFINITY);
    assert!((title_width - fits.holds.start()).abs() < 2e-3);
}

#[test]
fn a_long_title_overhangs_the_side_its_alignment_leaves_open() {
    let (width, _) = measure(TITLE, &TitleConfig::default(), false);
    let length = width - 50.0;
    let overhang = |side, align| {
        let title = Title::new(TITLE, None, &config(side, align), &engine()).unwrap();
        let extent = title.extent(length);
        assert_eq!(extent.holds, length..=length);
        extent.overhang
    };
    let near = |[a, b]: [f32; 2], [c, d]: [f32; 2]| close(a, c) && close(b, d);
    for side in [GuideSide::Top, GuideSide::Bottom, GuideSide::Right] {
        assert!(near(overhang(side, TitleAlign::Start), [0.0, 50.0]));
        assert!(near(overhang(side, TitleAlign::Center), [25.0, 25.0]));
        assert!(near(overhang(side, TitleAlign::End), [50.0, 0.0]));
    }
    // A title on the left reads up, from the slab's end.
    assert!(near(
        overhang(GuideSide::Left, TitleAlign::Start),
        [50.0, 0.0]
    ));
    assert!(near(
        overhang(GuideSide::Left, TitleAlign::End),
        [0.0, 50.0]
    ));
}

#[test]
fn lines_align_along_the_reading_direction_on_each_side() {
    let length = 500.0;
    for side in SIDES {
        for (align, reading, text_align) in [
            (TitleAlign::Start, 0.0, TextAlign::Left),
            (TitleAlign::Center, length / 2.0, TextAlign::Center),
            (TitleAlign::End, length, TextAlign::Right),
        ] {
            let title = Title::new(TITLE, Some(SUBTITLE), &config(side, align), &engine()).unwrap();
            let along = if side == GuideSide::Left {
                length - reading
            } else {
                reading
            };
            for (text, point, drawn_align, angle) in texts(&title.part(length).unwrap()) {
                let (drawn_along, _) = side.along_across(point);
                assert!(close(drawn_along, along), "{side:?} {align:?} {text}");
                assert_eq!(drawn_align, text_align, "{side:?} {align:?}");
                let expected = match side {
                    GuideSide::Left => -90.0,
                    GuideSide::Right => 90.0,
                    _ => 0.0,
                };
                assert_eq!(angle, expected);
            }
        }
    }
}

#[test]
fn the_title_reads_above_the_subtitle_on_each_side() {
    let config = TitleConfig::default();
    let (_, title_height) = measure(TITLE, &config, false);
    let (_, subtitle_height) = measure(SUBTITLE, &config, true);
    let thickness = title_height + config.gap + subtitle_height;
    for side in SIDES {
        let title = Title::new(
            TITLE,
            Some(SUBTITLE),
            &self::config(side, TitleAlign::Start),
            &engine(),
        )
        .unwrap();
        let texts = texts(&title.part(400.0).unwrap());
        let across: Vec<f32> = texts
            .iter()
            .map(|(_, point, _, _)| side.along_across(*point).1)
            .collect();
        let (title_middle, subtitle_middle) = if side == GuideSide::Bottom {
            (
                title_height / 2.0,
                title_height + config.gap + subtitle_height / 2.0,
            )
        } else {
            (thickness - title_height / 2.0, subtitle_height / 2.0)
        };
        assert_eq!(texts[0].0, TITLE);
        assert!(close(across[0], title_middle), "{side:?} {across:?}");
        assert!(close(across[1], subtitle_middle), "{side:?} {across:?}");
    }
}

#[test]
fn the_origin_is_the_inner_edge_at_the_start_on_each_side() {
    let slot = [10.0, 20.0, 300.0, 40.0];
    for (side, origin) in [
        (GuideSide::Top, [10.0, 60.0]),
        (GuideSide::Bottom, [10.0, 20.0]),
        (GuideSide::Left, [310.0, 20.0]),
        (GuideSide::Right, [10.0, 20.0]),
    ] {
        let title = Title::new(TITLE, None, &config(side, TitleAlign::Center), &engine()).unwrap();
        assert_eq!(title.origin(slot), origin, "{side:?}");
    }
}

#[test]
fn markup_and_plain_lines_measure_in_their_syntax() {
    let mut config = TitleConfig::default();
    let plain = Title::new("Mass *by* sex", None, &config, &engine()).unwrap();
    config.title.syntax = TextSyntaxMode::TypstMarkup;
    let markup = Title::new("Mass *by* sex", None, &config, &engine()).unwrap();
    // Markup drops the asterisks that plain text shows.
    assert!(markup.extent(600.0).holds.start() < plain.extent(600.0).holds.start());
    // Invalid markup measures as its literal text, as every label draws it.
    let invalid = Title::new("#(1 +", None, &config, &engine()).unwrap();
    config.title.syntax = TextSyntaxMode::Plain;
    let literal = Title::new("#(1 +", None, &config, &engine()).unwrap();
    assert_eq!(invalid.extent(600.0), literal.extent(600.0));

    config.title.syntax = TextSyntaxMode::PlainLines;
    let one = Title::new("Penguins", None, &config, &engine()).unwrap();
    let two = Title::new("Penguins\nof Palmer", None, &config, &engine()).unwrap();
    assert!(two.extent(600.0).thickness > 1.8 * one.extent(600.0).thickness);
    config.title.syntax = TextSyntaxMode::Plain;
    let spaced = Title::new("Penguins\nof Palmer", None, &config, &engine()).unwrap();
    assert_eq!(spaced.extent(600.0).thickness, one.extent(600.0).thickness);
}

#[test]
fn a_title_without_text_takes_no_space() {
    let title = Title::new("", None, &TitleConfig::default(), &engine()).unwrap();
    let extent = title.extent(300.0);
    assert_eq!((extent.thickness, extent.overhang), (0.0, [0.0; 2]));
    assert_eq!(extent.holds, 0.0..=f32::INFINITY);
    assert!(title.part(300.0).is_none());
    let gap = TitleConfig {
        gap: -2.0,
        ..Default::default()
    };
    assert!(matches!(
        Title::new(TITLE, None, &gap, &engine()),
        Err(AvengerGuidesError::TitleGap(_))
    ));
}

/// A plot outline with a title and a subtitle on each side, aligned to the start on the top,
/// the center on the bottom and the left, and the end on the right.
#[test]
fn titles_on_four_sides_match_the_baseline() {
    let engine = engine();
    let plot = [360.0, 280.0];
    let margin = 60.0;
    let size = [plot[0] + 2.0 * margin, plot[1] + 2.0 * margin];
    let mut marks: Vec<SceneMark> = vec![SceneRectMark {
        len: 1,
        x: margin.into(),
        y: margin.into(),
        width: Some(plot[0].into()),
        height: Some(plot[1].into()),
        fill: ColorOrGradient::Color([0.94, 0.94, 0.94, 1.0]).into(),
        stroke: ColorOrGradient::Color([0.6, 0.6, 0.6, 1.0]).into(),
        stroke_width: 1.0.into(),
        ..Default::default()
    }
    .into()];
    for (side, align) in [
        (GuideSide::Top, TitleAlign::Start),
        (GuideSide::Bottom, TitleAlign::Center),
        (GuideSide::Left, TitleAlign::Center),
        (GuideSide::Right, TitleAlign::End),
    ] {
        let title = Title::new(TITLE, Some(SUBTITLE), &config(side, align), &engine).unwrap();
        let length = if side.is_vertical() { plot[1] } else { plot[0] };
        let gap = 8.0;
        let thickness = title.extent(length).thickness + gap;
        // The slab beside the plot, a gap away from it.
        let slot = match side {
            GuideSide::Top => [margin, margin - thickness, plot[0], thickness - gap],
            GuideSide::Bottom => [margin, margin + plot[1] + gap, plot[0], thickness - gap],
            GuideSide::Left => [margin - thickness, margin, thickness - gap, plot[1]],
            GuideSide::Right => [margin + plot[0] + gap, margin, thickness - gap, plot[1]],
        };
        let mut group = title.part(length).unwrap();
        group.origin = title.origin(slot);
        marks.push(group.into());
    }
    let scene = SceneGraph {
        marks,
        width: size[0],
        height: size[1],
        origin: [0.0, 0.0],
    };
    let svg = SvgRenderer::new()
        .render_scene_graph(&scene, &engine)
        .unwrap();
    let png = svg_to_png(&svg, 2.0);
    assert_baseline(env!("CARGO_MANIFEST_DIR"), &png, "title-sides");
}
