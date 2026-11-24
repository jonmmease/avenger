//! `avenger quarto2-engine`: the cells of a Quarto 2 document for the avenger engine extension of
//! q2, which runs it. It reads one request on standard input and writes one response on standard
//! output, which carries nothing else, as JSON.
//!
//! The request names the document, whose project the cells run in, and holds its text as q2 passes
//! it to the engine, which can differ from the file: q2 adds its front matter and writes a labeled
//! cell's figure, and a document can include another file. The cells of the text run in order as
//! one script, as `avenger render` runs a document's cells, and each value draws to SVG or PNG. The
//! response holds each cell's number, the line of its opening fence, and its outputs, in nbformat
//! 4, which the extension gives q2's core to write.
//!
//! A request in another protocol, or one whose document or project doesn't load, writes an error
//! and exits with status 1, as does a failing cell whose errors are no output.

use std::path::PathBuf;

use avenger_sql_project::qmd::{FigFormat, Qmd};
use avenger_style::ColorScheme;
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    cells,
    render::{image, Failure, Format, Options, Project, Renderer},
    report::{Output, Report},
};

/// The protocol of the requests and the responses, which a change of either raises after the
/// first release of avenger with `quarto2-init`.
pub const PROTOCOL: u32 = 1;

/// Pixels to the inch of a cell's `fig-width` and `fig-height`, as CSS has it.
const PIXELS_PER_INCH: f64 = 96.0;

/// A request of the extension, whose `protocol` the request's text answers first.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Request {
    document: PathBuf,
    text: String,
    #[serde(default)]
    image: ImageDefault,
    #[serde(default)]
    color_scheme: Option<String>,
    /// The document's `execute: error`.
    #[serde(default)]
    error: bool,
}

/// The image of a cell without its own `fig-format`.
#[derive(Debug, Deserialize)]
struct ImageDefault {
    format: String,
    #[serde(default = "one")]
    scale: f32,
}

fn one() -> f32 {
    1.0
}

impl Default for ImageDefault {
    fn default() -> Self {
        ImageDefault {
            format: "svg".to_string(),
            scale: 1.0,
        }
    }
}

/// The response to the request on standard input, and whether it succeeded.
pub(crate) fn answer() -> (Value, bool) {
    match serve() {
        Ok(cells) => (json!({"protocol": PROTOCOL, "cells": cells}), true),
        Err(error) => (json!({"protocol": PROTOCOL, "error": error}), false),
    }
}

/// The cells of the request on standard input.
fn serve() -> Result<Vec<Value>, String> {
    use std::io::Read;
    let mut input = String::new();
    (std::io::stdin().read_to_string(&mut input)).map_err(|error| error.to_string())?;
    // The futures of a loaded project are not `Send`, so they run on this thread, and the chart
    // apps prepare their charts on threads of their own.
    let error = |error: std::io::Error| error.to_string();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(error)?;
    let apps = (tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build())
    .map_err(error)?;
    let cells = runtime.block_on(run(&input, apps.handle().clone()));
    apps.shutdown_background();
    cells
}

async fn run(input: &str, runtime: tokio::runtime::Handle) -> Result<Vec<Value>, String> {
    let value: Value =
        serde_json::from_str(input).map_err(|error| format!("the request is not JSON: {error}"))?;
    match value.get("protocol").and_then(Value::as_u64) {
        Some(protocol) if protocol == u64::from(PROTOCOL) => {}
        Some(protocol) if protocol > u64::from(PROTOCOL) => {
            return Err(format!(
                "this project's extension speaks protocol {protocol}, newer than this avenger's \
                 {PROTOCOL}: upgrade avenger"
            ))
        }
        Some(protocol) => {
            return Err(format!(
                "this project's extension speaks protocol {protocol}, and this avenger speaks \
                 {PROTOCOL}: run `avenger quarto2-init` to update it"
            ))
        }
        None => return Err("the request names no protocol".to_string()),
    }
    let request: Request =
        serde_json::from_value(value).map_err(|error| format!("the request: {error}"))?;
    let file = std::fs::canonicalize(&request.document)
        .map_err(|error| format!("{}: {error}", request.document.display()))?;
    let color_scheme = match request.color_scheme.as_deref() {
        None | Some("light") => ColorScheme::Light,
        Some("dark") => ColorScheme::Dark,
        Some(other) => return Err(format!("`colorScheme` `{other}`: light or dark")),
    };
    let default = match request.image.format.as_str() {
        "svg" => (Format::Svg, 1.0),
        "png" => (Format::Png, request.image.scale),
        other => return Err(format!("`image.format` `{other}`: svg or png")),
    };
    let document = Qmd::parse(request.text.as_str());
    let mut project = Project::open(&file, false, Some(request.text.clone())).await?;
    let statements = cells::statements(&document);
    let mut options = Options {
        size: [None, None],
        scale: 1.0,
        color_scheme,
        style: None,
        modes: Vec::new(),
        chart: None,
        rows: 20,
        timeout: std::time::Duration::from_secs(30),
        runtime,
    };
    let renderer = Renderer::new(&options)?;
    let chart = file.to_string_lossy().into_owned();
    let mut responses = Vec::with_capacity(document.cells.len());
    for (index, cell) in document.cells.iter().enumerate() {
        let number = index + 1;
        let line = document.text[..cell.range.start].matches('\n').count();
        let mut response = json!({"number": number, "line": line, "outputs": []});
        // A cell that `eval: false` keeps from running, or without a query, has no outputs.
        let runs = cell.options.eval != Some(false) && cells::value(&statements, index).is_some();
        if !runs {
            responses.push(response);
            continue;
        }
        let (format, scale) = match cell.options.fig_format {
            None => default,
            Some(FigFormat::Svg) => (Format::Svg, 1.0),
            Some(FigFormat::Png) => (Format::Png, request.image.scale),
            Some(FigFormat::Retina) => (Format::Png, 2.0),
        };
        let scale = match (format, cell.options.fig_dpi) {
            (Format::Png, Some(dpi)) => (dpi / PIXELS_PER_INCH) as f32,
            _ => scale,
        };
        let inches = |side: Option<f64>| side.map(|inches| (inches * PIXELS_PER_INCH) as f32);
        options.size = [
            inches(cell.options.fig_width),
            inches(cell.options.fig_height),
        ];
        options.scale = scale;
        let drawn = match project.aim(&format!("{chart}#{number}")) {
            Ok(aim) => {
                let target = aim.target(project);
                let drawn = image(&target, format, &options, &renderer).await;
                project = target.into_project();
                drawn
            }
            Err(failure) => Err(failure),
        };
        let output = match drawn {
            Ok(drawn) => display(format, drawn.bytes, drawn.size),
            Err(failure) => {
                let reports = match failure {
                    Failure::Reports(reports) => reports,
                    Failure::Usage(error) => {
                        vec![Report::file(
                            &file,
                            crate::report::Severity::Error,
                            error,
                            "run",
                        )]
                    }
                };
                if !cell.options.error.unwrap_or(request.error) {
                    return Err(traceback(&reports));
                }
                error_output(&reports)
            }
        };
        response["outputs"] = json!([output]);
        responses.push(response);
    }
    Ok(responses)
}

/// The `display_data` output of an image in `format`, whose bytes are `bytes` and whose size in
/// CSS pixels is `size`.
fn display(format: Format, bytes: Vec<u8>, size: [f32; 2]) -> Value {
    match format {
        Format::Png => json!({
            "output_type": "display_data",
            "data": {
                "image/png": base64::engine::general_purpose::STANDARD.encode(bytes),
                "text/plain": "chart",
            },
            "metadata": {"image/png": {"width": size[0], "height": size[1]}},
        }),
        _ => json!({
            "output_type": "display_data",
            "data": {
                "image/svg+xml": String::from_utf8_lossy(&bytes),
                "text/plain": "chart",
            },
            "metadata": {},
        }),
    }
}

/// The `error` output of a cell whose errors are `reports`.
fn error_output(reports: &[Report]) -> Value {
    let evalue = reports.first().map_or("", |report| report.message.as_str());
    let traceback: Vec<String> = traceback(reports).lines().map(str::to_string).collect();
    json!({
        "output_type": "error",
        "ename": "avenger",
        "evalue": evalue,
        "traceback": traceback,
    })
}

/// `reports` as `avenger check` prints them, without colors.
fn traceback(reports: &[Report]) -> String {
    let mut text = Vec::new();
    Output::new(false).write_diagnostics(&mut text, reports.to_vec());
    String::from_utf8_lossy(&text).trim_end().to_string()
}
