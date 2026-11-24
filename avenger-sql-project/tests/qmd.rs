//! The cells of Quarto documents: the fences that are cells and those that are not, fences in
//! HTML comments, indented fences, with spaces and tabs, the offsets of a cell in its document,
//! the formatting of cells, the options of a cell's leading `#|` lines, and the script of a
//! document's cells.

use avenger_sql_project::qmd::{CellOptions, FigFormat, Qmd};

const DOCUMENT: &str = "\
---
title: Overview
---

```{avenger-sql}
#| label: fig-heavy
FROM heavy
```

1. A list

    ```{avenger-sql echo=false}
    FROM wet
    |> SELECT date
    ```

````markdown
```{avenger-sql}
FROM not_a_cell
```
````

```{.avenger-sql include=\"modules/stats.av.sqlm\"}
```

~~~{avenger-sql}
FROM tilde
~~~~
";

#[test]
fn the_cells_are_the_avenger_fences_outside_other_blocks() {
    let document = Qmd::parse(DOCUMENT);
    let texts: Vec<&str> = document
        .cells
        .iter()
        .map(|cell| cell.text.as_str())
        .collect();
    assert_eq!(
        texts,
        [
            "#| label: fig-heavy\nFROM heavy\n",
            "FROM wet\n|> SELECT date\n",
            "FROM tilde\n"
        ]
    );
    assert_eq!(document.cells[1].indent, "    ");
    assert!(document.cells.iter().all(|cell| cell.closed));
}

#[test]
fn a_cell_maps_its_offsets_to_the_document_line_by_line() {
    let document = Qmd::parse(DOCUMENT);
    let cell = &document.cells[1];
    // `SELECT` in the cell, after the indent in the document.
    let in_cell = cell.text.find("SELECT").unwrap();
    let in_document = DOCUMENT.find("SELECT").unwrap();
    assert_eq!(cell.to_document(in_cell), in_document);
    assert_eq!(cell.to_cell(in_document), Some(in_cell));
    // An offset in the indent is the start of its line.
    let line = DOCUMENT.find("    |> SELECT").unwrap();
    assert_eq!(cell.to_cell(line + 2), Some(cell.text.find("|>").unwrap()));
    // The fences and the text around them are outside the cell.
    assert_eq!(cell.to_cell(DOCUMENT.find("1. A list").unwrap()), None);
    assert_eq!(cell.to_cell(cell.range.end), None);
}

#[test]
fn an_unclosed_fence_runs_to_the_end_and_an_empty_cell_has_no_lines() {
    let document = Qmd::parse("```{avenger-sql}\n```\n\n```{avenger-sql}\nFROM t");
    assert_eq!(document.cells.len(), 2);
    let (empty, open) = (&document.cells[0], &document.cells[1]);
    assert_eq!((empty.text.as_str(), empty.closed), ("", true));
    assert_eq!(empty.to_cell(empty.range.start), None);
    assert_eq!((open.text.as_str(), open.closed), ("FROM t", false));
    // The end of the document is inside a cell that no fence closes.
    assert_eq!(open.to_cell(document.text.len()), Some(6));
}

#[test]
fn crlf_lines_keep_their_breaks() {
    let document = Qmd::parse("```{avenger-sql}\r\nFROM t\r\n```\r\n");
    assert_eq!(document.cells[0].text, "FROM t\r\n");
}

#[test]
fn format_formats_the_cells_and_leaves_the_rest() {
    let text = "Prose  with  spaces.\n\n  ```{avenger-sql}\n  from t\n\n  |>select a\n  ```\n";
    let document = Qmd::parse(text);
    let upper = |cell: &str| {
        cell.replace("from", "FROM")
            .replace("|>select", "|> SELECT")
    };
    let formatted = document.format(upper);
    assert_eq!(
        formatted,
        "Prose  with  spaces.\n\n  ```{avenger-sql}\n  FROM t\n\n  |> SELECT a\n  ```\n"
    );
    assert_eq!(Qmd::parse(formatted.as_str()).format(upper), formatted);
}

#[test]
fn the_script_of_a_document_keeps_its_lines_and_ends_each_cell() {
    let text = "# Report\n\n```{avenger-sql}\nCREATE TEMP TABLE t AS (FROM UNNEST([1]) AS a)\n```\n\nProse.\n\n```{avenger-sql}\nFROM t;\n```\n\n```{avenger-sql}\n#| eval: false\nFROM nope\n```\n";
    let document = Qmd::parse(text);
    let script = &document.script;
    // A line for each line of the document: the cells' lines, a `;` where a cell's last
    // statement has none, and empty lines elsewhere, the cell that `eval: false` keeps from
    // running too.
    assert_eq!(
        script.text,
        "\n\n\nCREATE TEMP TABLE t AS (FROM UNNEST([1]) AS a)\n;\n\n\n\n\nFROM t;\n\n\n\n\n\n\n"
    );
    assert_eq!(script.text.lines().count(), text.lines().count());
    assert_eq!(script.cells.len(), 3);
    assert!(script.cells[2].is_none());
    let starts: Vec<u32> = script
        .cell_starts()
        .iter()
        .map(|start| u32::from(*start))
        .collect();
    assert_eq!(starts, [3, script.text.find("FROM t").unwrap() as u32]);
    // Offsets map both ways in a cell's text, and the added `;` is in no cell's text.
    let from = script.text.find("FROM UNNEST").unwrap();
    assert_eq!(script.to_document(from), text.find("FROM UNNEST").unwrap());
    assert_eq!(
        script.to_script(text.find("FROM UNNEST").unwrap()),
        Some(from)
    );
    assert_eq!(script.to_script(text.find("Prose").unwrap()), None);
    let added = script.text.find("\n;").unwrap() + 1;
    assert!(!script.holds(added));
    assert!(script.holds(from));
    assert_eq!(script.cell_at(from), Some(0));
}

#[test]
fn the_script_of_an_indented_cell_drops_the_fence_s_indent() {
    let text = "1. Item\n\n    ```{avenger-sql}\n    FROM t\n    |> SELECT a\n    ```\n";
    let document = Qmd::parse(text);
    let script = &document.script;
    assert_eq!(script.text, "\n\n\nFROM t\n|> SELECT a\n;\n");
    let select = script.text.find("SELECT").unwrap();
    assert_eq!(script.to_document(select), text.find("SELECT").unwrap());
    // An offset in the indent of a line is the start of its line in the script.
    let indent = text.find("    |>").unwrap();
    assert_eq!(
        script.to_script(indent),
        Some(script.text.find("|>").unwrap())
    );
}

#[test]
fn a_cell_has_quartos_attributes_with_their_closing_brace() {
    let texts = |document: &str| -> Vec<String> {
        let document = Qmd::parse(document);
        document.cells.into_iter().map(|cell| cell.text).collect()
    };
    for fence in [
        "{avenger-sql}",
        "{avenger-sql echo=false}",
        "{avenger-sql, echo=FALSE}",
        " {avenger-sql}  ",
    ] {
        let document = format!("```{fence}\nFROM t\n```\n");
        assert_eq!(texts(&document), ["FROM t\n"], "{fence}");
    }
    for fence in [
        "{avenger-sql",
        "{avenger-sql echo=false",
        "{avenger-sqlm}",
        "{.avenger-sql}",
        "avenger-sql",
    ] {
        let document = format!("```{fence}\nFROM t\n```\n");
        assert_eq!(texts(&document), Vec::<String>::new(), "{fence}");
    }
}

#[test]
fn an_html_comment_hides_its_cells() {
    let document = Qmd::parse(
        "<!--\n```{avenger-sql}\nFROM hidden\n```\n-->\n\n<!-- one line -->\n```{avenger-sql}\nFROM shown\n```\n\n```{avenger-sql}\n<!--\nFROM in_a_cell\n```\n",
    );
    let texts: Vec<&str> = document
        .cells
        .iter()
        .map(|cell| cell.text.as_str())
        .collect();
    // A comment in a cell is the cell's text.
    assert_eq!(texts, ["FROM shown\n", "<!--\nFROM in_a_cell\n"]);
}

#[test]
fn a_tab_indents_a_fence_to_the_next_multiple_of_four_columns() {
    let text = "1. Item\n\n\t```{avenger-sql}\n\tFROM t\n    |> SELECT a\n\t```\n";
    let document = Qmd::parse(text);
    let cell = &document.cells[0];
    assert_eq!(cell.text, "FROM t\n|> SELECT a\n");
    assert_eq!(cell.indent, "\t");
    let select = text.find("SELECT").unwrap();
    assert_eq!(cell.to_document(cell.text.find("SELECT").unwrap()), select);
    // Formatting indents with the fence's tab.
    let formatted = document.format(|cell| cell.replace("|> ", "|>  "));
    assert_eq!(
        formatted,
        "1. Item\n\n\t```{avenger-sql}\n\tFROM t\n\t|>  SELECT a\n\t```\n"
    );
}

#[test]
fn a_cell_has_a_number_and_the_label_of_its_options() {
    let document = Qmd::parse(
        "```{avenger-sql}\n#| label: fig-heavy\n#| echo: false\nFROM heavy\n```\n\n```{avenger-sql}\n#| echo: false\nFROM t\n#| label: not-an-option\n```\n\n```{avenger-sql}\n#| label: \"fig-quoted\"\nFROM q\n```\n",
    );
    let labels: Vec<Option<&str>> = (document.cells.iter())
        .map(|cell| cell.options.label.as_deref())
        .collect();
    // An option line after the cell's first statement line is no option.
    assert_eq!(labels, [Some("fig-heavy"), None, Some("fig-quoted")]);
    assert_eq!(document.cell("fig-heavy").map(|(index, _)| index), Some(0));
    assert_eq!(document.cell("2").map(|(index, _)| index), Some(1));
    assert_eq!(document.cell("fig-quoted").map(|(index, _)| index), Some(2));
    assert!(document.cell("0").is_none());
    assert!(document.cell("4").is_none());
    assert!(document.cell("fig-none").is_none());
}

/// The options and the option errors of the one cell of `options`, its `#|` lines, and its
/// text.
fn cell_options(options: &str) -> (CellOptions, Vec<(String, String)>) {
    let document = Qmd::parse(format!("```{{avenger-sql}}\n{options}FROM t\n```\n"));
    let cell = &document.cells[0];
    let errors = (cell.option_errors.iter())
        .map(|error| {
            (
                cell.text[error.range.clone()].to_string(),
                error.message.clone(),
            )
        })
        .collect();
    (cell.options.clone(), errors)
}

#[test]
fn a_cell_without_options_has_none() {
    assert_eq!(cell_options(""), (CellOptions::default(), Vec::new()));
}

#[test]
fn a_cell_reads_the_options_that_avenger_uses_and_keeps_others_out() {
    let (options, errors) = cell_options(
        "#| label: fig-cars\n#| eval: false\n#| fig-width: 6\n#| fig-height: 3.5\n#| fig-format: retina\n#| fig-dpi: 192\n#| echo: false\n#| fig-cap:\n#|   - One\n#|   - Two\n",
    );
    assert_eq!(errors, Vec::new());
    assert_eq!(
        options,
        CellOptions {
            label: Some("fig-cars".to_string()),
            eval: Some(false),
            fig_width: Some(6.0),
            fig_height: Some(3.5),
            fig_format: Some(FigFormat::Retina),
            fig_dpi: Some(192.0),
        }
    );
}

#[test]
fn a_value_of_options_is_yaml() {
    // Quotes, a value over lines, an indented `#|`, and no space after `#|`.
    let (options, errors) = cell_options("  #|label: 'fig-quoted'\n#| fig-format: >-\n#|   png\n");
    assert_eq!(errors, Vec::new());
    assert_eq!(options.label.as_deref(), Some("fig-quoted"));
    assert_eq!(options.fig_format, Some(FigFormat::Png));
}

#[test]
fn a_value_of_the_wrong_type_is_an_error_at_the_value() {
    let (options, errors) = cell_options(
        "#| eval: sometimes\n#| fig-width: wide\n#| fig-height: 0\n#| fig-format: pdf\n#| label: true\n",
    );
    assert_eq!(options, CellOptions::default());
    let errors: Vec<(&str, &str)> = (errors.iter())
        .map(|(text, message)| (text.as_str(), message.as_str()))
        .collect();
    assert_eq!(
        errors,
        [
            ("sometimes", "`eval` is `true` or `false`"),
            (
                "wide",
                "`fig-width` is a number of inches, as `fig-width: 6`, more than 0"
            ),
            (
                "0",
                "`fig-height` is a number of inches, as `fig-height: 4`, more than 0"
            ),
            ("pdf", "the avenger engine draws `svg`, `png`, or `retina`"),
            ("true", "`label` is a name, as `label: fig-cars`"),
        ]
    );
}

#[test]
fn options_that_are_not_yaml_are_an_error() {
    let (options, errors) = cell_options("#| label: [unclosed\n");
    assert_eq!(options, CellOptions::default());
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].1.starts_with("the cell's options are not YAML"),
        "{errors:?}"
    );
    let (_, errors) = cell_options("#| just text\n");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].1.contains("a YAML mapping"), "{errors:?}");
}

#[test]
fn the_options_of_an_indented_cell_have_its_offsets() {
    let text = "1. Item\n\n    ```{avenger-sql}\n    #| fig-width: wide\n    FROM t\n    ```\n";
    let document = Qmd::parse(text);
    let cell = &document.cells[0];
    let error = &cell.option_errors[0];
    let start = cell.to_document(error.range.start);
    assert_eq!(&text[start..start + 4], "wide");
    let diagnostics = cell.option_diagnostics();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].source, avenger_sql_language::Source::Options);
}
