//! The paths of the imports of a project's stylesheets.

/// The path, relative to the project's root, of the import `url` of the stylesheet at `importer`,
/// itself a path relative to the root: the names of `url` after the directory of `importer`, where
/// `..` leaves a directory. An import is a path relative to the stylesheet that holds it, so a URL
/// that starts with `/`, that names a scheme such as `std:`, or that holds `\` is an error, and so
/// is a path that goes above the root.
pub fn resolve_import(url: &str, importer: &str) -> Result<String, String> {
    let scheme = url.split_once(':').map(|(scheme, _)| scheme);
    let not_relative =
        || "an import is a path relative to the stylesheet that holds it".to_string();
    if url.starts_with('/')
        || url.contains('\\')
        || importer.contains('\\')
        || scheme.is_some_and(|scheme| !scheme.contains('/'))
    {
        return Err(not_relative());
    }
    let mut names: Vec<&str> = (importer.split('/'))
        .filter(|name| !matches!(*name, "" | "."))
        .collect();
    names.pop();
    for name in url.split('/') {
        match name {
            "" | "." => {}
            ".." => {
                names.pop().ok_or("the path goes above the project root")?;
            }
            name => names.push(name),
        }
    }
    match names.is_empty() {
        true => Err(not_relative()),
        false => Ok(names.join("/")),
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_import;

    #[test]
    fn an_import_resolves_after_the_directory_of_its_stylesheet() {
        assert_eq!(
            resolve_import("theme.css", "styles/chart.css").unwrap(),
            "styles/theme.css"
        );
        assert_eq!(
            resolve_import("../base.css", "styles/chart.css").unwrap(),
            "base.css"
        );
        assert_eq!(
            resolve_import("./a/./b.css", "chart.css").unwrap(),
            "a/b.css"
        );
        for url in [
            "/abs.css",
            "std:base.css",
            "https://x/y.css",
            "a\\b.css",
            "../../up.css",
        ] {
            assert!(resolve_import(url, "styles/chart.css").is_err(), "{url}");
        }
    }
}
