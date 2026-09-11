use std::{io::Read, sync::Arc};

use avenger_typst_label::{EngineOptions, MathFontBytesId, RegisteredFont};

fn engine_options() -> EngineOptions {
    let mut options = EngineOptions::default();
    options.fonts.load_system_fonts = false;
    options.fonts.default_sans_serif_family = Some("Lato".into());
    options.fonts.default_monospace_family = Some("DejaVu Sans Mono".into());
    options.fonts.default_math_family = Some("Lete Sans Math".into());
    options.fonts.registered_fonts = FONT_BYTES
        .iter()
        .enumerate()
        .map(|(index, compressed)| {
            let mut bytes = Vec::new();
            brotli::Decompressor::new(*compressed, 4096)
                .read_to_end(&mut bytes)
                .expect("fixture font should decompress");
            RegisteredFont::new(MathFontBytesId(index as u64 + 1), Arc::<[u8]>::from(bytes))
        })
        .collect();
    options
}

const FONT_BYTES: &[&[u8]] = &[
    include_bytes!("../../avenger-text/fonts/Lato/Lato-Light.ttf.br"),
    include_bytes!("../../avenger-text/fonts/Lato/Lato-Italic.ttf.br"),
    include_bytes!("../../avenger-text/fonts/Lato/Lato-Medium.ttf.br"),
    include_bytes!("../../avenger-text/fonts/Lato/Lato-Bold.ttf.br"),
    include_bytes!("../../avenger-text/fonts/DejaVu_Sans_Mono/DejaVuSansMono.ttf.br"),
    include_bytes!("../../avenger-text/fonts/Lete_Sans_Math/LeteSansMath.otf.br"),
    include_bytes!("../../avenger-text/fonts/Lete_Sans_Math/LeteSansMath-Bold.otf.br"),
];

use avenger_typst_label::{LabelEngine, LabelOptions, RasterOptions, rasterize};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "typst-labels.png".into());
    let engine = LabelEngine::new(engine_options())?;
    let mut options = LabelOptions::default();
    options.text.font_size = 24.0;
    options.math.font_size = 24.0;
    let lines = [
        "#strong[Single-line typesetting] with _emphasis_",
        "$sqrt(x^2 + y^2)$   $frac(a + b, c)$   $sum_(i=1)^n i$",
        "#underline[Measured once]   #strike[old value]   #super[annotation]",
    ];
    let (width, height) = (1440usize, 520usize);
    let mut pixels = vec![255u8; width * height * 4];
    for (row, source) in lines.iter().enumerate() {
        let label = engine.compile(source, &options)?;
        let raster = rasterize(&label, &RasterOptions { scale: 2.0 })?;
        let image = raster.image;
        let (left, top) = (48usize, 50 + row * 155);
        if left + image.width as usize > width || top + image.height as usize > height {
            return Err("label does not fit gallery canvas".into());
        }
        for y in 0..image.height as usize {
            for x in 0..image.width as usize {
                let src = (y * image.width as usize + x) * 4;
                let dst = ((top + y) * width + left + x) * 4;
                let alpha = image.data[src + 3] as u16;
                for channel in 0..3 {
                    pixels[dst + channel] =
                        ((image.data[src + channel] as u16 * alpha + 255 * (255 - alpha) + 127)
                            / 255) as u8;
                }
            }
        }
    }
    if let Some(parent) = std::path::Path::new(&output)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::File::create(&output)?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&pixels)?;
    println!("Wrote {output}");
    Ok(())
}
