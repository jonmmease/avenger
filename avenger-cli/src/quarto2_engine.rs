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
//! A value draws in each color scheme of the request. A document with a light and a dark theme
//! asks for both, and each value then has an image of each scheme, on a transparent background,
//! whose class, `light-content` or `dark-content`, shows it only in its scheme.
//!
//! An interactive request, of an HTML format that runs scripts, gives each chart that declares a
//! tool, a widget, or a mode an `<avenger-chart>` element too, after its images, which draws the
//! chart in a canvas with its tools. The engine then writes the runtime of interactive charts to
//! the q2 project's `_avenger/` folder, and the response holds the page's `header`, which loads
//! the runtime once, when the first chart nears the screen. A chart whose message is too large to
//! hold in a page, or a runtime that does not write, leaves the charts as images, and the
//! response's `warnings` say so.
//!
//! A request in another protocol, or one whose document or project doesn't load, writes an error
//! and exits with status 1, as does a failing cell whose errors are no output.

use std::path::PathBuf;

use avenger_chart::{results::Shape, Chart};
use avenger_sql_project::{
    qmd::{FigFormat, Qmd},
    ChartQuery,
};
use avenger_style::ColorScheme;
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    cells, quarto2_runtime,
    render::{
        collect_target, plan_target, result_image, Failure, Format, Image, Options, Planned,
        Project, Renderer, Target,
    },
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
    /// The color schemes of the document's themes, each `light` or `dark`.
    color_schemes: Vec<String>,
    /// The document's `execute: error`.
    #[serde(default)]
    error: bool,
    /// Whether the output format runs scripts, so charts with tools draw in canvases.
    #[serde(default)]
    interactive: bool,
    /// The q2 project's folder, whose `_avenger/` holds the runtime of interactive charts.
    #[serde(default)]
    project_dir: Option<PathBuf>,
    /// Whether the format stretches a figure that is alone on a slide, as revealjs does, which
    /// lifts its image out and drops its other outputs unless the image has a set size.
    #[serde(default)]
    auto_stretch: bool,
}

/// The answer to a request: each cell's outputs, the header of a page with interactive charts,
/// and warnings, such as of a chart that stays an image.
struct Answer {
    cells: Vec<Value>,
    header: Option<String>,
    warnings: Vec<String>,
}

/// The largest message of a chart that a page holds inline for its canvas. A larger chart, whose
/// data would weigh the page down beside its images, stays an image.
const MAX_INTERACTIVE_BYTES: usize = 4 * 1024 * 1024;

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
        Ok(Answer {
            cells,
            header,
            warnings,
        }) => {
            let mut response = json!({"protocol": PROTOCOL, "cells": cells});
            if let Some(header) = header {
                response["header"] = Value::String(header);
            }
            if !warnings.is_empty() {
                response["warnings"] = json!(warnings);
            }
            (response, true)
        }
        Err(error) => (json!({"protocol": PROTOCOL, "error": error}), false),
    }
}

/// The answer to the request on standard input.
fn serve() -> Result<Answer, String> {
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

async fn run(input: &str, runtime: tokio::runtime::Handle) -> Result<Answer, String> {
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
    let schemes = color_schemes(&request.color_schemes)?;
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
        color_scheme: schemes[0],
        // Images of several schemes take the color of the page around them.
        transparent: schemes.len() > 1,
        style: None,
        modes: Vec::new(),
        chart: None,
        rows: 20,
        timeout: std::time::Duration::from_secs(30),
        runtime,
    };
    let renderer = Renderer::new(&options)?;
    let chart = file.to_string_lossy().into_owned();
    let mut warnings = Vec::new();
    // The q2 project that holds the runtime, for an interactive request. A folder that does not
    // resolve leaves the charts as images.
    let project_dir = match (request.interactive, &request.project_dir) {
        (true, Some(dir)) if !dir.as_os_str().is_empty() => match std::fs::canonicalize(dir) {
            Ok(dir) => Some(dir),
            Err(error) => {
                warnings.push(format!(
                    "the charts stay images, since the project's folder {} does not resolve: \
                     {error}",
                    dir.display()
                ));
                None
            }
        },
        _ => None,
    };
    let mut interactive = false;
    // The cells whose charts act in the page: each one's response and output, its query, and the
    // size of its image, which one graph of the document's queries draws in the page.
    let mut graphed: Vec<(usize, usize, ChartQuery, [f32; 2])> = Vec::new();
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
                let drawn = images(&target, format, &mut options, &schemes, &renderer).await;
                project = target.into_project();
                drawn
            }
            Err(failure) => Err(failure),
        };
        let outputs = match drawn {
            Ok((images, value, query)) => {
                let tagged = images.len() > 1;
                let size = images.first().map(|image| image.size);
                let mut outputs: Vec<Value> = (images.into_iter().zip(&schemes))
                    .map(|(drawn, scheme)| {
                        let mut output = display(format, drawn.bytes, drawn.size);
                        if tagged {
                            output["metadata"]["quarto_attrs"] =
                                json!({"classes": [content_class(*scheme)]});
                        }
                        output
                    })
                    .collect();
                // A chart that declares a tool or a widget acts in the page, and so does a chart of
                // a document that declares a selection, which another chart can restyle or filter.
                let chart = value.filter(|chart| {
                    project_dir.is_some()
                        && (chart.is_interactive() || !chart.selections().is_empty())
                        && browser_draws(chart.root())
                });
                let element = match (chart, size) {
                    (Some(chart), Some(size)) => match element(&chart, size)? {
                        Some(element) => Some(element),
                        None => {
                            warnings.push(format!(
                                "cell {number} stays an image, since its chart's message is \
                                 above {} MB, the most that a page holds for a canvas",
                                MAX_INTERACTIVE_BYTES / (1024 * 1024)
                            ));
                            None
                        }
                    },
                    _ => None,
                };
                if let (Some(element), Some(size)) = (element, size) {
                    // An image of a set size is one that revealjs's auto-stretch leaves in its
                    // figure, where it would otherwise lift the image out and drop the element.
                    // Elsewhere an image keeps no set size, which would keep it from shrinking
                    // to a narrow page. A PNG has its size already.
                    if request.auto_stretch && format != Format::Png {
                        for output in &mut outputs {
                            if let Some(metadata) = output["metadata"].as_object_mut() {
                                metadata.insert(
                                    "image/svg+xml".to_string(),
                                    json!({"width": size[0].round(), "height": size[1].round()}),
                                );
                            }
                        }
                    }
                    if let Some(query) = query {
                        graphed.push((responses.len(), outputs.len(), query, size));
                    }
                    outputs.push(element);
                    interactive = true;
                }
                outputs
            }
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
                vec![error_output(&reports)]
            }
        };
        response["outputs"] = Value::Array(outputs);
        responses.push(response);
    }
    // A runtime that does not write leaves the page without its loader, so the charts show their
    // images.
    let header = match (interactive, project_dir) {
        (true, Some(dir)) => match quarto2_runtime::write(&dir) {
            Ok(folder) => {
                graph(&dir, &graphed, &mut responses, &mut warnings).await;
                let entry = &project.files.manifest().style.stylesheet;
                let styles = (project.root.as_deref())
                    .and_then(|root| quarto2_runtime::styles(root, entry, &project.stylesheet));
                Some(quarto2_runtime::header(&file, &folder, &schemes, styles))
            }
            Err(error) => {
                warnings.push(format!("the charts stay images: {error}"));
                None
            }
        },
        _ => None,
    };
    Ok(Answer {
        cells: responses,
        header,
        warnings,
    })
}

/// Writes the graph of the queries of the cells `graphed` for the page, and points their elements
/// at it, so the page's charts take their selections from each other and follow them without a
/// server. A graph that does not build leaves each element with its own chart, which restyles by
/// the others but does not filter, with a warning.
async fn graph(
    dir: &std::path::Path,
    graphed: &[(usize, usize, ChartQuery, [f32; 2])],
    responses: &mut [Value],
    warnings: &mut Vec<String>,
) {
    if graphed.is_empty() {
        return;
    }
    let queries: Vec<ChartQuery> = graphed
        .iter()
        .map(|(_, _, query, _)| query.clone())
        .collect();
    let name = match ChartQuery::page_of(&queries).await {
        Ok(page) => {
            for warning in page.warnings {
                warnings.push(format!("a chart does not filter in the page: {warning}"));
            }
            quarto2_runtime::write_graph(dir, &page.bytes)
        }
        Err(error) => Err(error.to_string()),
    };
    let name = match name {
        Ok(name) => name,
        Err(error) => {
            warnings.push(format!(
                "the charts restyle by their selections but do not filter in the page: {error}"
            ));
            return;
        }
    };
    for (output, (response, at, _, size)) in graphed.iter().enumerate() {
        let html = format!(
            "<avenger-chart width=\"{}\" height=\"{}\" graph=\"{name}\" output=\"{output}\"></avenger-chart>",
            size[0].round(),
            size[1].round(),
        );
        responses[*response]["outputs"][*at] = json!({
            "output_type": "display_data",
            "data": {"text/html": html},
            "metadata": {},
        });
    }
}

/// The color schemes `names` of a request, at least one, each once.
fn color_schemes(names: &[String]) -> Result<Vec<ColorScheme>, String> {
    let mut schemes = Vec::with_capacity(names.len());
    for name in names {
        let scheme = match name.as_str() {
            "light" => ColorScheme::Light,
            "dark" => ColorScheme::Dark,
            other => return Err(format!("`colorSchemes` `{other}`: light or dark")),
        };
        if schemes.contains(&scheme) {
            return Err(format!("`colorSchemes` names `{name}` twice"));
        }
        schemes.push(scheme);
    }
    if schemes.is_empty() {
        return Err("`colorSchemes` is empty: light, dark, or both".to_string());
    }
    Ok(schemes)
}

/// The class of q2's themes that shows an image only in the color scheme `scheme`.
fn content_class(scheme: ColorScheme) -> &'static str {
    match scheme {
        ColorScheme::Light => "light-content",
        ColorScheme::Dark => "dark-content",
    }
}

/// The images of `target`, which runs once, drawn in each of `schemes`, in order, its chart, when
/// its result is one chart, and its query.
async fn images(
    target: &Target,
    format: Format,
    options: &mut Options,
    schemes: &[ColorScheme],
    renderer: &Renderer,
) -> Result<Drawn, Failure> {
    let planned = plan_target(target).await?;
    let query = match &planned {
        Planned::Query(query) => Some((**query).clone()),
        Planned::Step(_) => None,
    };
    let result = collect_target(target, planned).await?;
    let mut images = Vec::with_capacity(schemes.len());
    for scheme in schemes {
        options.color_scheme = *scheme;
        images.push(result_image(target, &result, format, options, renderer).await?);
    }
    let chart = (result.shape() == Shape::Chart)
        .then(|| result.chart(0, 0).cloned())
        .flatten();
    Ok((images, chart, query))
}

/// A cell's images, its chart when its result is one chart, and its query.
type Drawn = (Vec<Image>, Option<Chart>, Option<ChartQuery>);

/// Whether the browser's library draws the node `node` as its image draws it: it leaves out geo
/// plots, whose projections and tiles it does not build, and fetches no images, so a chart with
/// either stays an image.
fn browser_draws(node: &avenger_chart::proto::Node) -> bool {
    use avenger_chart::proto::{cartesian_layer, node::Node, plot::Coord};
    match &node.node {
        Some(Node::Plot(plot)) => match &plot.coord {
            Some(Coord::Geo(_)) => false,
            Some(Coord::Cartesian(cartesian)) => (cartesian.layers.iter())
                .all(|layer| !matches!(layer.mark, Some(cartesian_layer::Mark::Image(_)))),
            Some(Coord::Polar(_)) | None => true,
        },
        Some(Node::Concat(concat)) => concat.children.iter().all(browser_draws),
        Some(Node::Wrap(wrap)) => wrap.children.iter().all(browser_draws),
        Some(Node::Empty(_)) | None => true,
    }
}

/// The `display_data` output of the `<avenger-chart>` element of `chart`, at the natural size of
/// its image, `size`, which holds the chart's bytes as base64, or none for a chart whose message
/// is above [`MAX_INTERACTIVE_BYTES`].
fn element(chart: &Chart, size: [f32; 2]) -> Result<Option<Value>, String> {
    let bytes = chart.to_bytes().map_err(|error| error.to_string())?;
    if bytes.len() > MAX_INTERACTIVE_BYTES {
        return Ok(None);
    }
    let html = format!(
        "<avenger-chart width=\"{}\" height=\"{}\">\n<script type=\"application/vnd.avenger.chart+base64\">{}</script>\n</avenger-chart>",
        size[0].round(),
        size[1].round(),
        base64::engine::general_purpose::STANDARD.encode(bytes),
    );
    Ok(Some(json!({
        "output_type": "display_data",
        "data": {"text/html": html},
        "metadata": {},
    })))
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::{array::Float64Array, record_batch::RecordBatch};
    use avenger_chart::prelude::*;

    use super::*;

    /// A chart of symbols at `rows` points.
    fn points(rows: usize) -> Chart {
        let values = Arc::new(Float64Array::from_iter_values(
            (0..rows).map(|row| row as f64),
        ));
        let table =
            RecordBatch::try_from_iter([("x", values.clone() as _), ("y", values as _)]).unwrap();
        let symbols = CartesianLayer::symbol(table, SymbolMark::builder().x("x").y("y"));
        Chart::builder(Plot::cartesian([symbols])).build().unwrap()
    }

    #[test]
    fn a_chart_whose_message_is_too_large_for_a_page_has_no_element() {
        assert!(element(&points(10), [300.0, 200.0]).unwrap().is_some());
        let large = points(MAX_INTERACTIVE_BYTES / 8);
        assert!(large.to_bytes().unwrap().len() > MAX_INTERACTIVE_BYTES);
        assert!(element(&large, [300.0, 200.0]).unwrap().is_none());
    }
}
