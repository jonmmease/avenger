//! Regenerates the upstream PNG references in `tests/fixtures/upstream_png/ref`.
//!
//! ```sh
//! cargo run --release -p avenger-typst-label --features upstream-png-parity --bin generate_upstream_png_refs
//! cargo run --release -p avenger-typst-label --features upstream-png-parity --bin generate_upstream_png_refs -- --check
//! ```
//!
//! The Typst CLI must be the release pinned in `tests/fixtures/typst-pin.toml`: either
//! `TYPST_BIN`, or a `--locked` build of the `../typst` checkout at the pinned commit. `--check`
//! renders into `target/typst-parity/check` and fails if any reference differs, without writing
//! to `ref/`.

use std::{
    error::Error,
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    process::Command,
};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Cases {
    case: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct Case {
    id: String,
    source: String,
    font_size: f32,
    #[serde(default = "default_scale")]
    scale: f32,
    font_weight: u16,
    text_font: String,
    math_font: String,
    #[serde(default)]
    requires_system_emoji: bool,
}

fn default_scale() -> f32 {
    2.0
}

#[derive(Debug, Deserialize)]
struct Pin {
    version: String,
    commit: String,
}

/// The page margin around the label box, in points. The reference's logical size is its page
/// size minus twice this.
const MARGIN_PT: f32 = 128.0;

fn main() -> Result<(), Box<dyn Error>> {
    let check = match std::env::args().nth(1).as_deref() {
        None => false,
        Some("--check") => true,
        Some(other) => return Err(format!("unknown argument {other}; expected --check").into()),
    };

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .ok_or("failed to resolve repository root")?;
    let typst_dir = repo_root
        .parent()
        .ok_or("failed to resolve ../typst path")?
        .join("typst");
    let fixtures_dir = manifest_dir.join("tests/fixtures");
    let pin: Pin = toml::from_str(&fs::read_to_string(fixtures_dir.join("typst-pin.toml"))?)?;

    let cli = TypstCli::resolve(&typst_dir, &pin)?;

    let png_dir = fixtures_dir.join("upstream_png");
    let mut cases: Cases = toml::from_str(&fs::read_to_string(png_dir.join("cases.toml"))?)?;
    cases.case.sort_by(|left, right| left.id.cmp(&right.id));

    let target_dir = repo_root.join("target/typst-parity");
    let font_dir = target_dir.join("fonts");
    let source_dir = target_dir.join("src");
    let out_dir = if check {
        target_dir.join("check")
    } else {
        png_dir.join("ref")
    };
    fs::create_dir_all(&font_dir)?;
    fs::remove_dir_all(&source_dir).ok();
    fs::create_dir_all(&source_dir)?;
    if check {
        fs::remove_dir_all(&out_dir).ok();
    }
    fs::create_dir_all(&out_dir)?;

    prepare_fonts(
        repo_root,
        &font_dir,
        cases.case.iter().any(|case| case.requires_system_emoji),
    )?;

    for case in &cases.case {
        let source = read_label_source(&png_dir.join("src").join(&case.source))?;
        let wrapped_path = source_dir.join(format!("{}.typ", case.id));
        fs::write(&wrapped_path, wrap_source(case, &source))?;
        let status = cli
            .command()
            .arg("compile")
            .arg("--font-path")
            .arg(&font_dir)
            .arg("--ignore-system-fonts")
            .arg("--ignore-embedded-fonts")
            .arg("--ppi")
            .arg((72.0 * case.scale).to_string())
            .arg(&wrapped_path)
            .arg(out_dir.join(format!("{}.png", case.id)))
            .status()?;
        if !status.success() {
            return Err(format!(
                "upstream Typst failed while generating reference for {}",
                case.id
            )
            .into());
        }
    }

    if check {
        let mut stale = Vec::new();
        for case in &cases.case {
            let name = format!("{}.png", case.id);
            if fs::read(out_dir.join(&name)).ok() != fs::read(png_dir.join("ref").join(&name)).ok()
            {
                stale.push(name);
            }
        }
        if !stale.is_empty() {
            return Err(format!(
                "{} references differ from upstream Typst {} (fresh renders in {}): {}",
                stale.len(),
                pin.version,
                out_dir.display(),
                stale.join(", ")
            )
            .into());
        }
        println!(
            "all {} references match upstream Typst {}",
            cases.case.len(),
            pin.version
        );
    }
    Ok(())
}

/// How to run the pinned Typst CLI.
enum TypstCli {
    Binary(PathBuf),
    Cargo(PathBuf),
}

impl TypstCli {
    /// Uses `TYPST_BIN` when set, otherwise a `--locked` build of `../typst`, and checks that it
    /// is the pinned release.
    fn resolve(typst_dir: &Path, pin: &Pin) -> Result<Self, Box<dyn Error>> {
        let cli = match std::env::var_os("TYPST_BIN") {
            Some(binary) => Self::Binary(PathBuf::from(binary)),
            None => {
                let manifest = typst_dir.join("crates/typst-cli/Cargo.toml");
                if !manifest.is_file() {
                    return Err(format!(
                        "missing upstream Typst CLI manifest at {}; expected a ../typst checkout",
                        manifest.display()
                    )
                    .into());
                }
                let head = Command::new("git")
                    .arg("-C")
                    .arg(typst_dir)
                    .args(["rev-parse", "HEAD"])
                    .output()?;
                let head = String::from_utf8(head.stdout)?;
                if head.trim() != pin.commit {
                    return Err(format!(
                        "../typst is at {}, but references are pinned to Typst {} ({}); run `git -C ../typst checkout v{}` or set TYPST_BIN",
                        head.trim(),
                        pin.version,
                        pin.commit,
                        pin.version
                    )
                    .into());
                }
                Self::Cargo(manifest)
            }
        };

        let output = cli.command().arg("--version").output()?;
        let version = String::from_utf8(output.stdout)?;
        let expected = format!("typst {} ({})", pin.version, &pin.commit[..8]);
        if !output.status.success() || version.trim() != expected {
            return Err(format!(
                "the Typst CLI reports {:?}, but references are pinned to {expected:?}",
                version.trim()
            )
            .into());
        }
        Ok(cli)
    }

    fn command(&self) -> Command {
        match self {
            Self::Binary(binary) => Command::new(binary),
            Self::Cargo(manifest) => {
                let mut command = Command::new("cargo");
                command
                    .args(["run", "--quiet", "--release", "--locked", "--manifest-path"])
                    .arg(manifest)
                    .arg("--");
                command
            }
        }
    }
}

fn prepare_fonts(
    repo_root: &Path,
    font_dir: &Path,
    include_system_emoji: bool,
) -> Result<(), Box<dyn Error>> {
    let fonts = [
        "avenger-fonts/fonts/Lato/Lato-Light.ttf.br",
        "avenger-fonts/fonts/Lato/Lato-Italic.ttf.br",
        "avenger-fonts/fonts/Lato/Lato-Medium.ttf.br",
        "avenger-fonts/fonts/Lato/Lato-Bold.ttf.br",
        "avenger-fonts/fonts/DejaVu_Sans_Mono/DejaVuSansMono.ttf.br",
        "avenger-fonts/fonts/Lete_Sans_Math/LeteSansMath.otf.br",
        "avenger-fonts/fonts/Lete_Sans_Math/LeteSansMath-Bold.otf.br",
    ];

    for relative_path in fonts {
        let source = repo_root.join(relative_path);
        let file_name = source
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("invalid font path {}", source.display()))?;
        let output_name = file_name
            .strip_suffix(".br")
            .ok_or_else(|| format!("expected brotli font path to end in .br: {file_name}"))?;
        decompress_brotli_file(&source, &font_dir.join(output_name))?;
    }

    for entry in fs::read_dir(repo_root.join("avenger-typst-label/tests/fixtures/fonts"))? {
        let source = entry?.path();
        if source
            .extension()
            .is_some_and(|extension| extension == "br")
        {
            decompress_brotli_file(&source, &font_dir.join(source.file_stem().unwrap()))?;
        }
    }

    if include_system_emoji {
        let macos_emoji = Path::new("/System/Library/Fonts/Apple Color Emoji.ttc");
        if macos_emoji.is_file() {
            fs::copy(macos_emoji, font_dir.join("Apple Color Emoji.ttc"))?;
        } else {
            return Err(format!(
                "emoji parity case requested, but {} is unavailable",
                macos_emoji.display()
            )
            .into());
        }
    }

    Ok(())
}

fn decompress_brotli_file(source: &Path, destination: &Path) -> Result<(), Box<dyn Error>> {
    let compressed = fs::read(source)?;
    let mut decompressor = brotli::Decompressor::new(Cursor::new(compressed), 4096);
    let mut decompressed = Vec::new();
    decompressor.read_to_end(&mut decompressed)?;
    if decompressed.is_empty() {
        return Err(format!("font {} decompressed to empty data", source.display()).into());
    }
    fs::write(destination, decompressed)?;
    Ok(())
}

fn read_label_source(path: &Path) -> Result<String, Box<dyn Error>> {
    Ok(fs::read_to_string(path)?
        .trim_end_matches(['\r', '\n'])
        .to_string())
}

/// Puts the label in a box on an auto-sized page, so the page is the box plus `MARGIN_PT` on
/// every side.
fn wrap_source(case: &Case, source: &str) -> String {
    format!(
        "#set page(width: auto, height: auto, margin: {MARGIN_PT}pt, fill: white)\n#set text(font: {:?}, size: {}pt, weight: {})\n#show math.equation: set text(font: {:?}, weight: {})\n#show raw: set text(font: \"DejaVu Sans Mono\")\n#box[{}]\n",
        case.text_font, case.font_size, case.font_weight, case.math_font, case.font_weight, source
    )
}
