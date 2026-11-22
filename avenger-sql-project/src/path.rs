//! The paths of a project's files and directories, relative to its root.

/// The path `path`, relative to a project's root, with `/` between its names and without empty
/// names and `.` names, so that `./charts//a.sql` is `charts/a.sql`, or the empty path for the
/// root itself. A path that starts with `/`, that holds `\`, or that has a `..` name is not a
/// path relative to the root, and is `None`.
pub(crate) fn relative(path: &str) -> Option<String> {
    if path.starts_with('/') || path.contains('\\') {
        return None;
    }
    let names: Vec<&str> = (path.split('/'))
        .filter(|name| !matches!(*name, "" | "."))
        .collect();
    (!names.contains(&"..")).then(|| names.join("/"))
}

/// The path `path` relative to `root`, with its names separated by `/`, when it is under the
/// root and its names are UTF-8.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn under_root(root: &std::path::Path, path: &std::path::Path) -> Option<String> {
    let names: Option<Vec<&str>> = (path.strip_prefix(root).ok()?.components())
        .map(|component| component.as_os_str().to_str())
        .collect();
    Some(names?.join("/"))
}
