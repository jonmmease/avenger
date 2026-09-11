use avenger_color::{
    interpolate_colors, parse_color_string_strict, ColorInterpolationSpace, OklabMixer,
};
use std::{error::Error, fs::File, io::BufWriter};

fn main() -> Result<(), Box<dyn Error>> {
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "color-interpolation.png".into());
    let (width, height) = (800_u32, 352_u32);
    let mut pixels = vec![255_u8; (width * height * 4) as usize];
    let colors = [
        parse_color_string_strict("#713bc1")?,
        parse_color_string_strict("#f6c744")?,
    ];
    let values: Vec<f32> = (0..736).map(|x| x as f32 / 735.0).collect();
    let mut rows = [
        ColorInterpolationSpace::Srgba,
        ColorInterpolationSpace::Hsla,
        ColorInterpolationSpace::Laba,
    ]
    .into_iter()
    .map(|space| interpolate_colors(space, &colors, &values))
    .collect::<Result<Vec<_>, _>>()?;
    let mixer = OklabMixer::new(&colors);
    rows.push(
        values
            .iter()
            .map(|&t| {
                let c = mixer.mix(&[1.0 - t, t]).expect("positive weights");
                [c[0], c[1], c[2], 1.0]
            })
            .collect(),
    );
    for (row, colors) in rows.iter().enumerate() {
        for y in (32 + row * 80)..(32 + row * 80 + 48) {
            for (x, color) in colors.iter().enumerate() {
                let offset = (y * width as usize + x + 32) * 4;
                for channel in 0..3 {
                    pixels[offset + channel] =
                        (color[channel].clamp(0.0, 1.0) * 255.0).round() as u8;
                }
            }
        }
    }
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(&output)?), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&pixels)?;
    println!("Saved {output}: sRGB, HSL, Lab, and Oklab from top to bottom");
    Ok(())
}
