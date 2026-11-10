//! The surface of the constructors of the `functions` feature: every constructor's parameters and
//! the inputs that each accepts match their snapshot. The tests of their SQL functions are in
//! avenger-sql.

use std::path::Path;

use avenger_chart::functions::surface;

#[test]
fn the_sql_surface_matches_its_snapshot() {
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let output = tests.join("output").join("constructors.txt");
    std::fs::create_dir_all(output.parent().unwrap()).unwrap();
    let surface = surface();
    std::fs::write(&output, &surface).unwrap();
    let baseline = tests.join("baselines").join("constructors.txt");
    let expected = std::fs::read_to_string(&baseline).unwrap_or_default();
    assert!(
        expected == surface,
        "the constructors changed; to accept {}, copy it to {}",
        output.display(),
        baseline.display()
    );
}
