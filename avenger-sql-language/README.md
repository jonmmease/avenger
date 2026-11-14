# avenger-sql-language

Editor features of avenger-sql scripts: diagnostics, semantic highlighting, the outline, folding and selection ranges, go to definition, references, rename, hover, completion, and signature help.

A `Document` parses a script's text with avenger-sql-syntax and analyzes it with avenger-sql once, and each request reads that parse and analysis. The tables of a script come from an avenger-sql `Catalog`, and its functions from a `Library`.

```rust
use std::sync::Arc;

use avenger_sql::{analysis::Options, catalog::MemoryCatalog, library::Library};
use avenger_sql_language::Document;
use datafusion::arrow::datatypes::{DataType, Field, Schema};

let schema = Schema::new(vec![Field::new("species", DataType::Utf8, true)]);
let catalog = MemoryCatalog::new().with_schema(&["penguins"], Arc::new(schema));
let text = "FROM penguins |> WHERE ";
let document = Document::new(text, Arc::new(catalog), Arc::new(Library::new()), Options::default());
for item in document.completions((text.len() as u32).into()).items {
    println!("{} {:?} {:?}", item.label, item.kind, item.detail);
}
```

## The requests

| Request | Result |
|---|---|
| `diagnostics()` | The errors of the syntax and of the analysis, with their help and the fixes of the syntax's errors. An error of the analysis inside a statement with an error of the syntax is left out, since it repeats the syntax's. |
| `highlights()` | The classes of the tokens, which the analysis refines: a variable, a range variable, a column, a function by what it computes, and a value of a set. |
| `symbols()` | The outline: variables, temporary tables and functions, imports, and queries, and under a query, its pipe operators with their output columns. |
| `folding_ranges()` | The statements, parenthesized queries, and argument lists that span lines, and the comments that span lines. |
| `selection_ranges(offset)` | The ranges of the token at the offset and of the nodes around it, innermost first. |
| `definition(offset)` | The name at the offset and the range that defines it. |
| `references(offset)` | The uses of the definition of the name at the offset, and the name that defines it. |
| `prepare_rename(offset)`, `rename(offset, name)` | The name that a rename changes, or the reason that it can't, and the edits of the rename. A rename that would add an error, or change what a name refers to, fails. |
| `hover(offset)` | Markdown on a name, a function, or a value of a set: a column's type and definition, a table's columns, a function's forms and reference. |
| `completions(offset)` | The items, and the text that an item replaces when an editor inserts it or replaces the whole name, as the Language Server Protocol's `InsertReplaceEdit` does: the columns of the operator's input, range variables, variables, the functions that the place takes, tables, the parameters and the values of a call's arguments, and keywords. In a quoted name, an item inserts the name, with the closing backtick that the name lacks. |
| `signature_help(offset)` | The forms of the call at the offset, each with the ranges of its parameters, the form that the call takes, and the parameter at the offset. |

A `Document` is `Send` and `Sync`. Every edit of the text makes a new `Document`, and `analysis()` gives its analysis to a host that runs the script.

## Completion

Completion offers what the grammar takes at the cursor, as avenger-sql-syntax's `expected_at` gives it, filled with the names that the analysis and the catalog know there. The analysis's place at the cursor decides the names and functions: a `WHERE` takes the columns of its input and scalar and window functions, an item of `|> AGGREGATE` takes aggregates and marks but no column by itself, `LIMIT` takes no column, and the functions of `MATCH_RECOGNIZE` stand only in its clauses, by the same rules that the analysis reports as errors. After a named argument, an argument takes only the name of a parameter. A subquery takes the columns and the range variables of the queries that it can read, after its own. Columns of the type that the place expects, such as `BOOL` in a `WHERE`, come first.

`tests/completion_eval.rs` evaluates completion at about 780 locations of 100 example queries, in each whole query and in its text cut at the cursor, against an oracle that parses and analyzes each item after the text before the cursor. It fails when completion does not offer the token of the query where the text before the cursor tells it, offers an item that the oracle rejects, or leaves out an item that the oracle accepts.

## Positions

Positions and ranges are byte offsets of the text, as in avenger-sql-syntax. A host converts them to its own units: the Language Server Protocol takes UTF-16 positions, or UTF-8 positions with a client that offers them, and a text widget can take characters. A request takes an offset at a character boundary, and an offset past the end reads as the end.

## Known gaps

- The crate has no server of the Language Server Protocol.
- The text of a module isn't in the document: a module's functions have no definition, and completion offers no module paths and no functions of a module.
- The analysis records no use of a temporary function's parameters in its body, of a named window, or of a field after a pattern variable, as `s` in `p.s`. These names have no definition, references, rename, or hover. Completion offers the parameters of a function in its body.
- Every edit analyzes the whole script again.
