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
    font_weight: u16,
    text_font: String,
    math_font: String,
    #[serde(default)]
    requires_system_emoji: bool,
}

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .ok_or("failed to resolve repository root")?;
    let typst_cli = repo_root
        .parent()
        .ok_or("failed to resolve ../typst path")?
        .join("typst/crates/typst-cli/Cargo.toml");
    if !typst_cli.is_file() {
        return Err(format!(
            "missing upstream Typst CLI manifest at {}; expected ../typst checkout",
            typst_cli.display()
        )
        .into());
    }

    let fixtures_dir = manifest_dir.join("tests/fixtures/upstream_png");
    let cases_path = fixtures_dir.join("cases.toml");
    let mut cases: Cases = toml::from_str(&fs::read_to_string(&cases_path)?)?;
    cases.case.sort_by(|left, right| left.id.cmp(&right.id));

    let target_dir = repo_root.join("target/typst-parity");
    let font_dir = target_dir.join("fonts");
    let source_dir = target_dir.join("src");
    fs::create_dir_all(&font_dir)?;
    fs::remove_dir_all(&source_dir).ok();
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(fixtures_dir.join("ref"))?;

    prepare_fonts(
        repo_root,
        &font_dir,
        cases.case.iter().any(|case| case.requires_system_emoji),
    )?;

    for case in &cases.case {
        let source = read_label_source(&fixtures_dir.join("src").join(&case.source))?;
        let wrapped_source = wrap_source(case, &source);
        let wrapped_path = source_dir.join(format!("{}.typ", case.id));
        let ref_path = fixtures_dir.join("ref").join(format!("{}.png", case.id));
        fs::write(&wrapped_path, wrapped_source)?;

        let status = Command::new("cargo")
            .arg("run")
            .arg("--release")
            .arg("--manifest-path")
            .arg(&typst_cli)
            .arg("--")
            .arg("compile")
            .arg("--font-path")
            .arg(&font_dir)
            .arg("--ignore-system-fonts")
            .arg("--ppi")
            .arg("72")
            .arg(&wrapped_path)
            .arg(&ref_path)
            .status()?;

        if !status.success() {
            return Err(format!(
                "upstream Typst failed while generating reference for {}",
                case.id
            )
            .into());
        }
    }

    Ok(())
}

fn prepare_fonts(
    repo_root: &Path,
    font_dir: &Path,
    include_system_emoji: bool,
) -> Result<(), Box<dyn Error>> {
    let fonts = [
        "avenger-text/fonts/Lato/Lato-Light.ttf.br",
        "avenger-text/fonts/Lato/Lato-Italic.ttf.br",
        "avenger-text/fonts/Lato/Lato-Medium.ttf.br",
        "avenger-text/fonts/Lato/Lato-Bold.ttf.br",
        "avenger-text/fonts/DejaVu_Sans_Mono/DejaVuSansMono.ttf.br",
        "avenger-text/fonts/Lete_Sans_Math/LeteSansMath.otf.br",
        "avenger-text/fonts/Lete_Sans_Math/LeteSansMath-Bold.otf.br",
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

fn wrap_source(case: &Case, source: &str) -> String {
    format!(
        r#"#set page(width: auto, height: 120pt, margin: 20pt, fill: white)
#set align(horizon)
#set text(font: "{}", size: {}pt, weight: {}, fill: black)
#show math.equation: set text(font: "{}", weight: {})
{}
"#,
        case.text_font, case.font_size, case.font_weight, case.math_font, case.font_weight, source
    )
}
