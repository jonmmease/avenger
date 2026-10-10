//! Text marks drawn with their layout options.

use std::ops::Range;

use avenger_common::canvas::CanvasDimensions;
use avenger_common::types::{TextAlign, TextBaseline};
use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_scenegraph::marks::text::SceneTextMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_wgpu::canvas::{Canvas, PngCanvas};

/// The leftmost and rightmost columns with ink in some rows.
fn ink_columns(image: &image::RgbaImage, rows: Range<u32>) -> (u32, u32) {
    let columns: Vec<u32> = (0..image.width())
        .filter(|&x| rows.clone().any(|y| image.get_pixel(x, y).0[0] < 128))
        .collect();
    (columns[0], columns[columns.len() - 1])
}

/// A sign that starts a line hangs left of the label's position, so that a signed number's
/// digits draw where an unsigned one's do.
#[test]
fn hanging_signs_align_numbers_by_their_digits() {
    let render = |hanging_signs| {
        let mark = SceneTextMark {
            len: 2,
            text: ScalarOrArray::new_array(vec!["−12".to_string(), "12".to_string()]),
            x: 40.0.into(),
            y: ScalarOrArray::new_array(vec![10.0, 40.0]),
            align: TextAlign::Left.into(),
            baseline: TextBaseline::Top.into(),
            font_size: 16.0.into(),
            hanging_signs,
            ..Default::default()
        };
        let scene = SceneGraph {
            width: 100.0,
            height: 70.0,
            origin: [0.0; 2],
            marks: vec![SceneGroup {
                marks: vec![mark.into()],
                ..Default::default()
            }
            .into()],
        };
        let mut canvas = pollster::block_on(PngCanvas::new(
            CanvasDimensions {
                size: [100.0, 70.0],
                scale: 2.0,
            },
            Default::default(),
        ))
        .unwrap();
        canvas
            .set_scene(&scene, &avenger_typst_label::bundled_label_engine())
            .unwrap();
        pollster::block_on(canvas.render()).unwrap()
    };
    // Each label's rows at scale 2, and its position's column.
    let (signed, unsigned, position) = (20..60, 80..120, 80);

    let image = render(true);
    let (signed_left, signed_right) = ink_columns(&image, signed.clone());
    let (unsigned_left, unsigned_right) = ink_columns(&image, unsigned.clone());
    assert!(
        signed_left < position,
        "the sign hangs left of the position"
    );
    assert!(unsigned_left >= position);
    assert!(
        signed_right.abs_diff(unsigned_right) <= 1,
        "the digits end at {signed_right} and {unsigned_right}"
    );

    // Without hanging signs, the sign pushes its number's digits right.
    let image = render(false);
    let (signed_left, signed_right) = ink_columns(&image, signed);
    let (_, unsigned_right) = ink_columns(&image, unsigned);
    assert!(signed_left >= position);
    assert!(signed_right > unsigned_right + 4);
}
