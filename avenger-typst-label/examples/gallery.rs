mod common;

use avenger_typst_label::{LabelEngine, LabelOptions, RasterOptions, rasterize};

use common::{Canvas, engine_options};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args().nth(1).unwrap_or_else(|| "typst-labels.png".into());
    let engine = LabelEngine::new(engine_options()).with_number_formatting(
        std::sync::Arc::new(avenger_format_number_d3::D3NumberFormatProvider::new()),
    );
    let mut options = LabelOptions::default();
    options.text.font_size = 24.0;
    let lines = [
        "#strong[Single-line typesetting] with _emphasis_ and `raw text`",
        "$sqrt(x^2 + y^2)$, $frac(a + b, c)$ and $display(sum_(i=1)^n i)$",
        "#underline[Measured once], #strike[old value], H#sub[2]O and #text(fill: teal)[color]",
        "$sqrt(frac(1, x^2))_n^m$, $hat(hat(x))$ and $underbrace(x + y, n)$",
        "abc #underline[אבג 123] xyz, $\"हिन्दी\"$, $cal(A B C)$ and $scr(A B C)$",
    ];
    let (width, height) = (1440usize, 850usize);
    let mut canvas = Canvas::new(width, height);
    for (row, source) in lines.iter().enumerate() {
        let label = engine.compile(source, &options)?;
        let raster = rasterize(&label, &RasterOptions { scale: 2.0 })?;
        let image = raster.image;
        let (left, top) = (48usize, 50 + row * 155);
        if left + image.width as usize > width || top + image.height as usize > height {
            return Err("label does not fit gallery canvas".into());
        }
        canvas.draw(&image, left as i64, top as i64);
    }
    canvas.save(&output)?;
    println!("Wrote {output}");
    Ok(())
}
