//! The runtime of the interactive charts of Quarto 2 pages. The binary embeds the WebAssembly
//! library of avenger-query-web as gzip, which holds avenger-chart-web's library and runs the
//! documents' graphs, its glue, and avenger-chart-web's `<avenger-chart>` element, which
//! `quarto2/scripts/build-runtime.sh` builds as one snapshot, and the engine writes them, with the
//! faces of `avenger-fonts` as gzip, to a project's `_avenger/runtime/`, which q2 copies to the
//! output with the project's resources. A page's header holds the loader and the page's
//! configuration, and imports the runtime when its first chart nears the screen, with the runtime's
//! hash in each URL, so a browser never reads a cached file of another build, and a page that an
//! older avenger rendered reads the newest runtime.

use std::{
    io::Write,
    path::{Component, Path, PathBuf},
};

use avenger_style::ColorScheme;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// The files of the runtime, by their names in its folder.
const FILES: &[(&str, &[u8])] = &[
    (
        "avenger_query_web_bg.wasm.gz",
        include_bytes!("../quarto2/runtime/avenger_query_web_bg.wasm.gz"),
    ),
    (
        "avenger_query_web.js",
        include_bytes!("../quarto2/runtime/avenger_query_web.js"),
    ),
    (
        "avenger-chart.js",
        include_bytes!("../quarto2/runtime/avenger-chart.js"),
    ),
];

/// The loader that a page's header holds.
const LOADER: &str = include_str!("../quarto2/loader.js");

/// The folder of the runtime in a project, which `quarto2-init` adds to the project's resources.
pub(crate) const FOLDER: &str = "_avenger";

/// The folder of `_avenger/` that holds the runtime, of the newest build that wrote it.
const RUNTIME: &str = "runtime";

/// The file of the runtime's folder that holds its hash, which the runtime's files write before.
const HASH_FILE: &str = "HASH";

/// The hash of the runtime: the first 16 hexadecimal digits of the SHA-256 of its files and its
/// fonts.
pub(crate) fn hash() -> &'static str {
    static HASH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HASH.get_or_init(|| {
        let mut hasher = Sha256::new();
        for (name, bytes) in FILES.iter().chain(avenger_fonts::FACES) {
            hasher.update(name.as_bytes());
            hasher.update(bytes);
        }
        let digest = hasher.finalize();
        digest[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    })
}

/// Writes the runtime to `<project>/_avenger/runtime/`, unless the folder holds the runtime of this
/// build already, and returns the folder. It never rewrites a current folder, since
/// `q2 preview --static` renders again on every change of the project's files. The files go to a
/// folder of a temporary name that takes the place of the folder of another build, so engines that
/// q2 runs at once for several documents never leave or read a folder half written. It removes
/// the other folders of `_avenger/` but the documents' graphs.
pub(crate) fn write(project: &Path) -> Result<PathBuf, String> {
    let error = |error: std::io::Error| format!("the runtime of interactive charts: {error}");
    let root = project.join(FOLDER);
    let folder = root.join(RUNTIME);
    let hash = hash();
    let current = |folder: &Path| {
        std::fs::read_to_string(folder.join(HASH_FILE)).is_ok_and(|text| text.trim() == hash)
    };
    if !current(&folder) {
        let id = std::process::id();
        let temporary = root.join(format!(".{RUNTIME}-{id}"));
        let _ = std::fs::remove_dir_all(&temporary);
        std::fs::create_dir_all(temporary.join("fonts")).map_err(error)?;
        for (name, bytes) in FILES {
            std::fs::write(temporary.join(name), bytes).map_err(error)?;
        }
        for (name, face) in avenger_fonts::FACES {
            let mut encoder =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
            encoder
                .write_all(&avenger_fonts::decompress(face))
                .map_err(error)?;
            let gzip = encoder.finish().map_err(error)?;
            std::fs::write(temporary.join("fonts").join(format!("{name}.gz")), gzip)
                .map_err(error)?;
        }
        std::fs::write(temporary.join(HASH_FILE), hash).map_err(error)?;
        // The folder of another build moves aside, and this one takes its place.
        let aside = root.join(format!(".old-{id}"));
        let moved = std::fs::rename(&folder, &aside).is_ok();
        let placed = std::fs::rename(&temporary, &folder);
        if moved {
            let _ = std::fs::remove_dir_all(&aside);
        }
        if placed.is_err() {
            let _ = std::fs::remove_dir_all(&temporary);
            // Another engine put the folder in place first.
            if !current(&folder) {
                return Err(format!(
                    "the runtime of interactive charts: {} did not write",
                    folder.display()
                ));
            }
        }
    }
    for entry in std::fs::read_dir(&root).map_err(error)?.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name != RUNTIME && name != GRAPHS && !name.starts_with('.') && entry.path().is_dir() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
    Ok(folder)
}

/// The folder of the documents' graphs in `_avenger/`, beside the runtime.
const GRAPHS: &str = "graphs";

/// Writes the graph's file `bytes` of a document's page to `<project>/_avenger/graphs/`, under a
/// name of its content, unless the file is there, and returns the name. A file of the same name
/// holds the same graph, so `q2 preview --static`, which renders again on every change of the
/// project's files, finds the file in place.
pub(crate) fn write_graph(project: &Path, bytes: &[u8]) -> Result<String, String> {
    let error = |error: std::io::Error| format!("the graph of the page: {error}");
    let folder = project.join(FOLDER).join(GRAPHS);
    let digest = Sha256::digest(bytes);
    let name: String = digest[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let name = format!("{name}.graph");
    let file = folder.join(&name);
    if !file.exists() {
        std::fs::create_dir_all(&folder).map_err(error)?;
        let temporary = folder.join(format!(".{name}-{}", std::process::id()));
        std::fs::write(&temporary, bytes).map_err(error)?;
        std::fs::rename(&temporary, &file).map_err(error)?;
    }
    Ok(name)
}

/// The header of the page of the document at `document`, whose runtime is at `folder`: the page's
/// configuration, with the runtime's hash as its version, and the loader, with a rule that keeps an
/// element that has not loaded from taking room. `schemes` are the color schemes of the document's themes, and `styles` the project's
/// stylesheets, `{ entry, files }`, or none.
pub(crate) fn header(
    document: &Path,
    folder: &Path,
    schemes: &[ColorScheme],
    styles: Option<Value>,
) -> String {
    let from = document.parent().unwrap_or(Path::new(""));
    let config = json!({
        "runtime": format!("{}/", relative_url(from, folder)),
        "version": hash(),
        "fonts": (avenger_fonts::FACES.iter()).map(|(name, _)| *name).collect::<Vec<_>>(),
        "styles": styles,
        "colorSchemes": (schemes.iter())
            .map(|scheme| match scheme {
                ColorScheme::Light => "light",
                ColorScheme::Dark => "dark",
            })
            .collect::<Vec<_>>(),
    });
    // A stylesheet's text could end the script, so `</` is escaped, which JSON reads the same.
    let config = config.to_string().replace("</", "<\\/");
    format!(
        "<style>avenger-chart:not(:defined) {{ display: block; height: 0; overflow: hidden; }}</style>\n\
         <script type=\"application/json\" id=\"avenger-runtime\">{config}</script>\n\
         <script type=\"module\">\n{LOADER}</script>\n"
    )
}

/// The URL of the path `to` relative to the folder `from`, both absolute, with `/` between its
/// names.
fn relative_url(from: &Path, to: &Path) -> String {
    let names = |path: &Path| -> Vec<String> {
        (path.components())
            .filter_map(|component| match component {
                Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect()
    };
    let (from, to) = (names(from), names(to));
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = vec!["..".to_string(); from.len() - common];
    parts.extend(to[common..].iter().cloned());
    parts.join("/")
}

/// The project's stylesheets, as the page gives them to the element: the entry stylesheet, and
/// the text of each stylesheet that it reaches through its imports, by its path relative to the
/// project's root, or none for a project without a stylesheet.
pub(crate) fn styles(
    root: &Path,
    entry: &str,
    stylesheet: &avenger_sql_project::ProjectStylesheet,
) -> Option<Value> {
    stylesheet.sheet.as_ref()?;
    let files: serde_json::Map<String, Value> = (stylesheet.sources.iter())
        .filter(|path| !stylesheet.unreached.contains(path))
        .filter_map(|path| {
            let text = std::fs::read_to_string(root.join(path)).ok()?;
            Some((path.clone(), Value::String(text)))
        })
        .collect();
    Some(json!({ "entry": entry, "files": files }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Git trees of the libraries and of the crates that decode and run a page's graph,
    /// which the runtime's build saw, the version of DataFusion whose plans its graphs hold, and
    /// the chart format's version that its library reads.
    const RUNTIME: &str = include_str!("../quarto2/runtime/RUNTIME");

    /// The crates whose sources the runtime's build records: the library, and the crates that read
    /// the graphs that this avenger writes.
    const TREES: [&str; 6] = [
        "avenger-chart-web",
        "avenger-query-web",
        "avenger-physical-graph",
        "avenger-selection-datafusion",
        "avenger-sql",
        "avenger-datafusion-transforms",
    ];

    #[test]
    fn the_runtime_is_a_build_of_the_libraries_of_this_checkout() {
        // The element is the library's own, as the build copies it.
        assert_eq!(
            std::str::from_utf8(FILES[2].1).unwrap(),
            include_str!("../../avenger-chart-web/element/avenger-chart.js"),
            "run quarto2/scripts/build-runtime.sh"
        );
        // The build saw the committed sources of the library and of the graphs' readers. A
        // checkout without Git skips it.
        for name in TREES {
            let recorded = RUNTIME
                .lines()
                .find_map(|line| line.strip_prefix(&format!("{name} ")))
                .unwrap_or_else(|| panic!("the runtime records the tree of {name}"));
            let Ok(output) = std::process::Command::new("git")
                .args(["rev-parse", &format!("HEAD:{name}")])
                .current_dir(env!("CARGO_MANIFEST_DIR"))
                .output()
            else {
                return;
            };
            if output.status.success() {
                let tree = String::from_utf8_lossy(&output.stdout);
                assert_eq!(
                    recorded,
                    tree.trim(),
                    "{name} changed since the runtime's build: run quarto2/scripts/build-runtime.sh"
                );
            }
        }
        // The plans of the graphs that this avenger writes are DataFusion's of its version.
        let datafusion = RUNTIME
            .lines()
            .find_map(|line| line.strip_prefix("datafusion "))
            .expect("the runtime records DataFusion's version");
        assert_eq!(
            datafusion,
            datafusion::DATAFUSION_VERSION,
            "run quarto2/scripts/build-runtime.sh"
        );
    }

    #[test]
    fn the_runtime_reads_the_chart_format_of_this_avenger() {
        let format = RUNTIME
            .lines()
            .find_map(|line| line.strip_prefix("format "))
            .and_then(|version| version.trim().parse::<u32>().ok());
        assert_eq!(format, Some(avenger_chart::FORMAT_VERSION), "{RUNTIME}");
        // The module is gzip, not a pointer file of Git LFS.
        let (_, module) = FILES[0];
        assert_eq!(&module[..2], &[0x1f, 0x8b]);
    }

    #[test]
    fn a_url_relative_to_a_folder_climbs_to_the_common_folder() {
        let url = |from: &str, to: &str| relative_url(Path::new(from), Path::new(to));
        assert_eq!(url("/p", "/p/_avenger/abc"), "_avenger/abc");
        assert_eq!(url("/p/docs/a", "/p/_avenger/abc"), "../../_avenger/abc");
    }
}
