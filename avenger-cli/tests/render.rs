//! `avenger render` on copies of avenger-sql-project's fixture project: a chart to SVG, PDF, and
//! PNG at its natural size, `--size`, `--scale`, and `--color-scheme`, a table of charts and
//! `--chart`, a table of text and an empty one, a cell of a Quarto document by number and by
//! label, `--all`, and the errors that stop a render.
//! PNG needs a wgpu adapter, and its tests pass without one after saying so.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A copy of the fixture project in a temporary directory, by its canonical root, with a chart
/// of a table of charts and a document of one labeled cell.
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
    std::fs::write(
        root.join("charts/by-origin.av.sql"),
        "FROM vega_datasets.cars\n|> AGGREGATE COUNT(*) AS cars, mark_symbol(x => Horsepower, y => Miles_per_Gallon) AS chart GROUP BY Origin\n|> ORDER BY Origin\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("reports")).unwrap();
    std::fs::write(
        root.join("reports/overview.qmd"),
        "# Report\n\n```{avenger-sql}\n#| label: fig-weather\nFROM wet_days |> AGGREGATE mark_bar(x => weather, y => COUNT(*)) AS chart\n```\n",
    )
    .unwrap();
    (directory, root)
}

/// The exit status, standard output, and standard error of `avenger` with `args` in `directory`.
fn avenger(directory: &Path, args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_avenger"))
        .args(args)
        .current_dir(directory)
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    (
        output.status.code().unwrap(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

/// The width and the height of an SVG, from its root's attributes.
fn svg_size(svg: &str) -> [f32; 2] {
    let root = &svg[..svg.find('>').unwrap()];
    let attribute = |name: &str| -> f32 {
        let start = root.find(&format!(" {name}=\"")).unwrap() + name.len() + 3;
        root[start..start + root[start..].find('"').unwrap()]
            .parse()
            .unwrap()
    };
    [attribute("width"), attribute("height")]
}

#[test]
fn a_chart_renders_to_svg_and_pdf_at_its_size() {
    let (_directory, root) = project();
    let (status, stdout, stderr) = avenger(
        &root,
        &["render", "charts/origins.av.sql", "-o", "origins.svg"],
    );
    assert_eq!((status, stdout.as_str()), (0, "origins.svg\n"), "{stderr}");
    let svg = std::fs::read_to_string(root.join("origins.svg")).unwrap();
    assert!(svg.starts_with("<svg"));
    let natural = svg_size(&svg);
    assert!(natural[0] > 0.0 && natural[1] > 0.0, "{natural:?}");
    // The same chart renders the same.
    avenger(
        &root,
        &["render", "charts/origins.av.sql", "-o", "again.svg"],
    );
    assert_eq!(
        std::fs::read_to_string(root.join("again.svg")).unwrap(),
        svg
    );

    // `--size` sets the size, or one side of it.
    avenger(
        &root,
        &[
            "render",
            "charts/origins.av.sql",
            "-o",
            "sized.svg",
            "--size",
            "400x300",
        ],
    );
    let sized = std::fs::read_to_string(root.join("sized.svg")).unwrap();
    assert_eq!(svg_size(&sized), [400.0, 300.0]);
    avenger(
        &root,
        &[
            "render",
            "charts/origins.av.sql",
            "-o",
            "wide.svg",
            "--size",
            "500x",
        ],
    );
    let wide = std::fs::read_to_string(root.join("wide.svg")).unwrap();
    assert_eq!(svg_size(&wide)[0], 500.0);

    let (status, _, stderr) = avenger(
        &root,
        &["render", "charts/origins.av.sql", "-o", "origins.pdf"],
    );
    assert_eq!(status, 0, "{stderr}");
    assert!(std::fs::read(root.join("origins.pdf"))
        .unwrap()
        .starts_with(b"%PDF"));
}

#[test]
fn a_png_takes_the_scale_and_the_background_of_the_color_scheme() {
    let (_directory, root) = project();
    let args = [
        "render",
        "charts/origins.av.sql",
        "-o",
        "dark.png",
        "--scale",
        "2",
        "--color-scheme",
        "dark",
    ];
    let (status, _, stderr) = avenger(&root, &args);
    if status != 0 && stderr.contains("without a GPU") {
        eprintln!("no wgpu adapter, so no PNG: {stderr}");
        return;
    }
    assert_eq!(status, 0, "{stderr}");
    avenger(
        &root,
        &["render", "charts/origins.av.sql", "-o", "natural.svg"],
    );
    let natural = svg_size(&std::fs::read_to_string(root.join("natural.svg")).unwrap());
    let image = image::open(root.join("dark.png")).unwrap().to_rgba8();
    // The canvas's pixels are the size times the scale, to a pixel.
    let near = |pixels: u32, size: f32| (pixels as f32 - size * 2.0).abs() <= 1.0;
    assert!(
        near(image.width(), natural[0]),
        "{} {natural:?}",
        image.width()
    );
    assert!(
        near(image.height(), natural[1]),
        "{} {natural:?}",
        image.height()
    );
    // The corner is the background of the dark color scheme.
    assert_eq!(image.get_pixel(0, image.height() - 1).0, [30, 30, 30, 255]);
}

#[test]
fn a_result_of_several_charts_renders_as_a_table_and_chart_picks_one() {
    let (_directory, root) = project();
    let (status, _, stderr) = avenger(
        &root,
        &["render", "charts/by-origin.av.sql", "-o", "table.svg"],
    );
    assert_eq!(status, 0, "{stderr}");
    let table = svg_size(&std::fs::read_to_string(root.join("table.svg")).unwrap());
    let (status, _, stderr) = avenger(
        &root,
        &[
            "render",
            "charts/by-origin.av.sql",
            "-o",
            "one.svg",
            "--chart",
            "2",
        ],
    );
    assert_eq!(status, 0, "{stderr}");
    let one = svg_size(&std::fs::read_to_string(root.join("one.svg")).unwrap());
    // The table holds the three charts, one above another, with their text columns beside them.
    assert!(
        table[0] > one[0] && table[1] > 3.0 * one[1],
        "{table:?} {one:?}"
    );

    // `--size` is the size of each chart of the table, and `--rows` caps the rows.
    let (status, _, stderr) = avenger(
        &root,
        &[
            "render",
            "charts/by-origin.av.sql",
            "-o",
            "cells.svg",
            "--size",
            "200x100",
            "--rows",
            "2",
        ],
    );
    assert_eq!(status, 0);
    assert!(stderr.contains("shows 2 of 3 rows"), "{stderr}");
    let cells = svg_size(&std::fs::read_to_string(root.join("cells.svg")).unwrap());
    assert!(
        cells[1] > 2.0 * 100.0 && cells[1] < 3.0 * 100.0 + 120.0,
        "{cells:?}"
    );

    // A row past the table, and a column that is no column of charts.
    let (status, stdout, _) = avenger(
        &root,
        &[
            "render",
            "charts/by-origin.av.sql",
            "-o",
            "x.svg",
            "--chart",
            "9",
        ],
    );
    assert_eq!(status, 1);
    assert!(stdout.contains("the rows are 1 to 3"), "{stdout}");
    let (status, stdout, _) = avenger(
        &root,
        &[
            "render",
            "charts/by-origin.av.sql",
            "-o",
            "x.svg",
            "--chart",
            "1.cars",
        ],
    );
    assert_eq!(status, 1);
    assert!(stdout.contains("no column of charts `cars`"), "{stdout}");
}

#[test]
fn a_result_without_charts_renders_as_a_table_of_text_lines() {
    let (_directory, root) = project();
    std::fs::write(
        root.join("charts/rows.av.sql"),
        "FROM vega_datasets.cars\n|> AGGREGATE COUNT(*) AS cars GROUP BY Origin\n|> ORDER BY Origin\n",
    )
    .unwrap();
    std::fs::write(
        root.join("charts/none.av.sql"),
        "FROM vega_datasets.cars |> WHERE FALSE |> SELECT Name, Origin\n",
    )
    .unwrap();
    let (status, _, stderr) = avenger(&root, &["render", "charts/rows.av.sql", "-o", "rows.svg"]);
    assert_eq!(status, 0, "{stderr}");
    let rows = svg_size(&std::fs::read_to_string(root.join("rows.svg")).unwrap());
    // A header and three rows, each one line of text tall with its padding.
    let line = rows[1] / 4.0;
    assert!(line > 12.0 && line < 30.0, "{rows:?}");
    // An empty result shows its header and that it has no rows.
    let (status, _, stderr) = avenger(&root, &["render", "charts/none.av.sql", "-o", "none.svg"]);
    assert_eq!(status, 0, "{stderr}");
    let none = svg_size(&std::fs::read_to_string(root.join("none.svg")).unwrap());
    assert!((none[1] - 2.0 * line).abs() < 1.0, "{none:?} {line}");
    // `--chart` picks no chart from a table without one.
    let (status, stdout, _) = avenger(
        &root,
        &[
            "render",
            "charts/rows.av.sql",
            "-o",
            "x.svg",
            "--chart",
            "1",
        ],
    );
    assert_eq!(status, 1);
    assert!(
        stdout.contains("the result has no column of charts"),
        "{stdout}"
    );
}

#[test]
fn a_cell_renders_by_its_number_or_its_label_and_all_renders_each_chart() {
    let (_directory, root) = project();
    for cell in [
        "reports/overview.qmd#1",
        "reports/overview.qmd#fig-weather",
        "reports/overview.qmd",
    ] {
        let (status, _, stderr) = avenger(&root, &["render", cell, "-o", "cell.svg"]);
        assert_eq!(status, 0, "{cell}: {stderr}");
    }
    let (status, _, stderr) = avenger(
        &root,
        &["render", "reports/overview.qmd#fig-none", "-o", "cell.svg"],
    );
    assert_eq!(status, 2);
    assert!(
        stderr.contains("no `{avenger-sql}` cell `fig-none`"),
        "{stderr}"
    );

    let (status, stdout, stderr) =
        avenger(&root, &["render", "--all", "-o", "all", "--format", "svg"]);
    assert_eq!(status, 0, "{stderr}");
    let mut written: Vec<&str> = stdout.lines().collect();
    written.sort();
    assert_eq!(
        written,
        [
            "all/charts-by-origin.av.sql.svg",
            "all/charts-origins.av.sql.svg",
            "all/charts-weather-warm-days.av.sql.svg",
            "all/reports-linked.qmd-1.svg",
            "all/reports-linked.qmd-2.svg",
            "all/reports-linked.qmd-3.svg",
            "all/reports-linked.qmd-4.svg",
            "all/reports-linked.qmd-5.svg",
            "all/reports-overview.qmd-1.svg",
        ]
    );
}

#[test]
fn errors_stop_a_render() {
    let (_directory, root) = project();
    // The script's errors, as `check` reports them.
    std::fs::write(
        root.join("charts/bad.av.sql"),
        "FROM vega_datasets.cars\n|> SELECT nope\n",
    )
    .unwrap();
    let (status, stdout, _) = avenger(&root, &["render", "charts/bad.av.sql", "-o", "bad.svg"]);
    assert_eq!(status, 1);
    assert!(
        stdout.starts_with("charts/bad.av.sql:2:11: error: Unrecognized name: nope"),
        "{stdout}"
    );
    // An image of no known format, a mode that the chart lacks, and a size of no number.
    let usage = [
        vec!["render", "charts/origins.av.sql", "-o", "origins.gif"],
        vec![
            "render",
            "charts/origins.av.sql",
            "-o",
            "o.svg",
            "--mode",
            "zoom",
        ],
        vec![
            "render",
            "charts/origins.av.sql",
            "-o",
            "o.svg",
            "--size",
            "big",
        ],
    ];
    for args in usage {
        let (status, _, stderr) = avenger(&root, &args);
        assert_eq!(status, 2, "{args:?}: {stderr}");
    }
}

#[test]
fn a_statement_at_a_line_renders_its_rows_as_a_table() {
    let (_directory, root) = project();
    std::fs::write(
        root.join("charts/steps.av.sql"),
        "CREATE TEMP TABLE origins AS (\n  FROM vega_datasets.cars\n  |> AGGREGATE COUNT(*) AS cars GROUP BY Origin\n);\nFROM origins |> AGGREGATE mark_bar(x => Origin, y => cars) AS chart\n",
    )
    .unwrap();
    let (status, _, stderr) = avenger(
        &root,
        &["render", "charts/steps.av.sql:2", "-o", "origins.svg"],
    );
    assert_eq!(status, 0, "{stderr}");
    let svg = std::fs::read_to_string(root.join("origins.svg")).unwrap();
    // The table's rows: the counts of cars print as text, where the script's chart has bars.
    assert!(svg.contains(">254<") && svg.contains(">79<"), "{svg}");
    let (status, _, _) = avenger(&root, &["render", "charts/steps.av.sql", "-o", "chart.svg"]);
    assert_eq!(status, 0);
    let chart = std::fs::read_to_string(root.join("chart.svg")).unwrap();
    assert!(!chart.contains(">254<"));
}
