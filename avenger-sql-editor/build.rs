//! Embeds the sample project: writes to `OUT_DIR/samples.rs` the path of each file of `samples/`
//! in the project, with its bytes through `include_bytes!`. A link in the folder, such as
//! `data/files/geo`, which links to avenger-guides-geo's geographic files, is followed. Hidden
//! files, whose names start with `.`, are not part of the project.

use std::path::{Path, PathBuf};

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let samples = manifest.join("samples");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", samples.display());
    let mut files = Vec::new();
    walk(&samples, "", &mut files);
    files.sort();
    let mut code = String::from("&[\n");
    for (path, file) in files {
        code.push_str(&format!(
            "    ({path:?}, include_bytes!({:?}) as &[u8]),\n",
            file.display().to_string()
        ));
    }
    code.push_str("]\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("samples.rs");
    std::fs::write(out, code).unwrap();
}

/// Adds each file under `directory`, whose path in the project starts with `prefix`, to `files`,
/// with the file that it is after links.
fn walk(directory: &Path, prefix: &str, files: &mut Vec<(String, PathBuf)>) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().into_string().unwrap();
        if name.starts_with('.') {
            continue;
        }
        let path = format!("{prefix}{name}");
        let file = entry.path().canonicalize().unwrap();
        if entry.file_type().unwrap().is_symlink() {
            println!("cargo:rerun-if-changed={}", file.display());
        }
        match file.is_dir() {
            true => walk(&file, &format!("{path}/"), files),
            false => files.push((path, file)),
        }
    }
}
