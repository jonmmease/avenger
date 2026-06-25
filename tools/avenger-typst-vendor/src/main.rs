use std::collections::{BTreeMap, BTreeSet};
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
    let upstream_workspace_toml = fs::read_to_string(upstream.join("Cargo.toml"))
        .map_err(|err| format!("failed to read upstream Cargo.toml: {err}"))?;

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

    let mut applied_patches = Vec::new();
    rewrite_copied_crate_manifests(
        &args.out,
        &recipe.copy.crates,
        &recipe.deny_dependencies.crates,
        &mut applied_patches,
    )?;
    rewrite_typst_syntax_without_toml(&args.out, &recipe.copy.crates, &mut applied_patches)?;
    rewrite_typst_library_math_subset(&args.out, &recipe.copy.crates, &mut applied_patches)?;
    rewrite_typst_layout_math_subset(&args.out, &recipe.copy.crates, &mut applied_patches)?;

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
        vendor_workspace_toml(&recipe.copy.crates, &args.out, &upstream_workspace_toml)?,
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
        applied_patches,
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
    mut contents: String,
    generated_files: &mut Vec<GeneratedFile>,
) -> Result<()> {
    normalize_trailing_newline(&mut contents);

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

fn normalize_trailing_newline(contents: &mut String) {
    while contents.ends_with("\n\n") {
        contents.pop();
    }
    if !contents.ends_with('\n') {
        contents.push('\n');
    }
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

fn vendor_workspace_toml(
    crates: &[String],
    vendor_out: &Path,
    upstream_workspace_toml: &str,
) -> Result<String> {
    let mut contents = String::from("[workspace]\n");
    if crates.is_empty() {
        contents.push_str("members = []\n");
    } else {
        contents.push_str("members = [\n");
        for crate_name in crates {
            contents.push_str(&format!("    \"crates/{crate_name}\",\n"));
        }
        contents.push_str("]\n");
    }
    contents.push_str("resolver = \"2\"\n\n");

    let workspace_package = extract_toml_section(upstream_workspace_toml, "workspace.package")
        .ok_or_else(|| "upstream Cargo.toml is missing [workspace.package]".to_string())?;
    contents.push_str(&rewrite_workspace_package_section(&workspace_package));
    contents.push('\n');

    contents.push_str("[workspace.dependencies]\n");
    for crate_name in crates {
        contents.push_str(&format!(
            "{crate_name} = {{ package = \"{}\", path = \"crates/{crate_name}\", version = \"0.15.0\" }}\n",
            renamed_typst_package_name(crate_name)
        ));
    }

    let upstream_dependency_lines = workspace_dependency_lines(upstream_workspace_toml)?;
    let required_dependencies =
        required_workspace_dependencies(out_path_from_workspace(vendor_out)?)?;
    for dependency in required_dependencies {
        if crates.iter().any(|crate_name| crate_name == &dependency) {
            continue;
        }
        let line = upstream_dependency_lines.get(&dependency).ok_or_else(|| {
            format!("upstream Cargo.toml is missing [workspace.dependencies].{dependency}")
        })?;
        contents.push_str(line);
        contents.push('\n');
    }

    if let Some(lints) = extract_toml_section(upstream_workspace_toml, "workspace.lints.clippy") {
        contents.push('\n');
        contents.push_str(&lints);
        contents.push('\n');
    }

    Ok(contents)
}

fn out_path_from_workspace(out: &Path) -> Result<&Path> {
    if out.is_dir() {
        Ok(out)
    } else {
        Err(format!("vendor output does not exist: {}", out.display()))
    }
}

fn rewrite_workspace_package_section(section: &str) -> String {
    section
        .lines()
        .map(|line| {
            if line.trim_start().starts_with("rust-version = ") {
                "rust-version = \"1.91\" # Avenger vendor override for current toolchain"
                    .to_string()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn workspace_dependency_lines(source: &str) -> Result<BTreeMap<String, String>> {
    let section = extract_toml_section(source, "workspace.dependencies")
        .ok_or_else(|| "upstream Cargo.toml is missing [workspace.dependencies]".to_string())?;
    let mut lines = BTreeMap::new();

    for raw_line in section.lines().skip(1) {
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if let Some((key, _)) = line.split_once('=') {
            lines.insert(key.trim().to_string(), raw_line.trim().to_string());
        }
    }

    Ok(lines)
}

fn required_workspace_dependencies(out: &Path) -> Result<BTreeSet<String>> {
    let mut required = BTreeSet::new();

    for manifest in copied_crate_manifests(out)? {
        let source = fs::read_to_string(&manifest)
            .map_err(|err| format!("failed to read {}: {err}", manifest.display()))?;
        let mut in_dependency_section = false;
        for raw_line in source.lines() {
            let line = strip_comment(raw_line).trim();
            if line.starts_with('[') && line.ends_with(']') {
                in_dependency_section = is_dependency_section(line);
                continue;
            }
            if !in_dependency_section {
                continue;
            }
            if !line.contains("workspace = true") {
                continue;
            }
            let Some((key, _)) = line.split_once('=') else {
                continue;
            };
            required.insert(key.trim().to_string());
        }
    }

    Ok(required)
}

fn is_dependency_section(header: &str) -> bool {
    let Some(section) = header.strip_prefix('[').and_then(|s| s.strip_suffix(']')) else {
        return false;
    };
    matches!(
        section,
        "dependencies" | "dev-dependencies" | "build-dependencies"
    ) || section.ends_with(".dependencies")
        || section.ends_with(".dev-dependencies")
        || section.ends_with(".build-dependencies")
}

fn extract_toml_section(source: &str, section_name: &str) -> Option<String> {
    let header = format!("[{section_name}]");
    let mut section = Vec::new();
    let mut in_section = false;

    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if in_section {
                break;
            }
            in_section = trimmed == header;
        }
        if in_section {
            section.push(line);
        }
    }

    (!section.is_empty()).then(|| section.join("\n") + "\n")
}

fn copied_crate_manifests(out: &Path) -> Result<Vec<PathBuf>> {
    let crates_dir = out.join("crates");
    if !crates_dir.exists() {
        return Ok(Vec::new());
    }

    let mut manifests = Vec::new();
    let mut entries = fs::read_dir(&crates_dir)
        .map_err(|err| format!("failed to read {}: {err}", crates_dir.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|err| format!("failed to read entry in {}: {err}", crates_dir.display()))?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let manifest = entry.path().join("Cargo.toml");
        if manifest.exists() {
            manifests.push(manifest);
        }
    }

    Ok(manifests)
}

fn rewrite_copied_crate_manifests(
    out: &Path,
    crates: &[String],
    denied_dependencies: &[String],
    applied_patches: &mut Vec<String>,
) -> Result<()> {
    for crate_name in crates {
        let manifest = out.join("crates").join(crate_name).join("Cargo.toml");
        let mut source = fs::read_to_string(&manifest)
            .map_err(|err| format!("failed to read {}: {err}", manifest.display()))?;
        let original_source = source.clone();

        source = source.replacen(
            &format!("name = \"{crate_name}\""),
            &format!("name = \"{}\"", renamed_typst_package_name(crate_name)),
            1,
        );
        source = ensure_lib_crate_name(&source, &rust_crate_name(crate_name));
        source = remove_dev_dependency_sections(&source);
        if crate_name == "typst-syntax" {
            source = remove_workspace_dependency_line(&source, "toml");
        }
        for dependency in denied_dependencies {
            source = remove_workspace_dependency_line(&source, dependency);
        }

        if source != original_source {
            fs::write(&manifest, source)
                .map_err(|err| format!("failed to write {}: {err}", manifest.display()))?;
            applied_patches.push(format!("mechanical:rewrite-manifest:{crate_name}"));
        }
    }

    Ok(())
}

fn remove_dev_dependency_sections(source: &str) -> String {
    let mut out = Vec::new();
    let mut skip = false;

    for line in source.lines() {
        let trimmed = strip_comment(line).trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            skip = is_dev_dependency_section(trimmed);
        }
        if !skip {
            out.push(line);
        }
    }

    out.join("\n") + "\n"
}

fn is_dev_dependency_section(header: &str) -> bool {
    let Some(section) = header.strip_prefix('[').and_then(|s| s.strip_suffix(']')) else {
        return false;
    };
    section == "dev-dependencies" || section.ends_with(".dev-dependencies")
}

fn ensure_lib_crate_name(source: &str, crate_name: &str) -> String {
    if source.lines().any(|line| line.trim() == "[lib]") {
        if source
            .lines()
            .any(|line| line.trim_start().starts_with("name = "))
        {
            return source.to_string();
        }

        return source.replacen("[lib]\n", &format!("[lib]\nname = \"{crate_name}\"\n"), 1);
    }

    if let Some(index) = source.find("\n[dependencies]") {
        let mut out = String::with_capacity(source.len() + crate_name.len() + 20);
        out.push_str(&source[..index + 1]);
        out.push_str(&format!("[lib]\nname = \"{crate_name}\"\n"));
        out.push_str(&source[index + 1..]);
        out
    } else {
        format!("{source}\n[lib]\nname = \"{crate_name}\"\n")
    }
}

fn remove_workspace_dependency_line(source: &str, dependency: &str) -> String {
    source
        .lines()
        .filter(|line| {
            let stripped = strip_comment(line).trim();
            !stripped.starts_with(&format!("{dependency} = "))
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn rewrite_typst_syntax_without_toml(
    out: &Path,
    crates: &[String],
    applied_patches: &mut Vec<String>,
) -> Result<()> {
    if !crates.iter().any(|crate_name| crate_name == "typst-syntax") {
        return Ok(());
    }

    let package_rs = out.join("crates/typst-syntax/src/package.rs");
    let source = fs::read_to_string(&package_rs)
        .map_err(|err| format!("failed to read {}: {err}", package_rs.display()))?;
    let patched = source.replace(
        "#[serde(flatten)]\n    pub sections: BTreeMap<EcoString, toml::Table>,",
        "#[serde(flatten, skip_serializing)]\n    pub sections: BTreeMap<EcoString, IgnoredAny>,",
    );

    if patched != source {
        fs::write(&package_rs, patched)
            .map_err(|err| format!("failed to write {}: {err}", package_rs.display()))?;
        applied_patches.push("mechanical:typst-syntax-tool-info-without-toml".to_string());
    }

    Ok(())
}

fn rewrite_typst_library_math_subset(
    out: &Path,
    crates: &[String],
    applied_patches: &mut Vec<String>,
) -> Result<()> {
    if !crates
        .iter()
        .any(|crate_name| crate_name == "typst-library")
    {
        return Ok(());
    }

    patch_typst_library_loading(out, applied_patches)?;
    write_patch_file(
        out,
        "crates/typst-library/src/foundations/plugin.rs",
        typst_library_plugin_stub(),
        "mechanical:typst-library-stub-plugin",
        applied_patches,
    )?;
    patch_typst_library_text_mod(out, applied_patches)?;
    patch_typst_library_value_display(out, applied_patches)?;
    write_patch_file(
        out,
        "crates/typst-library/src/model/bibliography.rs",
        typst_library_bibliography_stub(),
        "mechanical:typst-library-stub-bibliography",
        applied_patches,
    )?;
    write_patch_file(
        out,
        "crates/typst-library/src/model/cite.rs",
        typst_library_cite_stub(),
        "mechanical:typst-library-stub-cite",
        applied_patches,
    )?;
    write_patch_file(
        out,
        "crates/typst-library/src/visualize/image/mod.rs",
        typst_library_image_stub(),
        "mechanical:typst-library-stub-image",
        applied_patches,
    )?;
    write_patch_file(
        out,
        "crates/typst-library/src/text/font/color.rs",
        typst_library_color_font_stub(),
        "mechanical:typst-library-stub-color-fonts",
        applied_patches,
    )?;
    patch_typst_library_font_variant_without_usvg(out, applied_patches)?;
    patch_typst_library_color_without_assets(out, applied_patches)?;

    Ok(())
}

fn rewrite_typst_layout_math_subset(
    out: &Path,
    crates: &[String],
    applied_patches: &mut Vec<String>,
) -> Result<()> {
    if !crates.iter().any(|crate_name| crate_name == "typst-layout") {
        return Ok(());
    }

    patch_typst_layout_export_math_fragment(out, applied_patches)?;
    patch_typst_layout_linebreak_without_assets(out, applied_patches)?;
    patch_typst_layout_rules_without_unsupported(out, applied_patches)?;

    Ok(())
}

fn patch_typst_library_loading(out: &Path, applied_patches: &mut Vec<String>) -> Result<()> {
    let path = out.join("crates/typst-library/src/loading/mod.rs");
    let mut source = read_patch_source(&path)?;
    let original = source.clone();

    for needle in [
        "#[path = \"cbor.rs\"]\nmod cbor_;\n",
        "#[path = \"csv.rs\"]\nmod csv_;\n",
        "#[path = \"toml.rs\"]\nmod toml_;\n",
        "#[path = \"yaml.rs\"]\nmod yaml_;\n",
        "pub use self::cbor_::*;\n",
        "pub use self::csv_::*;\n",
        "pub use self::toml_::*;\n",
        "pub use self::yaml_::*;\n",
        "    global.define_func::<csv>();\n",
        "    global.define_func::<toml>();\n",
        "    global.define_func::<yaml>();\n",
        "    global.define_func::<cbor>();\n",
    ] {
        source = source.replace(needle, "");
    }

    write_if_changed(
        &path,
        original,
        source,
        "mechanical:typst-library-prune-data-loaders",
        applied_patches,
    )
}

fn patch_typst_library_text_mod(out: &Path, applied_patches: &mut Vec<String>) -> Result<()> {
    let path = out.join("crates/typst-library/src/text/mod.rs");
    let mut source = read_patch_source(&path)?;
    let original = source.clone();

    for needle in [
        "mod raw;\n",
        "pub use self::raw::*;\n",
        "    global.define_elem::<RawElem>();\n",
    ] {
        source = source.replace(needle, "");
    }

    write_if_changed(
        &path,
        original,
        source,
        "mechanical:typst-library-prune-raw-text",
        applied_patches,
    )
}

fn patch_typst_library_value_display(out: &Path, applied_patches: &mut Vec<String>) -> Result<()> {
    let path = out.join("crates/typst-library/src/foundations/value.rs");
    let mut source = read_patch_source(&path)?;
    let original = source.clone();

    source = source.replace(
        "use crate::text::{RawContent, RawElem, TextElem};",
        "use crate::text::TextElem;",
    );
    source = source.replace(
        "            _ => RawElem::new(RawContent::Text(self.repr()))\n                .with_lang(Some(\"typc\".into()))\n                .with_block(false)\n                .pack(),",
        "            _ => TextElem::packed(self.repr()),",
    );

    write_if_changed(
        &path,
        original,
        source,
        "mechanical:typst-library-value-display-without-raw",
        applied_patches,
    )
}

fn patch_typst_library_font_variant_without_usvg(
    out: &Path,
    applied_patches: &mut Vec<String>,
) -> Result<()> {
    let path = out.join("crates/typst-library/src/text/font/variant.rs");
    let source = read_patch_source(&path)?;
    let original = source.clone();

    let mut patched = source;
    for impl_name in [
        "impl From<usvg::FontStyle> for FontStyle",
        "impl From<usvg::FontStretch> for FontStretch",
    ] {
        patched = remove_impl_block(&patched, impl_name);
    }

    write_if_changed(
        &path,
        original,
        patched,
        "mechanical:typst-library-font-variant-without-usvg",
        applied_patches,
    )
}

fn remove_impl_block(source: &str, needle: &str) -> String {
    let Some(start) = source.find(needle) else {
        return source.to_string();
    };

    let Some(end) = find_braced_block_end(source, start) else {
        return source.to_string();
    };
    let mut end = end;
    while source[end..].starts_with('\n') {
        end += 1;
    }
    format!("{}{}", &source[..start], &source[end..])
}

fn find_braced_block_end(source: &str, start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut seen_open = false;
    for (offset, ch) in source[start..].char_indices() {
        match ch {
            '{' => {
                seen_open = true;
                depth += 1;
            }
            '}' if seen_open => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + offset + ch.len_utf8());
                }
            }
            _ => {}
        }
    }

    None
}

fn patch_typst_library_color_without_assets(
    out: &Path,
    applied_patches: &mut Vec<String>,
) -> Result<()> {
    let path = out.join("crates/typst-library/src/visualize/color.rs");
    let mut source = read_patch_source(&path)?;
    let original = source.clone();

    source = source.replace("use std::sync::{Arc, LazyLock};", "use std::sync::Arc;");
    source = source.replace(
        "use moxcms::{ColorProfile, Layout, RenderingIntent, TransformOptions};\n",
        "",
    );
    let profile_block = r#"/// The ICC profile used to convert from CMYK to RGB.
///
/// This is a minimal CMYK profile that only contains the necessary information
/// to convert from CMYK to RGB. It is based on the CGATS TR 001-1995
/// specification. See
/// <https://github.com/saucecontrol/Compact-ICC-Profiles#cmyk>.
static CMYK_TO_XYZ: LazyLock<ColorProfile> = LazyLock::new(|| {
    ColorProfile::new_from_slice(typst_assets::icc::CMYK_TO_XYZ).unwrap()
});

/// The target sRGB profile.
static SRGB_PROFILE: LazyLock<ColorProfile> = LazyLock::new(ColorProfile::new_srgb);

static TO_SRGB: LazyLock<Arc<moxcms::Transform8BitExecutor>> = LazyLock::new(|| {
    CMYK_TO_XYZ
        .create_transform_8bit(
            Layout::Rgba,
            &SRGB_PROFILE,
            Layout::Rgb,
            TransformOptions {
                // Our input profile only supports perceptual intent.
                rendering_intent: RenderingIntent::Perceptual,
                ..TransformOptions::default()
            },
        )
        .unwrap()
});

"#;
    source = source.replace(
        profile_block,
        "// The math subset uses only process colors needed for glyph and shape fills.\n",
    );
    source = replace_cmyk_to_rgba(&source);

    write_if_changed(
        &path,
        original,
        source,
        "mechanical:typst-library-color-without-assets",
        applied_patches,
    )
}

fn replace_cmyk_to_rgba(source: &str) -> String {
    let Some(start) = source.find("    fn to_rgba(self) -> Rgb {\n        let mut dest") else {
        return source.to_string();
    };
    let Some(end) = find_braced_block_end(source, start) else {
        return source.to_string();
    };
    let replacement = r#"    fn to_rgba(self) -> Rgb {
        let r = (1.0 - self.c) * (1.0 - self.k);
        let g = (1.0 - self.m) * (1.0 - self.k);
        let b = (1.0 - self.y) * (1.0 - self.k);
        Rgb::new(r, g, b, 1.0)
    }"#;

    format!("{}{}{}", &source[..start], replacement, &source[end..])
}

fn patch_typst_layout_linebreak_without_assets(
    out: &Path,
    applied_patches: &mut Vec<String>,
) -> Result<()> {
    let path = out.join("crates/typst-layout/src/inline/linebreak.rs");
    let mut source = read_patch_source(&path)?;
    let original = source.clone();

    source = source.replace("use icu_provider_blob::BlobDataProvider;\n", "");
    source = source.replace(
        "use icu_segmenter::{LineSegmenter, LineSegmenterBorrowed};",
        "use icu_segmenter::{LineSegmenter, LineSegmenterBorrowed};",
    );
    source = source.replace(
        r#"static CJ_SEGMENTER: LazyLock<LineSegmenter> = LazyLock::new(|| {
    let blob = typst_assets::icu::ICU_CJ_SEGMENT;
    let cj_provider = BlobDataProvider::try_new_from_static_blob(blob).unwrap();
    LineSegmenter::try_new_for_non_complex_scripts_with_buffer_provider(
        &cj_provider,
        LineBreakOptions::default(),
    )
    .unwrap()
});"#,
        r#"static CJ_SEGMENTER: LazyLock<LineSegmenterBorrowed> =
    LazyLock::new(|| LineSegmenter::new_auto(LineBreakOptions::default()));"#,
    );
    source = source.replace(
        "        Some(Lang::CHINESE | Lang::JAPANESE) => CJ_SEGMENTER.as_borrowed(),",
        "        Some(Lang::CHINESE | Lang::JAPANESE) => *CJ_SEGMENTER,",
    );

    write_if_changed(
        &path,
        original,
        source,
        "mechanical:typst-layout-linebreak-without-assets",
        applied_patches,
    )
}

fn patch_typst_layout_rules_without_unsupported(
    out: &Path,
    applied_patches: &mut Vec<String>,
) -> Result<()> {
    let path = out.join("crates/typst-layout/src/rules.rs");
    let mut source = read_patch_source(&path)?;
    let original = source.clone();

    for needle in [
        "    rules.register(Paged, CITE_GROUP_RULE);\n",
        "    rules.register(Paged, BIBLIOGRAPHY_RULE);\n",
        "    rules.register(Paged, CSL_LIGHT_RULE);\n",
        "    rules.register(Paged, CSL_INDENT_RULE);\n",
        "    rules.register(Paged, RAW_RULE);\n",
        "    rules.register(Paged, RAW_LINE_RULE);\n",
        "    rules.register(Paged, IMAGE_RULE);\n",
    ] {
        source = source.replace(needle, "");
    }

    source = source.replace(
        "    OverlineElem, RawElem, RawLine, ScriptKind, ShiftSettings, Smallcaps, SmallcapsElem,\n",
        "    OverlineElem, ScriptKind, ShiftSettings, Smallcaps, SmallcapsElem,\n",
    );
    source = source.replace(
        "    Attribution, BibliographyElem, CiteElem, CiteGroup, CslIndentElem, CslLightElem,\n    Destination, DirectLinkElem, DividerElem, EmphElem, EnumElem, FigureCaption,\n",
        "    Attribution, CiteElem, Destination, DirectLinkElem, DividerElem, EmphElem, EnumElem, FigureCaption,\n",
    );
    source = source.replace(
        "    TableCell, TableElem, TermsElem, TitleElem, Works,\n",
        "    TableCell, TableElem, TermsElem, TitleElem,\n",
    );
    source = source.replace(
        "    CircleElem, CurveElem, EllipseElem, ImageElem, LineElem, PolygonElem, RectElem,\n",
        "    CircleElem, CurveElem, EllipseElem, LineElem, PolygonElem, RectElem,\n",
    );

    source = remove_const_block(&source, "CITE_GROUP_RULE");
    source = remove_const_block(&source, "BIBLIOGRAPHY_RULE");
    source = remove_const_block(&source, "CSL_LIGHT_RULE");
    source = remove_const_block(&source, "CSL_INDENT_RULE");
    source = remove_const_block(&source, "RAW_RULE");
    source = remove_const_block(&source, "RAW_LINE_RULE");
    source = remove_const_block(&source, "IMAGE_RULE");

    write_if_changed(
        &path,
        original,
        source,
        "mechanical:typst-layout-unregister-unsupported",
        applied_patches,
    )
}

fn patch_typst_layout_export_math_fragment(
    out: &Path,
    applied_patches: &mut Vec<String>,
) -> Result<()> {
    let path = out.join("crates/typst-layout/src/lib.rs");
    let source = read_patch_source(&path)?;
    let original = source.clone();
    let needle = "pub use self::introspect::PagedIntrospector;\n";
    let source = if source.contains("pub use self::math::layout_equation_inline;\n") {
        source
    } else {
        source.replace(
            needle,
            "pub use self::introspect::PagedIntrospector;\npub use self::math::layout_equation_inline;\n",
        )
    };

    write_if_changed(
        &path,
        original,
        source,
        "mechanical:typst-layout-export-math-fragment",
        applied_patches,
    )
}

fn remove_const_block(source: &str, name: &str) -> String {
    let needle = format!("const {name}:");
    let Some(start) = source.find(&needle) else {
        return source.to_string();
    };
    let Some(relative_end) = source[start..].find("\n\nconst ") else {
        return source[..start].to_string();
    };
    let end = start + relative_end + 2;
    format!("{}{}", &source[..start], &source[end..])
}

fn read_patch_source(path: &Path) -> Result<String> {
    fs::read_to_string(path).map_err(|err| format!("failed to read {}: {err}", path.display()))
}

fn write_if_changed(
    path: &Path,
    original: String,
    mut patched: String,
    patch_name: &str,
    applied_patches: &mut Vec<String>,
) -> Result<()> {
    normalize_trailing_newline(&mut patched);
    if patched != original {
        fs::write(path, patched)
            .map_err(|err| format!("failed to write {}: {err}", path.display()))?;
        applied_patches.push(patch_name.to_string());
    }
    Ok(())
}

fn write_patch_file(
    out: &Path,
    rel_path: &str,
    contents: &str,
    patch_name: &str,
    applied_patches: &mut Vec<String>,
) -> Result<()> {
    let path = out.join(rel_path);
    let original = fs::read_to_string(&path).unwrap_or_default();
    write_if_changed(
        &path,
        original,
        contents.to_string(),
        patch_name,
        applied_patches,
    )
}

fn typst_library_plugin_stub() -> &'static str {
    r#"use ecow::EcoString;
use typst_syntax::Spanned;

use crate::diag::{SourceResult, StrResult, bail};
use crate::engine::Engine;
use crate::foundations::{Bytes, Func, Module, Value, cast, func, scope};
use crate::loading::DataSource;

#[func(scope)]
pub fn plugin(
    engine: &mut Engine,
    /// A path to a WebAssembly file or raw WebAssembly bytes.
    _source: Spanned<DataSource>,
) -> SourceResult<Module> {
    let _ = engine;
    bail!(typst_syntax::Span::detached(), "plugins are not available in avenger-typst math fragments")
}

#[scope]
impl plugin {
    #[func]
    pub fn transition(
        /// The plugin function to call.
        func: PluginFunc,
        /// The byte buffers to call the function with.
        #[variadic]
        arguments: Vec<Bytes>,
    ) -> StrResult<Module> {
        func.transition(arguments)
    }
}

/// A function loaded from a WebAssembly plugin.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct PluginFunc {
    name: EcoString,
}

impl PluginFunc {
    pub fn name(&self) -> &EcoString {
        &self.name
    }

    pub fn call(&self, _: Vec<Bytes>) -> StrResult<Bytes> {
        bail!("plugins are not available in avenger-typst math fragments")
    }

    pub fn transition(&self, _: Vec<Bytes>) -> StrResult<Module> {
        bail!("plugins are not available in avenger-typst math fragments")
    }
}

cast! {
    PluginFunc,
    self => Value::Func(self.into()),
    v: Func => v.to_plugin().ok_or("expected plugin function")?.clone(),
}
"#
}

fn typst_library_bibliography_stub() -> &'static str {
    r#"use crate::diag::{SourceResult, bail};
use crate::engine::Engine;
use crate::foundations::{Cast, Content, Derived, elem};
use crate::introspection::{Locatable, Location};
use crate::layout::Length;
use typst_syntax::Span;

#[elem(Locatable)]
pub struct BibliographyElem {
    #[required]
    pub path: Content,
}

impl BibliographyElem {
    pub fn has(_: &mut Engine, _: crate::foundations::Label, _: Span) -> bool {
        false
    }
}

impl crate::foundations::Packed<BibliographyElem> {
    pub fn realize_title(&self, _: crate::foundations::StyleChain) -> Option<Content> {
        None
    }
}

#[derive(Debug, Default, Copy, Clone, Eq, PartialEq, Hash, Cast)]
pub enum CitationForm {
    #[default]
    Normal,
    Prose,
    Full,
    Author,
    Year,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct CslSource;

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct CslStyle;

pub struct Bibliography {
    pub entries: Vec<BibliographyEntry>,
    pub hanging_indent: bool,
}

pub struct BibliographyEntry {
    pub prefix: Option<Content>,
    pub body: Content,
    pub backlink: Location,
}

pub struct Works;

impl Works {
    pub fn generate(_: &mut Engine, _: Span) -> SourceResult<std::sync::Arc<Works>> {
        Ok(std::sync::Arc::new(Works))
    }

    pub fn bibliography(
        &self,
        _location: Location,
        span: Span,
    ) -> SourceResult<Bibliography> {
        bail!(span, "bibliographies are not available in avenger-typst math fragments")
    }

    pub fn citation(&self, _location: Location, span: Span) -> SourceResult<Content> {
        bail!(span, "citations are not available in avenger-typst math fragments")
    }
}

#[elem]
pub struct CslLightElem {
    #[required]
    pub body: Content,
}

#[elem]
pub struct CslIndentElem {
    #[required]
    pub body: Content,
    pub amount: Length,
}

impl CslStyle {
    pub fn load(
        _: &mut Engine,
        _: typst_syntax::Spanned<CslSource>,
    ) -> SourceResult<Derived<CslSource, CslStyle>> {
        bail!(typst_syntax::Span::detached(), "bibliographies are not available in avenger-typst math fragments")
    }
}
"#
}

fn typst_library_cite_stub() -> &'static str {
    r#"use crate::diag::{SourceResult, bail};
use crate::engine::Engine;
use crate::foundations::{Content, Label, Packed, StyleChain, Synthesize, cast, elem};
use crate::introspection::Locatable;
use crate::model::CitationForm;
use crate::text::{Lang, Region, TextElem};

#[elem(Locatable, Synthesize)]
pub struct CiteElem {
    #[required]
    pub key: Label,
    pub supplement: Option<Content>,
    #[default(Some(CitationForm::Normal))]
    pub form: Option<CitationForm>,
    #[internal]
    #[synthesized]
    pub lang: Lang,
    #[internal]
    #[synthesized]
    pub region: Option<Region>,
}

impl Synthesize for Packed<CiteElem> {
    fn synthesize(&mut self, _: &mut Engine, styles: StyleChain) -> SourceResult<()> {
        let elem = self.as_mut();
        elem.lang = Some(styles.get(TextElem::lang));
        elem.region = Some(styles.get(TextElem::region));
        Ok(())
    }
}

cast! {
    CiteElem,
    v: Content => v.unpack::<Self>().map_err(|_| "expected citation")?,
}

#[elem(Locatable)]
pub struct CiteGroup {
    #[required]
    pub children: Vec<Content>,
}

impl Packed<CiteGroup> {
    pub fn realize(&self, _: &mut Engine) -> SourceResult<Content> {
        bail!(self.span(), "citations are not available in avenger-typst math fragments")
    }
}
"#
}

fn typst_library_image_stub() -> &'static str {
    r#"//! Image handling is stubbed in the avenger-typst math subset.

use std::fmt::{self, Debug, Formatter};

use crate::diag::{SourceResult, bail};
use crate::engine::Engine;
use crate::foundations::{Bytes, Cast, Packed, Smart, StyleChain, Synthesize, elem};
use crate::introspection::{Locatable, Tagged};
use crate::layout::{Length, Rel, Sizing};
use crate::model::Figurable;
use crate::text::Locale;

#[elem(Locatable, Tagged, Synthesize, Figurable)]
pub struct ImageElem {
    pub width: Smart<Rel<Length>>,
    pub height: Sizing,
    #[default(ImageFit::Cover)]
    pub fit: ImageFit,
    #[internal]
    #[synthesized]
    pub locale: Locale,
}

impl Synthesize for Packed<ImageElem> {
    fn synthesize(&mut self, _: &mut Engine, styles: StyleChain) -> SourceResult<()> {
        self.as_mut().locale = Some(Locale::get_in(styles));
        Ok(())
    }
}

impl Packed<ImageElem> {
    pub fn decode(&self, _: &mut Engine, _: StyleChain) -> SourceResult<Image> {
        bail!(self.span(), "images are not available in avenger-typst math fragments")
    }
}

impl Figurable for Packed<ImageElem> {}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Cast)]
pub enum ImageFit {
    Cover,
    Contain,
    Stretch,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Cast)]
pub enum ImageScaling {
    Smooth,
    Pixelated,
}

#[derive(Clone, Eq, PartialEq, Hash)]
pub struct Image;

impl Image {
    pub const DEFAULT_DPI: f64 = 72.0;
    pub const USVG_DEFAULT_DPI: f64 = 96.0;

    pub fn plain(_: impl Into<ImageKind>) -> Self {
        Self
    }

    pub fn width(&self) -> f64 {
        1.0
    }

    pub fn height(&self) -> f64 {
        1.0
    }

    pub fn dpi(&self) -> Option<f64> {
        Some(Self::DEFAULT_DPI)
    }
}

impl Debug for Image {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        f.pad("Image(..)")
    }
}

#[derive(Clone, Eq, PartialEq, Hash)]
pub enum ImageKind {
    Raster(RasterImage),
    Svg(SvgImage),
}

impl From<RasterImage> for ImageKind {
    fn from(image: RasterImage) -> Self {
        Self::Raster(image)
    }
}

impl From<SvgImage> for ImageKind {
    fn from(image: SvgImage) -> Self {
        Self::Svg(image)
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Cast)]
pub enum ExchangeFormat {
    Png,
    Jpg,
    Gif,
    Webp,
}

#[derive(Clone, Eq, PartialEq, Hash)]
pub struct RasterImage;

impl RasterImage {
    pub fn plain(_: Bytes, _: ExchangeFormat) -> crate::diag::StrResult<Self> {
        Ok(Self)
    }
}

#[derive(Clone, Eq, PartialEq, Hash)]
pub struct SvgImage;

impl SvgImage {
    pub fn new(_: Bytes) -> crate::diag::StrResult<Self> {
        Ok(Self)
    }
}
"#
}

fn typst_library_color_font_stub() -> &'static str {
    r#"//! Minimal color font handling for the avenger-typst math subset.

use ttf_parser::GlyphId;

use crate::layout::{Abs, Frame, FrameItem, Point, Size};
use crate::text::FontInstance;
use crate::visualize::{FixedStroke, Geometry};
use typst_syntax::Span;

/// Whether this glyph should be rendered via simple outlining instead of via
/// `glyph_frame`.
pub fn should_outline(font: &FontInstance, glyph_id: GlyphId) -> bool {
    let ttf = font.ttf();
    ttf.tables().glyf.is_some()
        || ttf.tables().cff.is_some()
        || ttf.tables().cff2.is_some()
        || !ttf.is_color_glyph(glyph_id)
}

/// A frame that can draw a glyph.
#[derive(Clone)]
pub struct GlyphFrame {
    pub upem: Abs,
    pub item: GlyphFrameItem,
}

impl GlyphFrame {
    pub fn size(&self) -> Size {
        Size::splat(self.upem)
    }
}

impl From<GlyphFrame> for Frame {
    fn from(g: GlyphFrame) -> Self {
        let mut frame = Frame::soft(Size::splat(g.upem));
        match g.item {
            GlyphFrameItem::Tofu(pos, shape) => {
                frame.push(pos, FrameItem::Shape(shape, Span::detached()))
            }
        }
        frame
    }
}

#[derive(Clone)]
pub enum GlyphFrameItem {
    Tofu(Point, crate::visualize::Shape),
}

impl GlyphFrameItem {
    pub fn pos(&self) -> Point {
        match *self {
            GlyphFrameItem::Tofu(pos, _) => pos,
        }
    }
}

#[comemo::memoize]
pub fn glyph_frame(font: &FontInstance, glyph_id: u16) -> Option<GlyphFrame> {
    let upem = Abs::pt(font.units_per_em());
    Some(draw_fallback_tofu(font, upem, GlyphId(glyph_id)))
}

fn draw_fallback_tofu(font: &FontInstance, upem: Abs, glyph_id: GlyphId) -> GlyphFrame {
    let advance = font
        .ttf()
        .glyph_hor_advance(glyph_id)
        .map(|advance| Abs::pt(advance as f64))
        .unwrap_or(upem / 3.0);
    let inset = 0.15 * advance;
    let height = 0.7 * upem;
    let pos = Point::new(inset, upem - height);
    let size = Size::new(advance - inset * 2.0, height);
    let thickness = upem / 20.0;
    let stroke = FixedStroke { thickness, ..Default::default() };
    let shape = Geometry::Rect(size).stroked(stroke);
    GlyphFrame { upem, item: GlyphFrameItem::Tofu(pos, shape) }
}
"#
}

fn renamed_typst_package_name(crate_name: &str) -> String {
    format!("avenger-{crate_name}")
}

fn rust_crate_name(crate_name: &str) -> String {
    crate_name.replace('-', "_")
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
