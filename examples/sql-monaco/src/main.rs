//! Writes the editor's files into `site/`: the faces that `avenger-fonts` bundles as
//! `fonts/<FILE>`, which the page adds, since the library embeds no fonts; the files of the Vega
//! project of avenger-sql-project's tests as `project/<PATH>`, which the page opens as a project
//! in memory; and `manifest.json`, with the fonts' files in the order that the page adds them and
//! the paths of the project's files.
//!
//! Usage: `build-site [SITE_DIR]`, where `SITE_DIR` is the example's `site` by default.

use std::path::{Path, PathBuf};

fn files(root: &Path, at: &Path, paths: &mut Vec<String>) {
    let mut entries: Vec<_> = std::fs::read_dir(at)
        .expect("the project's directory reads")
        .map(|entry| entry.expect("an entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            files(root, &path, paths);
        } else {
            let relative = path.strip_prefix(root).expect("a file of the project");
            paths.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
}

fn main() {
    let site = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("site"));
    let fonts = site.join("fonts");
    std::fs::create_dir_all(&fonts).expect("the fonts directory");
    let mut font_files = Vec::new();
    for (file, face) in avenger_fonts::FACES {
        std::fs::write(fonts.join(file), avenger_fonts::decompress(face))
            .expect("the font's file writes");
        font_files.push(format!("fonts/{file}"));
    }
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../avenger-sql-project/tests/projects/vega");
    let mut paths = Vec::new();
    files(&source, &source, &mut paths);
    for path in &paths {
        let to = site.join("project").join(path);
        std::fs::create_dir_all(to.parent().expect("a parent")).expect("the file's directory");
        std::fs::copy(source.join(path), &to).expect("the project's file copies");
    }
    println!("{} files of the project", paths.len());
    let manifest = serde_json::json!({ "fonts": font_files, "files": paths });
    let manifest = serde_json::to_string_pretty(&manifest).expect("the manifest encodes");
    std::fs::write(site.join("manifest.json"), manifest).expect("the manifest writes");
}
