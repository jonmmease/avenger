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

#[test]
fn chart_crates_do_not_depend_on_language_crates() {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    for entry in fs::read_dir(&workspace).expect("read workspace") {
        let entry = entry.expect("read workspace entry");
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("avenger-chart") {
            continue;
        }
        let manifest_path = entry.path().join("Cargo.toml");
        if !manifest_path.is_file() {
            continue;
        }
        let manifest = fs::read_to_string(&manifest_path).expect("read chart manifest");
        assert!(
            !manifest
                .lines()
                .any(|line| line.trim_start().starts_with("avenger-lang")),
            "{} must not depend on an avenger-lang crate",
            manifest_path.display()
        );
    }
}
