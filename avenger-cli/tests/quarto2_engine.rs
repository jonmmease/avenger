//! `avenger quarto2-engine` on copies of avenger-sql-project's fixture project: the cells of a
//! document's text as q2 passes it, their outputs, their options, the policy of errors, a document
//! outside every project, the color schemes of a document's themes, and the protocol.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

/// A copy of the fixture project in a temporary directory, by its canonical root.
fn project() -> (tempfile::TempDir, PathBuf) {
    fn walk(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let path = entry.unwrap().path();
            let target = to.join(path.file_name().unwrap());
            match path.is_dir() {
                true => walk(&path, &target),
                false => drop(std::fs::copy(&path, &target).unwrap()),
            }
        }
    }
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../avenger-sql-project/tests/projects/vega");
    let directory = tempfile::tempdir().unwrap();
    walk(&fixture, directory.path());
    let root = directory.path().canonicalize().unwrap();
    (directory, root)
}

/// The exit status and the response of `avenger quarto2-engine` to `request`.
fn engine(request: &Value) -> (i32, Value) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_avenger"))
        .arg("quarto2-engine")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(request.to_string().as_bytes()).unwrap();
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    // The response is the only thing on standard output.
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    (
        output.status.code().unwrap(),
        serde_json::from_str(&stdout).unwrap(),
    )
}

/// A request of the document at `document` with the text `text`.
fn request(document: &Path, text: &str) -> Value {
    json!({
        "protocol": 1,
        "document": document,
        "text": text,
        "image": {"format": "svg", "scale": 1.0},
        "colorSchemes": ["light"],
        "error": false,
    })
}

/// The kinds of the outputs of each cell of `response`, as its number, the line of its fence, and
/// the output types.
fn kinds(response: &Value) -> Vec<(u64, u64, Vec<String>)> {
    (response["cells"].as_array().unwrap().iter())
        .map(|cell| {
            let outputs = (cell["outputs"].as_array().unwrap().iter())
                .map(|output| output["output_type"].as_str().unwrap().to_string())
                .collect();
            (
                cell["number"].as_u64().unwrap(),
                cell["line"].as_u64().unwrap(),
                outputs,
            )
        })
        .collect()
}

/// A document as q2 passes it to the engine, with front matter: a table of cell 1 that cells 2
/// and 4 read, a cell that `eval: false` keeps from running, and a chart sized by its options.
const TEXT: &str = "---\ntitle: Report\n---\n\n```{avenger-sql}\nCREATE TEMP TABLE t AS (FROM heavy_by_origin)\n```\n\n```{avenger-sql}\nFROM t |> AGGREGATE mark_bar(x => Origin, y => cars) AS chart\n```\n\n```{avenger-sql}\n#| eval: false\nFROM nope\n```\n\n```{avenger-sql}\n#| fig-width: 3\n#| fig-height: 2\nFROM t |> AGGREGATE mark_line(x => Origin, y => cars) AS chart\n```\n";

#[test]
fn the_cells_of_a_document_draw_their_values_as_one_script() {
    let (_directory, root) = project();
    std::fs::create_dir_all(root.join("reports")).unwrap();
    // The file on disk differs from the text that q2 passes: the cells are the text's.
    std::fs::write(root.join("reports/overview.qmd"), "# Report\n").unwrap();
    let (status, response) = engine(&request(&root.join("reports/overview.qmd"), TEXT));
    assert_eq!(status, 0, "{response}");
    let svg = vec!["display_data".to_string()];
    assert_eq!(
        kinds(&response),
        [
            (1, 5, vec![]),
            (2, 9, svg.clone()),
            (3, 13, vec![]),
            (4, 18, svg),
        ]
    );
    // A cell's `fig-width` and `fig-height` size its chart, at 96 pixels to the inch.
    let sized = response["cells"][3]["outputs"][0]["data"]["image/svg+xml"]
        .as_str()
        .unwrap();
    assert!(
        sized.contains("width=\"288\"") && sized.contains("height=\"192\""),
        "{}",
        &sized[..200]
    );
}

#[test]
fn a_failing_cell_fails_the_request_unless_its_error_is_its_output() {
    let (_directory, root) = project();
    let document = root.join("report.qmd");
    std::fs::write(&document, "").unwrap();
    let text = "```{avenger-sql}\nFROM heavy_by_origin |> SELECT nope\n```\n";
    let (status, response) = engine(&request(&document, text));
    assert_eq!(status, 1);
    let error = response["error"].as_str().unwrap();
    assert!(
        error.contains("report.qmd:2:32: error: Unrecognized name: nope"),
        "{error}"
    );
    // With `error: true`, of the cell or of the request, the error is the cell's output.
    let shown = text.replace("```{avenger-sql}\n", "```{avenger-sql}\n#| error: true\n");
    let (status, response) = engine(&request(&document, &shown));
    assert_eq!(status, 0, "{response}");
    let output = &response["cells"][0]["outputs"][0];
    assert_eq!(output["output_type"], "error");
    assert_eq!(output["ename"], "avenger");
    assert_eq!(output["evalue"], "Unrecognized name: nope");
    let mut allowed = request(&document, text);
    allowed["error"] = json!(true);
    let (status, response) = engine(&allowed);
    assert_eq!(status, 0, "{response}");
    assert_eq!(response["cells"][0]["outputs"][0]["output_type"], "error");
}

#[test]
fn a_document_outside_every_project_runs_alone() {
    let directory = tempfile::tempdir().unwrap();
    let document = directory.path().join("alone.qmd");
    std::fs::write(&document, "").unwrap();
    let text = "```{avenger-sql}\nCREATE TEMP TABLE t AS (FROM UNNEST([1, 2, 3]) AS a)\n```\n\n```{avenger-sql}\nFROM t |> AGGREGATE mark_bar(x => a, y => a) AS chart\n```\n";
    let (status, response) = engine(&request(&document, text));
    assert_eq!(status, 0, "{response}");
    assert_eq!(
        kinds(&response),
        [(1, 1, vec![]), (2, 5, vec!["display_data".to_string()])]
    );
}

#[test]
fn a_result_of_rows_draws_as_a_table() {
    let (_directory, root) = project();
    let document = root.join("report.qmd");
    std::fs::write(&document, "").unwrap();
    let text = "```{avenger-sql}\nFROM heavy_by_origin |> SELECT Origin, cars\n```\n";
    let (status, response) = engine(&request(&document, text));
    assert_eq!(status, 0, "{response}");
    let svg = response["cells"][0]["outputs"][0]["data"]["image/svg+xml"]
        .as_str()
        .unwrap();
    assert!(svg.contains("Origin"), "{}", &svg[..200]);
}

/// The SVG of the output `output` of the cell `cell` of `response`.
fn svg_of(response: &Value, cell: usize, output: usize) -> &str {
    response["cells"][cell]["outputs"][output]["data"]["image/svg+xml"]
        .as_str()
        .unwrap()
}

#[test]
fn a_document_of_two_color_schemes_draws_each_chart_in_both() {
    let (_directory, root) = project();
    let document = root.join("report.qmd");
    std::fs::write(&document, "").unwrap();
    let text = "```{avenger-sql}\nFROM heavy_by_origin |> AGGREGATE mark_bar(x => Origin, y => cars) AS chart\n```\n\n```{avenger-sql}\n#| error: true\nFROM heavy_by_origin |> SELECT nope\n```\n";
    let mut both = request(&document, text);
    both["colorSchemes"] = json!(["dark", "light"]);
    let (status, response) = engine(&both);
    assert_eq!(status, 0, "{response}");
    // An image of each scheme, in the request's order, whose class shows it in its scheme.
    let outputs = response["cells"][0]["outputs"].as_array().unwrap();
    let classes: Vec<&Value> = (outputs.iter())
        .map(|output| &output["metadata"]["quarto_attrs"]["classes"])
        .collect();
    assert_eq!(
        classes,
        [&json!(["dark-content"]), &json!(["light-content"])]
    );
    // The dark image differs from the light one, and neither fills its background, the scheme's
    // `env(background)`, which the page shows through.
    let (dark, light) = (svg_of(&response, 0, 0), svg_of(&response, 0, 1));
    assert_ne!(dark, light);
    assert!(!dark.contains(r##"fill="#1e1e1e""##), "{dark}");
    assert!(!light.contains(r##"fill="#ffffff""##), "{light}");
    // An error is one output, of no scheme.
    assert_eq!(kinds(&response)[1].2, ["error"]);
    // One scheme draws one image, untagged, on the scheme's background.
    let (_, response) = engine(&request(&document, text));
    let outputs = response["cells"][0]["outputs"].as_array().unwrap();
    assert_eq!(outputs.len(), 1);
    assert!(outputs[0]["metadata"].get("quarto_attrs").is_none());
    assert!(svg_of(&response, 0, 0).contains(r##"fill="#ffffff""##));
    // A request names light, dark, or both, each once.
    for (schemes, message) in [
        (json!([]), "`colorSchemes` is empty: light, dark, or both"),
        (
            json!(["light", "light"]),
            "`colorSchemes` names `light` twice",
        ),
        (json!(["sepia"]), "`colorSchemes` `sepia`: light or dark"),
    ] {
        let mut wrong = request(&document, text);
        wrong["colorSchemes"] = schemes;
        let (status, response) = engine(&wrong);
        assert_eq!(status, 1);
        assert_eq!(response["error"], message);
    }
}

#[test]
fn a_transparent_png_keeps_the_colors_of_its_edges() {
    use base64::Engine as _;
    let (_directory, root) = project();
    let document = root.join("report.qmd");
    std::fs::write(&document, "").unwrap();
    let text = "```{avenger-sql}\n#| fig-format: png\nFROM heavy_by_origin |> AGGREGATE mark_bar(x => Origin, y => cars) AS chart\n```\n";
    let mut both = request(&document, text);
    both["colorSchemes"] = json!(["light", "dark"]);
    let (status, response) = engine(&both);
    if status != 0
        && response["error"]
            .as_str()
            .unwrap_or("")
            .contains("without a GPU")
    {
        eprintln!("no wgpu adapter, so no PNG: {response}");
        return;
    }
    assert_eq!(status, 0, "{response}");
    let dark = response["cells"][0]["outputs"][1]["data"]["image/png"]
        .as_str()
        .unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(dark)
        .unwrap();
    let image = image::load_from_memory(&bytes).unwrap().to_rgba8();
    // The faint edges of the dark scheme's light text keep its light color, with straight alpha,
    // as PNG stores it, and not its color times its alpha, which draws dark fringes.
    let faint: Vec<u8> = (image.pixels())
        .filter(|pixel| pixel[3] > 0 && pixel[3] < 64)
        .map(|pixel| pixel[0])
        .collect();
    assert!(!faint.is_empty());
    assert!(
        faint.iter().filter(|&&red| red > 150).count() * 2 > faint.len(),
        "{faint:?}"
    );
}

#[test]
fn a_request_of_another_protocol_names_the_side_to_update() {
    let directory = tempfile::tempdir().unwrap();
    let document = directory.path().join("alone.qmd");
    std::fs::write(&document, "").unwrap();
    let mut newer = request(&document, "");
    newer["protocol"] = json!(2);
    let (status, response) = engine(&newer);
    assert_eq!(status, 1);
    assert_eq!(
        response,
        json!({
            "protocol": 1,
            "error": "this project's extension speaks protocol 2, newer than this avenger's 1: upgrade avenger",
        })
    );
    let mut older = request(&document, "");
    older["protocol"] = json!(0);
    let (_, response) = engine(&older);
    assert!(
        response["error"]
            .as_str()
            .unwrap()
            .ends_with("run `avenger quarto2-init` to update it"),
        "{response}"
    );
    // A missing document fails the request.
    let (status, response) = engine(&request(&directory.path().join("missing.qmd"), ""));
    assert_eq!(status, 1);
    assert!(
        response["error"].as_str().unwrap().contains("missing.qmd"),
        "{response}"
    );
}

#[test]
fn a_png_has_its_size_in_css_pixels() {
    let directory = tempfile::tempdir().unwrap();
    let document = directory.path().join("alone.qmd");
    std::fs::write(&document, "").unwrap();
    let text = "```{avenger-sql}\n#| fig-format: retina\n#| fig-width: 3\n#| fig-height: 2\n#| error: true\nFROM UNNEST([1, 2]) AS a |> AGGREGATE mark_bar(x => a, y => a) AS chart\n```\n";
    let (status, response) = engine(&request(&document, text));
    assert_eq!(status, 0, "{response}");
    let output = &response["cells"][0]["outputs"][0];
    // PNG needs a wgpu adapter, which some machines lack.
    if output["output_type"] == "error" {
        let message = output["evalue"].as_str().unwrap();
        assert!(
            message.contains("GPU") || message.contains("adapter"),
            "{message}"
        );
        return;
    }
    assert!(output["data"]["image/png"].is_string());
    assert_eq!(
        output["metadata"]["image/png"],
        json!({"width": 288.0, "height": 192.0})
    );
}
