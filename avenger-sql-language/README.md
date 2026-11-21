# avenger-sql-language

Editor features of avenger-sql scripts: diagnostics, semantic highlighting, the outline, folding and selection ranges, go to definition, references, rename, hover, completion, signature help, and formatting.

A `Document` parses the text of a file with avenger-sql-syntax and analyzes it with avenger-sql once, in the mode of its `Role`, and each request reads that parse and analysis. A chart script analyzes with `analyze`, and a data script with `analyze_data`, and the catalog of both gives the tables of their files. The requests on a data script's statements of files, the completion of their options, formats, and URIs, the hover of an option or of a table that a statement defines, and the outline, apply to a chart script's own tables of files, `CREATE TEMP EXTERNAL TABLE` and `LOAD DATA ... TEMP TABLE`, too. A module has the requests of its syntax, since avenger-sql analyzes a module where a script imports it. The tables of a script come from an avenger-sql `Catalog`, and its functions from a `Library`.

```rust
use std::sync::Arc;

use avenger_sql::{analysis::Options, catalog::MemoryCatalog, library::Library};
use avenger_sql_language::{Document, Role};
use datafusion::arrow::datatypes::{DataType, Field, Schema};

let schema = Schema::new(vec![Field::new("species", DataType::Utf8, true)]);
let catalog = MemoryCatalog::new().with_schema(&["penguins"], Arc::new(schema));
let text = "FROM penguins |> WHERE ";
let document = Document::new(
    text,
    Role::Script,
    Arc::new(catalog),
    Arc::new(Library::new()),
    Options::default(),
);
for item in document.completions((text.len() as u32).into()).items {
    println!("{} {:?} {:?}", item.label, item.kind, item.detail);
}
```

## The requests

| Request | Result |
|---|---|
| `diagnostics()` | The errors of the syntax and of the analysis, with their help and the fixes of the syntax's errors. An error of the analysis inside a statement with an error of the syntax is left out, since it repeats the syntax's. |
| `highlights()` | The classes of the tokens, which the analysis refines: a variable, a range variable, a column, a function by what it computes, and a value of a set. In the options of a data script's files, the value of `format` is a `Format` and a URI of `uris` a `Uri`, so that a theme can show a format as a constant and a URI as a path. |
| `symbols()` | The outline: variables, temporary tables and functions, imports, and queries, and under a query, its pipe operators with their output columns. In a data script, the tables, views, and schemas that its statements define, each with its statement as its detail, as `TEMP EXTERNAL TABLE` or `LOAD DATA INTO`. |
| `folding_ranges()` | The statements, parenthesized queries, and argument lists that span lines, and the comments that span lines. |
| `selection_ranges(offset)` | The ranges of the token at the offset and of the nodes around it, innermost first. |
| `definition(offset)` | The name at the offset and the range that defines it. |
| `references(offset)` | The uses of the definition of the name at the offset, and the name that defines it. |
| `prepare_rename(offset)`, `rename(offset, name)` | The name that a rename changes, or the reason that it can't, and the edits of the rename. A rename that would add an error, or change what a name refers to, fails. |
| `hover(offset)` | Markdown on a name, a function, an argument's name, or a value of a set: a column's type and definition, a table's columns, a function's forms and summary, and an argument's parameter. In a data script, an option's type, meaning, and formats, or the reason that avenger-sql rejects an option of BigQuery, a table's format, URIs, description, and columns, and a schema's description, as a schema that BigQuery calls a dataset. A table of the catalog shows its description and the file that defines it, when the catalog gives them. A parameter that takes constructors shows what it takes in place of its type, as `pin => pin(value: ANY, position => 'low' \| 'mid' \| 'high')`, with three of its strings at most, then the constructors one level down. A constructor's hover names the place that it fills. |
| `completions(offset)` | The items, and the text that an item replaces when an editor inserts it or replaces the whole name, as the Language Server Protocol's `InsertReplaceEdit` does: the columns of the operator's input, range variables, variables, the functions that the place takes, tables, the parameters and the values of a call's arguments, and keywords. In a quoted name, an item inserts the name, with the closing backtick that the name lacks. In a data script, its statements, the tables and views of the statements before the cursor, in `OPTIONS` and `FROM FILES` the options that the statement's format takes and that it does not set, the values of `format` and `compression` that the build reads, and in a URI, the paths of the project's files. The items of each kind come together, a function's kind by its role, and the kinds come in the order of their most relevant items. |
| `signature_help(offset)` | The forms of the call at the offset, each with the ranges of its parameters, the form that the call takes, and the parameter at the offset. |
| `formatting()` | The edit that formats the text in the style of `avenger-sql/STYLE.md`, by its role, or none when it is formatted. A statement with an error stays as written. |

A `Document` is `Send` and `Sync`. Every edit of the text makes a new `Document`. `analysis()` gives the analysis of a chart script to a host that runs it, and `data_analysis()` the tables, views, and schemas that a data script defines.

A document does no I/O. A host gives it the paths of the project's files with `with_paths`, relative to the project's root and separated by `/`, as `data/files/sales.csv`. Completion in a URI offers the paths that extend the text before the cursor by one segment: the files of the statement's format, as `.csv` and `.csv.gz` for CSV, and the directories that hold them, each ending with `/`.

`quote(name)` writes a name as a script writes it, as completion inserts it: as it is when it is a plain name, and in backticks otherwise, or when it is a reserved word, such as `` `unit price` `` or `` `order` ``. Names are case-insensitive, so `PetalLengthCm` stays as it is.

## Completion

Completion offers what the grammar takes at the cursor, as avenger-sql-syntax's `expected_at` gives it, filled with the names that the analysis and the catalog know there. The analysis's place at the cursor decides the names and functions: a `WHERE` takes the columns of its input and scalar and window functions, an item of `|> AGGREGATE` takes aggregates and marks but no column by itself, `LIMIT` takes no column, and the functions of `MATCH_RECOGNIZE` stand only in its clauses, by the same rules that the analysis reports as errors. After a named argument, an argument takes only the name of a parameter. A subquery takes the columns and the range variables of the queries that it can read, after its own. Columns of the type that the place expects, such as `BOOL` in a `WHERE`, come first. At the start of an argument, the constructors that its parameter takes come first, under the names that the parameter gives them: the constructor named after the argument, then those one level down, as `scale`, `type`, and `pin` after `scale => `, then the values of the parameter's set, before the other items, and in a string, every value of the set.

`tests/completion_eval.rs` evaluates completion at several locations of each of its example queries, in each whole query and in its text cut at the cursor, against an oracle that parses and analyzes each item after the text before the cursor. It fails when completion does not offer the token of the query where the text before the cursor tells it, or offers an item that the oracle rejects. An ignored test, which `--ignored` runs, also tries each keyword, function, and name in the cut text, and fails when completion leaves out an item that the oracle accepts. The evaluation reads chart scripts: its oracle analyzes each text with a catalog of tables and no files, in which a statement of a data script that reads files has an error, so `tests/data_script.rs` tests the completion of data scripts.

## Positions

Positions and ranges are byte offsets of the text, as in avenger-sql-syntax. A host converts them to its own units: the Language Server Protocol takes UTF-16 positions, or UTF-8 positions with a client that offers them, and a text widget can take characters. A request takes an offset at a character boundary, and an offset past the end reads as the end.

## Known gaps

- The crate has no server of the Language Server Protocol.
- A module's document has no analysis, so it has the requests of its syntax alone.
- The text of a module isn't in the document: a module's functions have no definition, and completion offers no module paths and no functions of a module.
- The analysis records no use of a temporary function's parameters in its body, of a named window, or of a field after a pattern variable, as `s` in `p.s`. These names have no definition, references, rename, or hover. Completion offers the parameters of a function in its body.
- Every edit analyzes the whole script again.
