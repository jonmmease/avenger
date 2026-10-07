//! Renders multi-line labels, each under a caption and with its box outlined: explicit breaks,
//! the width modes, wrapping, alignment, line limits, line heights, hanging signs and newline
//! breaks.
//!
//! ```sh
//! cargo run --release -p avenger-typst-label --features raster --example multiline -- \
//!   docs/images/typst-multiline-labels.png
//! ```

mod common;

use std::num::NonZeroUsize;

use avenger_color::AbsoluteColor;
use avenger_typst_label::{
    CompiledLabel, LabelAlign, LabelEngine, LabelLineHeight, LabelOptions, LabelWidth,
    RasterOptions, TextDir, rasterize,
};

use common::{Canvas, engine_options};

/// Pixels per point.
const SCALE: f32 = 2.0;

/// The outline of a label's box.
const BOX: [u8; 4] = [70, 130, 200, 160];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "multiline-labels.png".into());
    let engine = LabelEngine::new(engine_options());

    let label = |width, align, max_lines: usize, ellipsis| LabelOptions {
        width,
        align,
        max_lines: NonZeroUsize::new(max_lines),
        ellipsis,
        ..LabelOptions::default()
    };
    let rtl = |options: LabelOptions| {
        let mut options = options;
        options.text.dir = TextDir::Rtl;
        options
    };
    let hanging = |options: LabelOptions| LabelOptions { hanging_signs: true, ..options };
    let unwrapped = |options: LabelOptions| LabelOptions { wrap: false, ..options };
    let spaced =
        |line_height, options: LabelOptions| LabelOptions { line_height, ..options };
    let newlines =
        |options: LabelOptions| LabelOptions { newline_breaks: true, ..options };
    use LabelAlign::{Center, End, Right, Start};
    use LabelLineHeight::{Fixed as Distance, Relative};
    use LabelWidth::{Auto, Fixed, Max};
    let long = "Revenue by region in millions of dollars";
    let hebrew = "שלום עולם זה טקסט ארוך מאוד";
    let numbers = "1,234.5 \\ −1,234.5 \\ +1,234.5 \\ ±1,234.5";
    let explicit = "Revenue by region \\ in millions of dollars \\ USD";
    let three = "Revenue \\ (millions) \\ by region";
    let math = "Revenue \\ ratio $sqrt(x^2 + y^2)$ \\ by region";
    let cases = [
        ("Breaks, start", "Revenue \\ (millions of USD)", label(Auto, Start, 0, false)),
        ("Breaks, center", "Revenue \\ (millions of USD)", label(Auto, Center, 0, false)),
        ("Max(150)", long, label(Max(150.0), Start, 0, false)),
        ("Fixed(150)", long, label(Fixed(150.0), Start, 0, false)),
        ("Fixed(150), center", long, label(Fixed(150.0), Center, 0, false)),
        ("Fixed(150), right", long, label(Fixed(150.0), Right, 0, false)),
        (
            "Fixed(150), right to left, start",
            hebrew,
            rtl(label(Fixed(150.0), Start, 0, false)),
        ),
        (
            "Fixed(150), right to left, end",
            hebrew,
            rtl(label(Fixed(150.0), End, 0, false)),
        ),
        ("Max(150), 1 line, ellipsis", long, label(Max(150.0), Start, 1, true)),
        ("Max(80), 2 lines, ellipsis", long, label(Max(80.0), Start, 2, true)),
        (
            "Max(70), ellipsis, overfull word",
            "Internationalization of labels",
            label(Max(70.0), Start, 0, true),
        ),
        (
            "Max(90), right to left, 1 line, ellipsis",
            hebrew,
            rtl(label(Max(90.0), Start, 1, true)),
        ),
        (
            "Max(100), math",
            "Fit $y = 2.5 x + 7$ to the data",
            label(Max(100.0), Start, 0, false),
        ),
        (
            "Fixed(150), justified break",
            "Revenue by #linebreak(justify: true) region",
            label(Fixed(150.0), Start, 0, false),
        ),
        (
            "Max(80), soft hyphens",
            "Inter-?national-?ization",
            label(Max(80.0), Start, 0, false),
        ),
        (
            "Max(110), bidirectional",
            "Revenue שלום עולם טקסט growth",
            label(Max(110.0), Start, 0, false),
        ),
        ("Signs", numbers, label(Auto, Start, 0, false)),
        ("Hanging signs", numbers, hanging(label(Auto, Start, 0, false))),
        (
            "Max(120), no wrap, ellipsis",
            explicit,
            unwrapped(label(Max(120.0), Start, 0, true)),
        ),
        (
            "Fixed(120), no wrap, center, ellipsis",
            explicit,
            unwrapped(label(Fixed(120.0), Center, 0, true)),
        ),
        ("Line height Auto", three, label(Auto, Start, 0, false)),
        (
            "Line height Fixed(26)",
            three,
            spaced(Distance(26.0), label(Auto, Start, 0, false)),
        ),
        ("Math, line height Auto", math, label(Auto, Start, 0, false)),
        (
            "Math, Relative(1.1)",
            math,
            spaced(Relative(1.1), label(Auto, Start, 0, false)),
        ),
        (
            "Newline breaks, literal text",
            "Revenue\n(millions of USD)",
            newlines(label(Auto, Start, 0, false)),
        ),
        (
            "Newline breaks, no wrap, ellipsis",
            "Revenue by region\nin millions of dollars",
            newlines(unwrapped(label(Max(120.0), Start, 0, true))),
        ),
    ];

    let mut caption = LabelOptions::default();
    caption.text.font_size = 9.0;
    caption.text.fill = AbsoluteColor::from_srgb(0.45, 0.45, 0.45, 1.0);
    let mut panels = vec![];
    for (title, source, mut options) in cases {
        options.text.font_size = 14.0;
        // Newline breaks apply to literal text.
        let label = if options.newline_breaks {
            engine.compile_text(source, &options)?
        } else {
            engine.compile(source, &options)?
        };
        panels.push([engine.compile_text(title, &caption)?, label]);
    }

    // Two columns of panels, each a caption over its label.
    let (margin, column, gap) = (40, 520, 8);
    let px = |pt: f32| (pt * SCALE).round() as i64;
    let rows: Vec<i64> = panels
        .chunks(2)
        .map(|row| {
            let height = |panel: &[CompiledLabel; 2]| {
                px(panel[0].metrics.height + panel[1].metrics.height)
            };
            row.iter().map(height).max().unwrap_or(0) + 3 * gap + 24
        })
        .collect();
    let mut canvas = Canvas::new(
        (2 * margin + 2 * column) as usize,
        (2 * margin + rows.iter().sum::<i64>()) as usize,
    );
    let mut top = margin;
    for (row, height) in panels.chunks(2).zip(&rows) {
        for (i, panel) in row.iter().enumerate() {
            let left = margin + i as i64 * column + 30;
            let mut y = top;
            for (j, label) in panel.iter().enumerate() {
                if j == 1 {
                    let (width, height) =
                        (px(label.metrics.width), px(label.metrics.height));
                    canvas.outline(left, y, left + width, y + height, BOX);
                }
                let raster = rasterize(label, &RasterOptions { scale: SCALE })?;
                canvas.draw(
                    &raster.image,
                    left + px(raster.origin_x),
                    y + px(raster.origin_y),
                );
                y += px(label.metrics.height) + gap;
            }
        }
        top += height;
    }
    canvas.save(&output)?;
    println!("Wrote {output}");
    Ok(())
}
