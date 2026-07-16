use std::{fs, path::PathBuf};

#[test]
fn core_manifest_excludes_runtime_and_rendering_dependencies() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest = fs::read_to_string(manifest).expect("read avenger-lang-core manifest");
    let forbidden = [
        "avenger-chart =",
        "avenger-chart-core =",
        "avenger-chart-lang-registry =",
        "avenger-app =",
        "avenger-wgpu =",
        "datafusion =",
        "arrow =",
    ];

    for dependency in forbidden {
        assert!(
            !manifest
                .lines()
                .any(|line| line.trim().starts_with(dependency)),
            "avenger-lang-core must not depend on {dependency}"
        );
    }
}
