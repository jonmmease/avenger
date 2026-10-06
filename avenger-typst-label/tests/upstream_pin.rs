//! The upstream Typst release is pinned in `tests/fixtures/typst-pin.toml`, and everything that
//! names it must agree: the dependency pins, the ported files' headers, the `// upstream:`
//! markers, the release named in prose, and UPSTREAM.md's revision link and Mirror table.
//! UPSTREAM.md gives the grammars and the procedure for moving the pin.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

struct Pin {
    version: String,
    commit: String,
    codex: String,
}

#[test]
fn dependencies_are_pinned_to_the_release() {
    let pin = pin();
    let manifest: toml::Table = read_toml(&crate_dir().join("Cargo.toml"));
    let dependencies = manifest["dependencies"].as_table().unwrap();
    let mut problems = Vec::new();
    for (name, release) in [
        ("typst-syntax", &pin.version),
        ("typst-utils", &pin.version),
        ("codex", &pin.codex),
    ] {
        let requirement = match dependencies.get(name) {
            Some(toml::Value::String(version)) => Some(version.as_str()),
            Some(toml::Value::Table(table)) => {
                table.get("version").and_then(toml::Value::as_str)
            }
            _ => None,
        };
        let expected = format!("={release}");
        if requirement != Some(expected.as_str()) {
            problems.push(format!(
                "avenger-typst-label/Cargo.toml: `{name}` requires {requirement:?}, not \"{expected}\""
            ));
        }
    }
    assert_none(problems);
}

#[test]
fn headers_and_markers_name_the_release() {
    let pin = pin();
    let mut problems = Vec::new();
    for root in [crate_dir().join("src"), references_dir().join("src")] {
        for path in files(&root, &["rs"]) {
            let text = fs::read_to_string(&path).unwrap();
            let shown = shown(&path);
            if let Some(first) = text.lines().next()
                && first.starts_with("//! Ported from")
            {
                match header_version(first) {
                    Some(version) if version == pin.version => {}
                    Some(version) => {
                        problems.push(format!("{shown}:1: the header names v{version}"))
                    }
                    None => problems.push(format!(
                        "{shown}:1: the header isn't one line of `//! Ported from \
                         crates/<path> @ v<version>, modified for Avenger.`"
                    )),
                }
            }
            for (index, line) in text.lines().enumerate() {
                let Some(marker) = line.trim_start().strip_prefix("// upstream:") else {
                    continue;
                };
                let line = index + 1;
                match marker_version(marker) {
                    Some(version) if version == pin.version => {}
                    Some(version) => problems
                        .push(format!("{shown}:{line}: the marker names v{version}")),
                    None => problems.push(format!(
                        "{shown}:{line}: the marker isn't `// upstream: \
                         crates/<path>::<Item>[::<method>] @ v<version>[, <note>]`"
                    )),
                }
            }
        }
    }
    assert_none(problems);
}

#[test]
fn prose_names_the_release() {
    let pin = pin();
    let mut problems = Vec::new();
    let mut paths = files(&crate_dir(), &["rs", "md", "toml"]);
    paths.push(crate_dir().join("NOTICE"));
    paths.extend(files(&references_dir().join("src"), &["rs"]));
    paths.push(references_dir().join("Cargo.toml"));
    paths.extend(files(&repo_dir().join("tools/typst-upstream"), &["py", "md"]));
    for path in paths {
        let text = fs::read_to_string(&path).unwrap();
        for (index, line) in text.lines().enumerate() {
            for version in named_versions(line) {
                if version != pin.version {
                    problems.push(format!(
                        "{}:{}: names {version}",
                        shown(&path),
                        index + 1
                    ));
                }
            }
        }
    }
    assert_none(problems);
}

#[test]
fn upstream_md_has_the_revision_and_every_ported_file() {
    let pin = pin();
    let text = fs::read_to_string(crate_dir().join("UPSTREAM.md")).unwrap();
    let mut problems = Vec::new();
    if !text.contains(&pin.commit) {
        problems
            .push(format!("UPSTREAM.md doesn't link the pinned commit {}", pin.commit));
    }
    let listed = mirror_rows(&text);
    let ported = ported_files();
    for (file, upstream) in &ported {
        match listed.get(file) {
            Some(row) if row == upstream => {}
            Some(row) => problems.push(format!(
                "UPSTREAM.md lists `{file}` as a port of `{row}`, but its header names `{upstream}`"
            )),
            None => problems.push(format!("UPSTREAM.md's Mirror table lacks `{file}`")),
        }
    }
    for file in listed.keys().filter(|file| !ported.contains_key(*file)) {
        problems.push(format!(
            "UPSTREAM.md's Mirror table lists `{file}`, which isn't a ported file"
        ));
    }
    assert_none(problems);
}

/// The version in `//! Ported from crates/<path> @ v<version>, modified for Avenger.`
fn header_version(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("//! Ported from crates/")?;
    let (path, version) =
        rest.strip_suffix(", modified for Avenger.")?.split_once(" @ v")?;
    (is_path(path) && path.ends_with(".rs") && is_version(version)).then_some(version)
}

/// The version in `// upstream: crates/<path>::<Item>[::<method>] @ v<version>[, <note>]`,
/// given what follows `// upstream:`.
fn marker_version(marker: &str) -> Option<&str> {
    let (target, rest) = marker.strip_prefix(" crates/")?.split_once(" @ v")?;
    let (path, item) = target.split_once(".rs::")?;
    let names: Vec<&str> = item.split("::").collect();
    let version = rest.split_once(", ").map_or(rest, |(version, _)| version);
    (is_path(path)
        && names.len() <= 2
        && names.iter().all(|name| !name.is_empty() && !name.contains(' '))
        && is_version(version))
    .then_some(version)
}

/// The releases a line names as `Typst X.Y.Z`, `@ vX.Y.Z` or `at vX.Y.Z`.
fn named_versions(line: &str) -> Vec<&str> {
    let mut versions = Vec::new();
    for prefix in ["Typst ", "@ v", "at v"] {
        for (start, _) in line.match_indices(prefix) {
            let rest = &line[start + prefix.len()..];
            let end = rest
                .find(|c: char| !c.is_ascii_digit() && c != '.')
                .unwrap_or(rest.len());
            let version = rest[..end].trim_end_matches('.');
            if is_version(version) {
                versions.push(version);
            }
        }
    }
    versions
}

fn is_path(path: &str) -> bool {
    !path.is_empty() && !path.contains(char::is_whitespace)
}

fn is_version(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

/// The Mirror table's rows, ``| `file` | `upstream` | status |``, by Avenger file.
fn mirror_rows(text: &str) -> BTreeMap<String, String> {
    let section = text
        .split("\n## Mirror\n")
        .nth(1)
        .expect("UPSTREAM.md has a Mirror section");
    let section = section.split("\n## ").next().unwrap();
    let code = |cell: &str| Some(cell.strip_prefix('`')?.strip_suffix('`')?.to_string());
    section
        .lines()
        .filter_map(|line| {
            let cells: Vec<&str> = line
                .strip_prefix('|')?
                .strip_suffix('|')?
                .split('|')
                .map(str::trim)
                .collect();
            let [file, upstream, _] = cells[..] else { return None };
            Some((code(file)?, code(upstream)?))
        })
        .collect()
}

/// The ported files, by path relative to `src/`, with the upstream file each header names.
fn ported_files() -> BTreeMap<String, String> {
    let src = crate_dir().join("src");
    files(&src, &["rs"])
        .into_iter()
        .filter_map(|path| {
            let text = fs::read_to_string(&path).unwrap();
            let rest = text.lines().next()?.strip_prefix("//! Ported from ")?;
            let (upstream, _) = rest.split_once(" @ ")?;
            let file = path.strip_prefix(&src).unwrap().to_str()?.replace('\\', "/");
            Some((file, upstream.to_string()))
        })
        .collect()
}

/// The files under `dir` with one of the extensions, in path order. Build and test output is
/// skipped.
fn files(dir: &Path, extensions: &[&str]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if !path.ends_with("target") && !path.ends_with("tests/output") {
                found.extend(files(&path, extensions));
            }
        } else if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extensions.contains(&extension))
        {
            found.push(path);
        }
    }
    found.sort();
    found
}

fn pin() -> Pin {
    let path = crate_dir().join("tests/fixtures/typst-pin.toml");
    let pin: toml::Table = read_toml(&path);
    let field = |name: &str| {
        pin.get(name)
            .and_then(toml::Value::as_str)
            .unwrap_or_else(|| panic!("{} has no `{name}`", shown(&path)))
            .to_string()
    };
    Pin {
        version: field("version"),
        commit: field("commit"),
        codex: field("codex"),
    }
}

fn read_toml(path: &Path) -> toml::Table {
    toml::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

/// Fails listing every problem, one per line.
fn assert_none(problems: Vec<String>) {
    assert!(problems.is_empty(), "\n{}\n", problems.join("\n"));
}

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo_dir() -> PathBuf {
    crate_dir().parent().unwrap().to_path_buf()
}

fn references_dir() -> PathBuf {
    repo_dir().join("tools/typst-upstream/references")
}

/// A path relative to the repository.
fn shown(path: &Path) -> String {
    path.strip_prefix(repo_dir()).unwrap_or(path).display().to_string()
}
