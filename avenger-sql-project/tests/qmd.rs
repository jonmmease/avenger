//! The cells of Quarto documents: the fences that are cells and those that are not, fences in
//! HTML comments, indented fences, with spaces and tabs, the offsets of a cell in its document,
//! and the formatting of cells.

use avenger_sql_project::qmd::{cell_path, split_cell_path, Qmd};

const DOCUMENT: &str = "\
---
title: Overview
---

```{avenger-sql}
--| label: fig-heavy
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
            "--| label: fig-heavy\nFROM heavy\n",
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
fn a_cell_path_names_its_document_and_number() {
    assert_eq!(
        cell_path("reports/overview.qmd", 3),
        "reports/overview.qmd#3"
    );
    assert_eq!(
        split_cell_path("reports/overview.qmd#3"),
        Some(("reports/overview.qmd", 2))
    );
    assert_eq!(split_cell_path("charts/a.av.sql"), None);
    assert_eq!(split_cell_path("reports/overview.qmd#0"), None);
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
