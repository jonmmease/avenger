//! Rasterization of drawing items.

use std::sync::Arc;

use avenger_color::AbsoluteColor;

use super::*;
use crate::label::{Point, Stroke};
use crate::typst_svg::{GlyphRef, PathKind};

/// A horizontal rule from (4, 10) to (44, 10) in a 48×20 label.
fn rule(width: f32, dash: Option<DashPattern>) -> SvgItem {
    SvgItem::Path(PathItem {
        path: Curve(vec![
            CurveItem::Move(Point::ZERO),
            CurveItem::Line(Point::new(40.0, 0.0)),
        ]),
        transform: Transform::translate(4.0, 10.0),
        fill: None,
        fill_rule: FillRule::NonZero,
        stroke: Some(Stroke {
            paint: AbsoluteColor::from_srgb(1.0, 0.0, 0.0, 1.0),
            thickness: width,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            dash,
            miter_limit: 4.0,
        }),
        kind: PathKind::Shape,
    })
}

const SIZE: Size = Size::new(48.0, 20.0);

/// The straight-alpha pixel at a point of the label.
fn pixel(raster: &RasterImage, x: f32, y: f32) -> [u8; 4] {
    let x = ((x - raster.origin_x) * raster.scale) as usize;
    let y = ((y - raster.origin_y) * raster.scale) as usize;
    let i = (y * raster.image.width as usize + x) * 4;
    raster.image.data[i..i + 4].try_into().unwrap()
}

#[test]
fn images_and_paths_draw_in_order() {
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, 8, 8);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[0, 0, 0, 255].repeat(64)).unwrap();
    }
    let image = SvgItem::Image(ImageItem {
        data: Arc::from(png),
        size: Size::new(8.0, 8.0),
        transform: Transform::translate(20.0, 6.0),
        glyph: GlyphRef { text: 0, glyph: 0, source: 0..0 },
    });
    let behind = rasterize_items(SIZE, &[rule(4.0, None), image.clone()], 1.0).unwrap();
    let front = rasterize_items(SIZE, &[image, rule(4.0, None)], 1.0).unwrap();
    assert_eq!(pixel(&behind, 24.0, 10.0), [0, 0, 0, 255]);
    assert_eq!(pixel(&front, 24.0, 10.0), [255, 0, 0, 255]);
}

#[test]
fn rule_thickness_scales_once() {
    // Whole physical-pixel widths avoid dependence on subpixel stroke rounding.
    for width in [2.0, 4.0] {
        for scale in [0.5, 1.0, 1.5, 2.0, 3.0] {
            let raster = rasterize_items(SIZE, &[rule(width, None)], scale).unwrap();
            let x = ((24.0 - raster.origin_x) * scale).floor() as usize;
            // Sum coverage through the line, including antialiased edge pixels.
            let physical: f32 = raster
                .image
                .data
                .chunks_exact(raster.image.width as usize * 4)
                .map(|row| f32::from(row[x * 4 + 3]) / 255.0)
                .sum();
            let logical = physical / scale;
            assert!(
                (logical - width).abs() < 0.05,
                "rule width {width} at scale {scale} rendered as {logical}"
            );
        }
    }
}

#[test]
fn rule_dashes_and_phase_scale_once() {
    let dash = DashPattern { array: vec![6.0, 4.0], phase: 2.0 };
    for scale in [1.0, 1.5, 2.0, 3.0] {
        let raster =
            rasterize_items(SIZE, &[rule(4.0, Some(dash.clone()))], scale).unwrap();
        for offset in 0..40 {
            let [.., alpha] = pixel(&raster, 4.0 + offset as f32 + 0.5, 10.0);
            let expected = if (offset + 2) % 10 < 6 { 255 } else { 0 };
            assert_eq!(
                alpha, expected,
                "dash coverage at offset {offset}, scale {scale}"
            );
        }
    }
}

#[test]
fn odd_dash_patterns_repeat() {
    let dash = sk_dash(&DashPattern { array: vec![1.0, 2.0, 3.0], phase: 0.5 });
    assert!(dash.is_some());
}

#[test]
fn invalid_scales_are_errors() {
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert!(rasterize_items(SIZE, &[rule(1.0, None)], scale).is_err());
    }
}

#[test]
fn nothing_to_draw_is_one_transparent_pixel() {
    let raster = rasterize_items(SIZE, &[], 2.0).unwrap();
    assert_eq!((raster.image.width, raster.image.height), (1, 1));
    assert_eq!(raster.image.data, [0, 0, 0, 0]);
    assert_eq!((raster.logical_width, raster.logical_height), (48.0, 20.0));
}
