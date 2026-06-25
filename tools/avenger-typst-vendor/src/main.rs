use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use sha2::{Digest, Sha256};

const GENERATOR_VERSION: &str = env!("CARGO_PKG_VERSION");

type Result<T> = std::result::Result<T, String>;

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = Args::parse()?;
    let recipe = Recipe::load(&args.recipe)?;

    if args.check_deny_list_only {
        check_denied_dependencies(&args.out, &recipe.deny_dependencies.crates)?;
        return Ok(());
    }

    let upstream = args
        .upstream
        .as_ref()
        .ok_or_else(|| "--upstream is required unless --check-deny-list-only is set".to_string())?;
    let requested_rev = args
        .rev
        .as_ref()
        .ok_or_else(|| "--rev is required unless --check-deny-list-only is set".to_string())?;

    validate_upstream(upstream, requested_rev, args.allow_dirty_upstream)?;
    validate_expected_workspace_version(
        upstream,
        recipe.upstream.expected_workspace_version.as_deref(),
    )?;
    generate_vendor_tree(&args, &recipe, upstream, requested_rev)
}

#[derive(Debug)]
struct Args {
    upstream: Option<PathBuf>,
    rev: Option<String>,
    recipe: PathBuf,
    out: PathBuf,
    allow_dirty_upstream: bool,
    check_deny_list_only: bool,
}

impl Args {
    fn parse() -> Result<Self> {
        let mut args = Self {
            upstream: None,
            rev: None,
            recipe: PathBuf::from("tools/vendor-typst-math/recipe.toml"),
            out: PathBuf::from("vendor/typst-avenger"),
            allow_dirty_upstream: false,
            check_deny_list_only: false,
        };

        let mut raw = env::args().skip(1);
        while let Some(arg) = raw.next() {
            match arg.as_str() {
                "--help" | "-h" => {
                    print_usage();
                    std::process::exit(0);
                }
                "--upstream" => args.upstream = Some(next_path(&mut raw, "--upstream")?),
                "--rev" => args.rev = Some(next_value(&mut raw, "--rev")?),
                "--recipe" => args.recipe = next_path(&mut raw, "--recipe")?,
                "--out" => args.out = next_path(&mut raw, "--out")?,
                "--allow-dirty-upstream" => args.allow_dirty_upstream = true,
                "--check-deny-list-only" => args.check_deny_list_only = true,
                other => return Err(format!("unknown argument: {other}")),
            }
        }

        Ok(args)
    }
}

fn next_path(raw: &mut impl Iterator<Item = String>, flag: &str) -> Result<PathBuf> {
    Ok(PathBuf::from(next_value(raw, flag)?))
}

fn next_value(raw: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
    raw.next().ok_or_else(|| format!("{flag} requires a value"))
}

fn print_usage() {
    println!(
        "Usage: cargo run --release -p avenger-typst-vendor -- \\
  --upstream ../typst \\
  --rev <git-rev> \\
  [--recipe tools/vendor-typst-math/recipe.toml] \\
  [--out vendor/typst-avenger] \\
  [--allow-dirty-upstream]\n\n\
Use --check-deny-list-only with --recipe/--out to scan an existing vendor tree."
    );
}

#[derive(Debug, Default)]
struct Recipe {
    upstream: RecipeUpstream,
    copy: RecipeCopy,
    deny_dependencies: RecipeDenyDependencies,
}

impl Recipe {
    fn load(path: &Path) -> Result<Self> {
        let source = fs::read_to_string(path)
            .map_err(|err| format!("failed to read recipe {}: {err}", path.display()))?;
        parse_recipe(&source)
    }
}

#[derive(Debug, Default)]
struct RecipeUpstream {
    expected_workspace_version: Option<String>,
}

#[derive(Debug, Default)]
struct RecipeCopy {
    crates: Vec<String>,
    root_files: Vec<String>,
}

#[derive(Debug, Default)]
struct RecipeDenyDependencies {
    crates: Vec<String>,
}

#[derive(Debug)]
struct PendingArray {
    section: String,
    key: String,
    values: Vec<String>,
}

fn parse_recipe(source: &str) -> Result<Recipe> {
    let mut recipe = Recipe::default();
    let mut section = String::new();
    let mut pending_array: Option<PendingArray> = None;

    for (line_index, raw_line) in source.lines().enumerate() {
        let line_number = line_index + 1;
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }

        if let Some(pending) = &mut pending_array {
            pending.values.extend(parse_quoted_values(line)?);
            if line.contains(']') {
                let pending = pending_array.take().expect("pending array exists");
                apply_array(&mut recipe, &pending.section, &pending.key, pending.values)?;
            }
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_string();
            continue;
        }

        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("invalid recipe line {line_number}: {raw_line}"))?;
        let key = key.trim();
        let value = value.trim();

        if value.starts_with('[') {
            let mut values = parse_quoted_values(value)?;
            if value.contains(']') {
                apply_array(&mut recipe, &section, key, values)?;
            } else {
                pending_array = Some(PendingArray {
                    section: section.clone(),
                    key: key.to_string(),
                    values: std::mem::take(&mut values),
                });
            }
        } else {
            apply_string(&mut recipe, &section, key, parse_string(value)?)?;
        }
    }

    if let Some(pending) = pending_array {
        return Err(format!(
            "unterminated array for [{}].{}",
            pending.section, pending.key
        ));
    }

    Ok(recipe)
}

fn strip_comment(line: &str) -> &str {
    let mut escaped = false;
    let mut in_string = false;

    for (idx, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            '#' if !in_string => return &line[..idx],
            _ => {}
        }
    }

    line
}

fn parse_string(value: &str) -> Result<String> {
    let value = value.trim();
    if !(value.starts_with('"') && value.ends_with('"')) {
        return Err(format!("expected quoted string, got {value}"));
    }
    Ok(value[1..value.len() - 1].replace("\\\"", "\""))
}

fn parse_quoted_values(value: &str) -> Result<Vec<String>> {
    let mut values = Vec::new();
    let mut current = String::new();
    let mut escaped = false;
    let mut in_string = false;

    for ch in value.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }

        match ch {
            '\\' if in_string => escaped = true,
            '"' if in_string => {
                values.push(std::mem::take(&mut current));
                in_string = false;
            }
            '"' => in_string = true,
            _ if in_string => current.push(ch),
            _ => {}
        }
    }

    if in_string {
        return Err(format!(
            "unterminated quoted string in array value: {value}"
        ));
    }

    Ok(values)
}

fn apply_string(recipe: &mut Recipe, section: &str, key: &str, value: String) -> Result<()> {
    match (section, key) {
        ("upstream", "expected_workspace_version") => {
            recipe.upstream.expected_workspace_version = Some(value);
            Ok(())
        }
        _ => Err(format!("unknown recipe string key [{}].{}", section, key)),
    }
}

fn apply_array(recipe: &mut Recipe, section: &str, key: &str, values: Vec<String>) -> Result<()> {
    match (section, key) {
        ("copy", "crates") => recipe.copy.crates = values,
        ("copy", "root_files") => recipe.copy.root_files = values,
        ("deny_dependencies", "crates") => recipe.deny_dependencies.crates = values,
        _ => return Err(format!("unknown recipe array key [{}].{}", section, key)),
    }
    Ok(())
}

fn validate_upstream(upstream: &Path, requested_rev: &str, allow_dirty: bool) -> Result<()> {
    if !upstream.exists() {
        return Err(format!(
            "upstream path does not exist: {}",
            upstream.display()
        ));
    }
    if !upstream.join(".git").exists() {
        return Err(format!(
            "upstream path is not a git checkout: {}",
            upstream.display()
        ));
    }
    if !upstream.join("Cargo.toml").exists() || !upstream.join("crates").exists() {
        return Err(format!(
            "upstream path does not look like a Typst checkout: {}",
            upstream.display()
        ));
    }

    let resolved_requested = git_output(upstream, &["rev-parse", "--verify", requested_rev])?;
    let current = git_output(upstream, &["rev-parse", "HEAD"])?;
    if current != resolved_requested {
        return Err(format!(
            "upstream checkout is at {current}, but requested revision resolves to {resolved_requested}"
        ));
    }

    let dirty = git_output(upstream, &["status", "--porcelain"])?;
    if !dirty.is_empty() && !allow_dirty {
        return Err(format!(
            "upstream checkout has uncommitted changes; rerun with --allow-dirty-upstream if intentional:\n{dirty}"
        ));
    }

    Ok(())
}

fn validate_expected_workspace_version(upstream: &Path, expected: Option<&str>) -> Result<()> {
    let Some(expected) = expected else {
        return Ok(());
    };

    let cargo_toml_path = upstream.join("Cargo.toml");
    let source = fs::read_to_string(&cargo_toml_path)
        .map_err(|err| format!("failed to read {}: {err}", cargo_toml_path.display()))?;
    let actual = workspace_package_version(&source).ok_or_else(|| {
        format!(
            "failed to find [workspace.package] version in {}",
            cargo_toml_path.display()
        )
    })?;

    if actual != expected {
        return Err(format!(
            "Typst workspace version is {actual}, but recipe expected {expected}"
        ));
    }

    Ok(())
}

fn workspace_package_version(cargo_toml: &str) -> Option<String> {
    let mut in_workspace_package = false;

    for raw_line in cargo_toml.lines() {
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_workspace_package = line == "[workspace.package]";
            continue;
        }
        if in_workspace_package {
            if let Some((key, value)) = line.split_once('=') {
                if key.trim() == "version" {
                    return parse_string(value.trim()).ok();
                }
            }
        }
    }

    None
}

fn git_output(cwd: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|err| format!("failed to run git {}: {err}", args.join(" ")))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn generate_vendor_tree(
    args: &Args,
    recipe: &Recipe,
    upstream: &Path,
    requested_rev: &str,
) -> Result<()> {
    if args.out.exists() {
        fs::remove_dir_all(&args.out)
            .map_err(|err| format!("failed to remove {}: {err}", args.out.display()))?;
    }
    fs::create_dir_all(&args.out)
        .map_err(|err| format!("failed to create {}: {err}", args.out.display()))?;

    let mut copied_files = Vec::new();
    for root_file in &recipe.copy.root_files {
        copy_one_file(upstream, &args.out, Path::new(root_file), &mut copied_files)?;
    }

    for crate_name in &recipe.copy.crates {
        let rel_dir = PathBuf::from("crates").join(crate_name);
        copy_dir_recursive(upstream, &args.out, &rel_dir, &mut copied_files)?;
    }

    let mut generated_files = Vec::new();
    write_generated(
        &args.out,
        Path::new("README.md"),
        vendor_readme(requested_rev),
        &mut generated_files,
    )?;
    write_generated(
        &args.out,
        Path::new("UPSTREAM_REV"),
        format!("{requested_rev}\n"),
        &mut generated_files,
    )?;
    write_generated(
        &args.out,
        Path::new("Cargo.toml"),
        vendor_workspace_toml(&recipe.copy.crates),
        &mut generated_files,
    )?;

    check_denied_dependencies(&args.out, &recipe.deny_dependencies.crates)?;

    let manifest = VendorManifest {
        upstream_rev: requested_rev.to_string(),
        upstream_path: canonical_path(upstream)?,
        generation_epoch_seconds: generation_epoch_seconds()?,
        generator_version: GENERATOR_VERSION.to_string(),
        recipe_path: canonical_path(&args.recipe)?,
        expected_workspace_version: recipe.upstream.expected_workspace_version.clone(),
        copied_files,
        applied_overlays: Vec::new(),
        applied_patches: Vec::new(),
        generated_files,
        denied_dependencies: recipe.deny_dependencies.crates.clone(),
    };

    let manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|err| format!("failed to serialize vendor manifest: {err}"))?;
    let manifest_path = args.out.join("VENDOR_MANIFEST.json");
    fs::write(&manifest_path, format!("{manifest_json}\n")).map_err(|err| {
        format!(
            "failed to write vendor manifest {}: {err}",
            manifest_path.display()
        )
    })?;

    Ok(())
}

fn copy_dir_recursive(
    upstream: &Path,
    out: &Path,
    rel_dir: &Path,
    copied_files: &mut Vec<CopiedFile>,
) -> Result<()> {
    let src_dir = upstream.join(rel_dir);
    if !src_dir.is_dir() {
        return Err(format!(
            "recipe copy directory not found: {}",
            rel_dir.display()
        ));
    }

    let mut entries = fs::read_dir(&src_dir)
        .map_err(|err| format!("failed to read {}: {err}", src_dir.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|err| format!("failed to read entry in {}: {err}", src_dir.display()))?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let file_name = entry.file_name();
        if should_skip_path_component(&file_name.to_string_lossy()) {
            continue;
        }

        let rel_path = rel_dir.join(file_name);
        let file_type = entry
            .file_type()
            .map_err(|err| format!("failed to inspect {}: {err}", entry.path().display()))?;
        if file_type.is_dir() {
            copy_dir_recursive(upstream, out, &rel_path, copied_files)?;
        } else if file_type.is_file() {
            copy_one_file(upstream, out, &rel_path, copied_files)?;
        }
    }

    Ok(())
}

fn should_skip_path_component(component: &str) -> bool {
    matches!(component, ".git" | "target")
}

fn copy_one_file(
    upstream: &Path,
    out: &Path,
    rel_path: &Path,
    copied_files: &mut Vec<CopiedFile>,
) -> Result<()> {
    let src = upstream.join(rel_path);
    if !src.is_file() {
        return Err(format!(
            "recipe copy file not found: {}",
            rel_path.display()
        ));
    }
    let dst = out.join(rel_path);
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }

    let bytes = fs::read(&src).map_err(|err| format!("failed to read {}: {err}", src.display()))?;
    let sha256 = sha256_hex(&bytes);
    fs::write(&dst, &bytes).map_err(|err| format!("failed to write {}: {err}", dst.display()))?;

    copied_files.push(CopiedFile {
        upstream_path: path_slash(rel_path),
        vendor_path: path_slash(rel_path),
        sha256,
        byte_len: bytes.len() as u64,
    });

    Ok(())
}

fn write_generated(
    out: &Path,
    rel_path: &Path,
    contents: String,
    generated_files: &mut Vec<GeneratedFile>,
) -> Result<()> {
    let path = out.join(rel_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }

    let mut file = fs::File::create(&path)
        .map_err(|err| format!("failed to create {}: {err}", path.display()))?;
    file.write_all(contents.as_bytes())
        .map_err(|err| format!("failed to write {}: {err}", path.display()))?;

    generated_files.push(GeneratedFile {
        path: path_slash(rel_path),
        sha256: sha256_hex(contents.as_bytes()),
        byte_len: contents.len() as u64,
    });

    Ok(())
}

fn vendor_readme(upstream_rev: &str) -> String {
    format!(
        "# Vendored Typst For Avenger\n\n\
This directory is generated by `avenger-typst-vendor` from Typst upstream revision `{upstream_rev}`.\n\n\
Do not edit generated files directly. Update `tools/vendor-typst-math/recipe.toml`, overlays, or patches, then regenerate with:\n\n\
```text\n\
cargo run --release -p avenger-typst-vendor -- --upstream ../typst --rev {upstream_rev} --allow-dirty-upstream\n\
```\n"
    )
}

fn vendor_workspace_toml(crates: &[String]) -> String {
    let mut out = String::from("[workspace]\n");
    if crates.is_empty() {
        out.push_str("members = []\n");
    } else {
        out.push_str("members = [\n");
        for crate_name in crates {
            out.push_str(&format!("    \"crates/{crate_name}\",\n"));
        }
        out.push_str("]\n");
    }
    out.push_str("resolver = \"2\"\n");
    out
}

fn check_denied_dependencies(out: &Path, denied: &[String]) -> Result<()> {
    if denied.is_empty() || !out.exists() {
        return Ok(());
    }

    let mut manifests = Vec::new();
    collect_cargo_tomls(out, &mut manifests)?;
    let mut violations = Vec::new();

    for manifest in manifests {
        let source = fs::read_to_string(&manifest)
            .map_err(|err| format!("failed to read {}: {err}", manifest.display()))?;
        for (line_index, raw_line) in source.lines().enumerate() {
            let line = strip_comment(raw_line).trim();
            if line.is_empty() {
                continue;
            }
            for denied_name in denied {
                if dependency_line_mentions(line, denied_name) {
                    violations.push(format!(
                        "{}:{} mentions denied dependency `{}`: {}",
                        manifest.display(),
                        line_index + 1,
                        denied_name,
                        raw_line.trim()
                    ));
                }
            }
        }
    }

    if violations.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "vendored dependency deny-list violations:\n{}",
            violations.join("\n")
        ))
    }
}

fn collect_cargo_tomls(dir: &Path, manifests: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }

    let mut entries = fs::read_dir(dir)
        .map_err(|err| format!("failed to read {}: {err}", dir.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|err| format!("failed to read entry in {}: {err}", dir.display()))?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let file_name = entry.file_name();
        if should_skip_path_component(&file_name.to_string_lossy()) {
            continue;
        }
        let file_type = entry
            .file_type()
            .map_err(|err| format!("failed to inspect {}: {err}", path.display()))?;
        if file_type.is_dir() {
            collect_cargo_tomls(&path, manifests)?;
        } else if file_type.is_file() && file_name == "Cargo.toml" {
            manifests.push(path);
        }
    }

    Ok(())
}

fn dependency_line_mentions(line: &str, denied_name: &str) -> bool {
    let quoted = format!("\"{denied_name}\"");
    let package_key = format!("package = {quoted}");
    if line.contains(&package_key) {
        return true;
    }

    if let Some((key, _)) = line.split_once('=') {
        key.trim() == denied_name
    } else {
        false
    }
}

fn generation_epoch_seconds() -> Result<u64> {
    match env::var("SOURCE_DATE_EPOCH") {
        Ok(value) => value.parse::<u64>().map_err(|err| {
            format!("SOURCE_DATE_EPOCH must be an unsigned integer, got {value}: {err}")
        }),
        Err(env::VarError::NotPresent) => Ok(0),
        Err(err) => Err(format!("failed to read SOURCE_DATE_EPOCH: {err}")),
    }
}

fn canonical_path(path: &Path) -> Result<String> {
    path.canonicalize()
        .map_err(|err| format!("failed to canonicalize {}: {err}", path.display()))
        .map(|path| path.display().to_string())
}

fn path_slash(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[derive(Debug, Serialize)]
struct VendorManifest {
    upstream_rev: String,
    upstream_path: String,
    generation_epoch_seconds: u64,
    generator_version: String,
    recipe_path: String,
    expected_workspace_version: Option<String>,
    copied_files: Vec<CopiedFile>,
    applied_overlays: Vec<String>,
    applied_patches: Vec<String>,
    generated_files: Vec<GeneratedFile>,
    denied_dependencies: Vec<String>,
}

#[derive(Debug, Serialize)]
struct CopiedFile {
    upstream_path: String,
    vendor_path: String,
    sha256: String,
    byte_len: u64,
}

#[derive(Debug, Serialize)]
struct GeneratedFile {
    path: String,
    sha256: String,
    byte_len: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_recipe() {
        let recipe = parse_recipe(
            r#"
            [upstream]
            expected_workspace_version = "0.15.0"

            [copy]
            crates = [
              "typst-syntax",
              "typst-layout",
            ]
            root_files = ["Cargo.toml"]

            [deny_dependencies]
            crates = ["typst", "typst-html"]
            "#,
        )
        .unwrap();

        assert_eq!(
            recipe.upstream.expected_workspace_version.as_deref(),
            Some("0.15.0")
        );
        assert_eq!(recipe.copy.crates, ["typst-syntax", "typst-layout"]);
        assert_eq!(recipe.copy.root_files, ["Cargo.toml"]);
        assert_eq!(recipe.deny_dependencies.crates, ["typst", "typst-html"]);
    }

    #[test]
    fn dependency_deny_scan_matches_direct_and_package_aliases() {
        assert!(dependency_line_mentions(
            "typst-html = \"0.1\"",
            "typst-html"
        ));
        assert!(dependency_line_mentions(
            "foo = { package = \"typst-pdf\", version = \"0.1\" }",
            "typst-pdf"
        ));
        assert!(!dependency_line_mentions(
            "typst-library = \"0.1\"",
            "typst"
        ));
    }

    #[test]
    fn reads_workspace_package_version() {
        let version = workspace_package_version(
            r#"
            [workspace]
            members = []

            [workspace.package]
            version = "0.15.0"
            "#,
        );

        assert_eq!(version.as_deref(), Some("0.15.0"));
    }
}
