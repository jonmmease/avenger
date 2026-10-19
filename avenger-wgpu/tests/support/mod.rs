//! A baseline check that the GPU tests share.

use std::path::Path;

use dssim::Dssim;

/// Asserts that `image` matches `tests/baselines/{name}.png`, and writes it to the ignored
/// `tests/output/{name}.png`, so that an intended change is accepted by copying it.
pub fn assert_baseline(image: &image::RgbaImage, name: &str) {
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let output = tests.join(format!("output/{name}.png"));
    std::fs::create_dir_all(output.parent().unwrap()).unwrap();
    image.save(&output).unwrap();
    let baseline = tests.join(format!("baselines/{name}.png"));
    let attr = Dssim::new();
    let expected = dssim::load_image(&attr, &baseline).expect("the baseline exists");
    let actual = dssim::load_image(&attr, &output).unwrap();
    let (diff, _) = attr.compare(&expected, actual);
    assert!(
        diff < 0.0001,
        "DSSIM {diff}; to accept {}, copy it to {}",
        output.display(),
        baseline.display()
    );
}
