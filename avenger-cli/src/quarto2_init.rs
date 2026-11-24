//! `avenger quarto2-init`: a project set up for Quarto 2 and avenger. It installs the engine
//! extension that the binary embeds, through `q2 add`, so q2 keeps its record of the extension's
//! files, writes `_quarto.yml` when the project has none, and adds q2's working folder to
//! `.gitignore`. It installs nothing on the machine: q2, Node, and `avenger` are the user's.
//!
//! The extension's `version` is avenger's, so a release of avenger moves the extension, as `q2 add`
//! replaces an installed extension only for another version. An installed extension that is newer
//! than this avenger's stays, unless `--force`, since installing the older one would put it in the
//! project for every collaborator.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::Status;

/// The files of the extension that q2 loads, by their paths in its folder, as the binary embeds
/// them.
const EXTENSION: &[(&str, &str)] = &[
    (
        "_extension.yml",
        include_str!("../quarto2/_extensions/avenger/_extension.yml"),
    ),
    (
        "dist/avenger-engine.js",
        include_str!("../quarto2/_extensions/avenger/dist/avenger-engine.js"),
    ),
];

/// The folder in the project, under q2's working folder, that `q2 add` installs the extension
/// from, so its record names a path of the project rather than of the machine.
const SOURCE: &str = ".quarto/avenger-extension";

/// The `_quarto.yml` of a project without one: a Quarto 2 project, which q2 presumes a project to
/// be only when it says so, whose preview runs the engine on every edit.
const QUARTO_YML: &str =
    "quarto-required: \"^2\"\nproject:\n  type: default\npreview:\n  engine: auto\n";

/// The document of `--example`.
const EXAMPLE: &str = "---\ntitle: Avenger and Quarto\n---\n\n```{avenger-sql}\nCREATE TEMP TABLE cars AS (\n  FROM UNNEST([STRUCT('Europe' AS origin, 3 AS count), STRUCT('Japan', 5), STRUCT('USA', 8)])\n)\n```\n\n```{avenger-sql}\n#| label: fig-cars\n#| fig-cap: Cars by origin\nFROM cars |> AGGREGATE mark_bar(x => origin, y => count) AS chart\n```\n\nThe cars by origin are in @fig-cars.\n";

/// The options of a run.
pub struct Options {
    /// The project's folder, or the current folder.
    pub dir: Option<PathBuf>,
    /// Writes `index.av.qmd`, a document with a chart.
    pub example: bool,
    /// Prints the steps without running `q2 add` or writing a file.
    pub dry_run: bool,
    /// Passes `--force` to `q2 add`, and replaces a newer extension.
    pub force: bool,
}

/// Sets up the project of `options`, and prints a line for each step as it takes it.
pub fn run(options: &Options) -> Result<Status, String> {
    let dir = match &options.dir {
        Some(dir) => dir.clone(),
        None => std::env::current_dir().map_err(|error| error.to_string())?,
    };
    let dir = std::fs::canonicalize(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;

    // The programs: q2 is required, and an old or missing Node is a warning, since q2 also finds
    // Node where this doesn't look.
    let Some(q2) = version("q2", &["--version"]) else {
        return Err(
            "q2 is not on PATH: install Quarto 2 (`brew install quarto-dev/tap/q2`, or a \
                    release of https://github.com/quarto-dev/q2), then run this again"
                .to_string(),
        );
    };
    println!("q2: {q2}");
    let node = std::env::var("QUARTO_NODE").unwrap_or_else(|_| "node".to_string());
    match version(&node, &["--version"]) {
        Some(found) if node_major(&found) >= Some(24) => println!("Node: {found}"),
        found => println!(
            "warning: q2 runs the extension with Node 24 or newer, and {} has {}: install it, \
             or set QUARTO_NODE; q2 also looks in Homebrew, volta, fnm, and nvm",
            node,
            found.as_deref().unwrap_or("no version"),
        ),
    }

    // The extension.
    let ours = env!("CARGO_PKG_VERSION");
    let installed = installed_version(&dir);
    let newer = installed
        .as_deref()
        .is_some_and(|installed| numbers(installed) > numbers(ours));
    let failed = match (newer && !options.force, options.dry_run) {
        (true, _) => {
            println!(
                "extension: kept, since the project's is from avenger {}, newer than this \
                 avenger's {ours}: upgrade avenger, or replace it with `--force`",
                installed.unwrap_or_default()
            );
            false
        }
        (false, true) => {
            println!(
                "extension: would write {SOURCE}/ and run `q2 add {SOURCE} --no-prompt{}`",
                if options.force { " --force" } else { "" }
            );
            false
        }
        (false, false) => {
            write_source(&dir, ours)?;
            let mut args = vec!["add", SOURCE, "--no-prompt"];
            if options.force {
                args.push("--force");
            }
            let added = Command::new("q2")
                .args(&args)
                .current_dir(&dir)
                .output()
                .map_err(|error| format!("q2 add: {error}"))?;
            let said = String::from_utf8_lossy(&added.stdout).to_string()
                + &String::from_utf8_lossy(&added.stderr);
            // q2's own message says why it failed, as a local change of the extension's files.
            match (added.status.success(), said.contains("No changes required")) {
                (true, true) => println!("extension: already at avenger {ours}"),
                (true, false) => println!("extension: installed, avenger {ours}"),
                (false, _) => println!("error: q2 add failed: {}", said.trim()),
            }
            !added.status.success()
        }
    };

    // The project's files.
    let quarto_yml = dir.join("_quarto.yml");
    match std::fs::read_to_string(&quarto_yml) {
        Err(_) => {
            write(&quarto_yml, QUARTO_YML, options.dry_run)?;
            println!(
                "{}",
                done(options.dry_run, "_quarto.yml: written, with `preview: engine: auto`")
            );
        }
        Ok(text) if has_preview_engine(&text) => {
            println!("_quarto.yml: has `preview.engine` already")
        }
        Ok(_) => println!(
            "_quarto.yml: unchanged; to redraw charts as you edit, add\n  preview:\n    engine: auto"
        ),
    }
    let gitignore = dir.join(".gitignore");
    let ignored = std::fs::read_to_string(&gitignore).unwrap_or_default();
    match (ignored.lines()).any(|line| matches!(line.trim(), "/.quarto/" | ".quarto/" | "/.quarto"))
    {
        true => println!(".gitignore: ignores `/.quarto/` already"),
        false => {
            let mut text = ignored.clone();
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str("/.quarto/\n");
            write(&gitignore, &text, options.dry_run)?;
            println!("{}", done(options.dry_run, ".gitignore: `/.quarto/` added"));
        }
    }
    let example = dir.join("index.av.qmd");
    if options.example {
        match example.exists() {
            true => println!("index.av.qmd: kept, since it exists"),
            false => {
                write(&example, EXAMPLE, options.dry_run)?;
                println!("{}", done(options.dry_run, "index.av.qmd: written"));
            }
        }
    }
    let preview = match example.exists() || options.example {
        true => "q2 preview index.av.qmd",
        false => "q2 preview <DOCUMENT>.av.qmd",
    };
    println!("preview: {preview}");
    Ok(match failed {
        true => Status::Failed,
        false => Status::Ok,
    })
}

/// The line of a step, which a dry run says it would take.
fn done(dry_run: bool, line: &str) -> String {
    match dry_run {
        true => format!("would: {line}"),
        false => line.to_string(),
    }
}

/// Writes `text` to `path`, unless `dry_run`.
fn write(path: &Path, text: &str, dry_run: bool) -> Result<(), String> {
    if dry_run {
        return Ok(());
    }
    std::fs::write(path, text).map_err(|error| format!("{}: {error}", path.display()))
}

/// Writes the embedded extension under [`SOURCE`] in the project `dir`, with avenger's version
/// `version` as the extension's.
fn write_source(dir: &Path, version: &str) -> Result<(), String> {
    let folder = dir.join(SOURCE).join("_extensions").join("avenger");
    for (path, text) in EXTENSION {
        let file = folder.join(path);
        let text = match *path {
            "_extension.yml" => with_version(text, version),
            _ => text.to_string(),
        };
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&file, text).map_err(|error| format!("{}: {error}", file.display()))?;
    }
    Ok(())
}

/// The manifest `text` with `version` as its `version`.
fn with_version(text: &str, version: &str) -> String {
    let lines: Vec<String> = (text.lines())
        .map(|line| match line.starts_with("version:") {
            true => format!("version: {version}"),
            false => line.to_string(),
        })
        .collect();
    lines.join("\n") + "\n"
}

/// The version of the extension installed in the project `dir`, or `None` without one.
fn installed_version(dir: &Path) -> Option<String> {
    let manifest = dir.join("_extensions/avenger/_extension.yml");
    let text = std::fs::read_to_string(manifest).ok()?;
    (text.lines())
        .find_map(|line| line.strip_prefix("version:"))
        .map(|version| version.trim().trim_matches(['"', '\'']).to_string())
}

/// The numbers of the version `version`, as `0.48.0` of `0.48.0-nightly`, which compare in order.
fn numbers(version: &str) -> Vec<u64> {
    let version = version.split(['-', '+']).next().unwrap_or(version);
    (version.split('.'))
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

/// The last word of the output of `program` with `args`, or `None` when it doesn't run.
fn version(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.split_whitespace().last().map(str::to_string)
}

/// The major version of Node's `v24.1.0`.
fn node_major(version: &str) -> Option<u64> {
    version
        .trim_start_matches('v')
        .split('.')
        .next()?
        .parse()
        .ok()
}

/// Whether the `_quarto.yml` text `text` sets `preview.engine`.
fn has_preview_engine(text: &str) -> bool {
    let mut in_preview = false;
    for line in text.lines() {
        if !line.starts_with([' ', '\t']) {
            in_preview = line.trim_end() == "preview:";
            continue;
        }
        if in_preview && line.trim_start().starts_with("engine:") {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_manifest_takes_avenger_s_version() {
        let (_, manifest) = EXTENSION[0];
        assert!(manifest.lines().any(|line| line.starts_with("version:")));
        let written = with_version(manifest, "9.8.7");
        assert!(written.contains("\nversion: 9.8.7\n"), "{written}");
        assert_eq!(written.matches("version:").count(), 1);
    }
}
