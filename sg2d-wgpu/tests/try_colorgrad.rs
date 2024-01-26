use colorgrad::Color;

#[test]
fn test_try_it() {
    let mut binding = colorgrad::CustomGradient::new();
    let builder = binding.domain(&[0.0, 0.1, 1.0]).colors(&[
        Color::new(0.0, 0.0, 0.0, 1.0),
        Color::new(0.5, 0.5, 0.5, 1.0),
        Color::new(1.0, 1.0, 1.0, 1.0),
    ]);
    let b = builder.build().unwrap();
    let c = b.at(0.5);

    for i in 0..256 {
        let p = (i as f64) / 255.0;
        println!("{i} - {p} - {:?}", b.at(p))
    }

    // println!("{c:?}");
}
