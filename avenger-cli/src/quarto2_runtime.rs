//! The runtime of the interactive charts of Quarto 2 pages. The binary embeds the WebAssembly
//! library of avenger-chart-web as gzip, its glue, and the `<avenger-chart>` element, which
//! `quarto2/scripts/build-runtime.sh` builds as one snapshot, and the engine writes them, with the
//! faces of `avenger-fonts` as gzip, to a project's `_avenger/<HASH>/`, which q2 copies to the
//! output with the project's resources. A page's header holds the loader and the page's
//! configuration, and imports the runtime when its first chart nears the screen.

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
        "avenger_chart_web_bg.wasm.gz",
        include_bytes!("../quarto2/runtime/avenger_chart_web_bg.wasm.gz"),
    ),
    (
        "avenger_chart_web.js",
        include_bytes!("../quarto2/runtime/avenger_chart_web.js"),
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

/// The name of the runtime's folder: the first 16 hexadecimal digits of the SHA-256 of its files
/// and its fonts, so a page never reads a cached runtime of another build.
pub(crate) fn hash() -> String {
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
}

/// Writes the runtime to `<project>/_avenger/<HASH>/`, when no folder of its hash exists, and
/// removes the folders of other hashes, and returns the folder. It never rewrites a folder that
/// exists, since `q2 preview --static` renders again on every change of the project's files. The
/// files go to a folder of a temporary name that a rename puts in place, so engines that q2 runs
/// at once for several documents never leave or read a folder half written.
pub(crate) fn write(project: &Path) -> Result<PathBuf, String> {
    let error = |error: std::io::Error| format!("the runtime of interactive charts: {error}");
    let root = project.join(FOLDER);
    let hash = hash();
    let folder = root.join(&hash);
    if !folder.is_dir() {
        let temporary = root.join(format!(".{hash}-{}", std::process::id()));
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
        // Another engine may have put the folder in place first, which leaves this one's copy.
        if std::fs::rename(&temporary, &folder).is_err() && folder.is_dir() {
            let _ = std::fs::remove_dir_all(&temporary);
        } else if !folder.is_dir() {
            return Err(format!(
                "the runtime of interactive charts: {} did not write",
                folder.display()
            ));
        }
    }
    for entry in std::fs::read_dir(&root).map_err(error)?.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name != hash && !name.starts_with('.') && entry.path().is_dir() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
    Ok(folder)
}

/// The header of the page of the document at `document`, whose runtime is at `folder`: the page's
/// configuration and the loader, with a rule that keeps an element that has not loaded from taking
/// room. `schemes` are the color schemes of the document's themes, and `styles` the project's
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

    /// The commit of avenger-chart-web that the runtime's build saw, and the chart format's
    /// version that its library reads.
    const RUNTIME: &str = include_str!("../quarto2/runtime/RUNTIME");

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
