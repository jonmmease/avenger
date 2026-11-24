# avenger-sql-syntax

The syntax of avenger-sql: a subset of GoogleSQL's grammar, read into a lossless syntax tree with typed nodes. The crate reads scripts of pipe queries, `IMPORT MODULE`, `DECLARE`, `SET`, `CREATE TEMP TABLE`, `CREATE FUNCTION`, and the `TEMP` forms of `CREATE EXTERNAL TABLE` and `LOAD DATA`, `.sqlm` modules of public functions, table functions and constants, and data scripts, which register the tables of a project. Within the subset, it accepts what GoogleSQL accepts, rejects what GoogleSQL rejects, and reports its first error where GoogleSQL does, with one extension: every comma-separated list takes a trailing comma, where GoogleSQL takes one only in the lists of the pipe operators and the columns of `CREATE TABLE`. A value in parentheses, `(x,)`, takes none, since GoogleSQL has no tuple of one value, and neither do the names of a `DECLARE`, which a type can follow. Outside the subset, it reports each construct of GoogleSQL's that avenger doesn't plan, such as a `CREATE MATERIALIZED VIEW`, a hint or a standard `SELECT` query, as unsupported, with its name, instead of a syntax error, and skips it to a boundary, such as the end of its clause or statement.

```rust
use avenger_sql_syntax::{ast, parse};

let parse = parse("FROM orders |> WHERE total > 10 |> AGGREGATE SUM(total) GROUP BY region");
assert!(parse.ok());
let script = parse.script().unwrap();
let Some(ast::Stmt::Query(query)) = script.stmts().next() else { unreachable!() };
for operator in query.pipe_operators() {
    println!("{operator}");
}
```

## Data scripts

`parse_data` reads a data script, a `.av.sql` file of a project whose statements register the tables that the project's scripts read, with a subset of GoogleSQL's DDL:

```sql
CREATE SCHEMA finance OPTIONS (description = 'the books');

CREATE EXTERNAL TABLE finance.orders
OPTIONS (format = 'CSV', uris = ['data/files/orders/*.csv']);

LOAD DATA OVERWRITE finance.rates (day DATE, rate FLOAT64)
FROM FILES (format = 'CSV', uris = ['data/files/rates.csv'], skip_leading_rows = 1);

CREATE TEMP EXTERNAL TABLE raw_events
OPTIONS (format = 'PARQUET', uris = ['data/files/events/*.parquet']);

CREATE VIEW recent AS (FROM finance.orders |> WHERE day > DATE '2025-01-01');

CREATE TABLE events AS (FROM raw_events |> WHERE kind IS NOT NULL);
```

`CREATE EXTERNAL TABLE` declares a table over files, with an optional column list, `WITH PARTITION COLUMNS` and the required `OPTIONS`. `LOAD DATA {OVERWRITE | INTO} [TEMP TABLE]` loads files into a table, with an optional column list and `OPTIONS` before `FROM FILES (...)`, whose entries take the form of `OPTIONS`, and `WITH PARTITION COLUMNS` after it. `CREATE VIEW` names a query, `CREATE TABLE ... AS` keeps the result of one, and `CREATE SCHEMA` creates a namespace. `TEMP` keeps a table or a view inside its data script. A chart script holds `CREATE TEMP EXTERNAL TABLE` and `LOAD DATA {OVERWRITE | INTO} TEMP TABLE` too, whose tables are its own. A data script also holds `CREATE TEMP FUNCTION` and `IMPORT MODULE`, for its own statements. The clause order follows GoogleSQL's grammar, and `WITH CONNECTION`, the partitions and clusters of `LOAD DATA`, and constraints are unsupported.

The parser reads the statements of every role in `parse`, `parse_module` and `parse_data`, and the validation pass checks each statement against the role of its text. A query, `DECLARE`, `SET`, a `CREATE TABLE` without `AS`, or a function without `TEMP` in a data script is an error whose help names the chart scripts, and a statement of a data script or a `CREATE TABLE` without `TEMP` in a script or a module is an error whose help names the data scripts. A chart script takes `CREATE TEMP EXTERNAL TABLE` and `LOAD DATA ... TEMP TABLE`, and the help of the forms without `TEMP` there names them. GoogleSQL takes the statements of data scripts in a script, so in a script they are unsupported. A data script's root is a `SCRIPT` node, as a script's is. `Role` names the three roles of a text, `Script`, `Module` and `Data`, for the functions that read a text in its role: completion and formatting.

## The tree

`parse`, `parse_module` and `parse_data` never fail. Each returns a `rowan` tree that holds every byte of the text, comments and whitespace included, with a range on every node. Every node is a `SyntaxNode` of a `SyntaxKind`. A comment before a statement or a pipe operator belongs to that node, and no node starts or ends with trivia.

`sql.ungram` describes the children of each kind of node, with notes on the GoogleSQL sources, the subset, and the forms left out. The grammar describes the tree, and the hand-written parser builds it. A test checks that every tree of the test corpora has the shape that the grammar describes.

## Typed nodes

`build.rs` reads `sql.ungram` and generates the node kinds of `SyntaxKind` and the typed nodes of `ast`. A typed node, such as `ast::PipeWhere`, wraps a node of its kind and has an accessor for each child, such as `condition()`. A rule that chooses between nodes, such as `Expr`, is an enum. Since a tree of a text with errors can lack any part, an accessor returns an `Option` or an iterator. `src/ast/manual.rs` adds the accessors that decode text: the values of literals, the text of names, the operators of expressions, and the modifiers of `CREATE` statements.

## Diagnostics

Each error is a `Diagnostic`, with a `DiagnosticCode`, a range, a message, help, and fixes, which are the edits that repair the text. Three stages report errors:

- The lexer reports malformed tokens, such as an unclosed string.
- The parser reports the errors of the grammar. It puts what it cannot read into `ERROR` nodes and goes on at the next `,`, `)`, `|>` or `;`.
- The validation pass, `src/validation.rs`, reports the checks that GoogleSQL makes after its parser and the statements that each role of a text holds: the rules of modules and of data scripts, the order of `DECLARE`, the clauses that each kind of function takes, the joins after comma joins, and the escapes of literals.

## Completion

`expected_at(text, offset, role)` returns what the grammar accepts at a cursor of a text of `role`: keywords, tokens, names in their roles (a column, a table, a function, and so on), expressions, types, pipe operators, and statements, and words outside the lexer's keywords: a set of words, such as the date parts, the sampling methods, and the words that start an expression or a pipe operator, and phrases, such as `BY NAME`, which the grammar takes only whole. When the text ends inside a phrase, the result holds the rest of it, as `NAME` after `UNION ALL BY`. When a word ends at the cursor, the result is what can take that word's place.

The result holds only what can follow the text before the cursor. After an operand, it holds the operators that can follow it. After a comma of a list that can end with one, it holds what an item takes. After a token that the text lacks, such as the `)` of an open call, it holds nothing more, since what follows the token cannot follow the cursor. A statement starts at the start of the text or after a `;`. The forms of GoogleSQL that avenger-sql reports as errors, such as `SELECT` at the start of a query or a `CONSTANT` in a script, are not in the result. The statements of a data script are in the result only in a data script: `LOAD DATA` at the start of a statement, and `EXTERNAL TABLE`, `VIEW` and `SCHEMA` after `CREATE`. A chart script's result holds its own tables of files, `LOAD DATA` at the start of a statement and `EXTERNAL TABLE` after `CREATE TEMP`.

## Highlighting

`highlight(root)` classifies the tokens of a tree for syntax highlighting, in the order of the text. A word is a keyword only where the parser read it as one, so a column named `name` is a name in `|> SELECT name` and `NAME` is a keyword in `BY NAME`. A name takes the role of the node around it, with the roles of completion: the path of a `FROM` item is a table, the path of a call is a function, and the first name of a path in an expression is a column. A function after `FROM` or a pipe `CALL` is a table function, and a `CREATE` tells the kind of the function that it defines, while the syntax leaves the kind of another call unresolved. In the options of the files that a statement reads, an external table's `OPTIONS (...)` or `FROM FILES (...)`, the string of `format` is a `Format` and each string of `uris` a `Uri`, so that a theme can show a format as a constant and a URI as a path. Each range also says whether it defines a name, as an alias or a parameter does. Only scope tells a column from a variable of a script or a range variable, and a scalar function from an aggregate, so a semantic model refines these ranges, as avenger-sql's `Model::refine_highlights` does. The refinement also gives the classes that the syntax never gives: `RangeVariable`, the roles of the functions of avenger-chart, and `EnumValue`, a string that a parameter takes as one value of a set.

## Layout

`layout` lays out text in lines of at most 100 characters. A `Doc` is text with the places where it can break, and a group of a `Doc` stays on one line when it fits in the rest of its line, and otherwise breaks while the groups inside it decide again. `list` lays out a call or an array: on one line, or with each item on a line of its own, a step deeper than the line that opens it, followed by a comma, and the closer back at that line's depth. The step is `INDENT`, 4, or `PIPE_STEP`, 1, for a list that a pipe operator's line opens, whose depth is that of the operator's keyword, so that its items are 4 deeper than the `|>`. avenger-sql's printer of lifted calls writes the calls that it generates with it.

## Formatting

`format::format(text, role)` writes a text of `role` in the style of avenger-sql's `STYLE.md`. It reads the syntax tree alone and changes the whitespace between tokens, the case of keywords and of the names of functions, and the commas after the last items of lists. Each kind of part has its rule of layout on `layout`: pipe operators on lines of their own, lists in brackets and the items of pipe operators with one item to a line when they break, chains of binary operators broken before their operators, and the clauses of a statement that defines a table, a view, or a schema, such as `OPTIONS` and `FROM FILES`, on lines of their own when the statement breaks. A comma after the last item of a list keeps it broken, and a comment stays before or after the same tokens. A statement with an error stays as written, so the role matters: a statement of a data script in a chart script, such as a `CREATE VIEW`, has an error and stays as written. The tests check that the formatted text of every corpus input holds the same tokens and comments, parses as the input does, and formats to itself, and an ignored test checks the same of GoogleSQL's parser corpus.

## Indentation

`indent::depth(text, line_start)` gives the depth of a line by the formatter's style, for an editor's Enter and Tab. It reads the lexer's tokens, not the syntax tree: on the unfinished text of an edit, the tree's shape is the parser's recovery, and the formatter keeps the statement as written. The innermost open bracket gives its items 4 deeper than its line, or than the `|>` of a pipe operator's line, and its closer the depth of its line, `|>` takes the depth of its query, `GROUP BY`, `WINDOW`, and a join's `ON` and `USING` the depth of the operator's keyword, and a line that starts with a binary operator, or that follows a line that ends with one, continues the chain of the line before it, or of the line of the bracket that a closer before it closes. A line without a first token, as after Enter at the end of a line, takes the depth of an item after a comma or a keyword that starts a list, and otherwise of the next pipe operator. It reads the formatter's depths, its table of the clauses that go on lines of their own, and the parser's levels of the binary operators, so a change to one of them reaches both. The rules need no role: the clauses of a data script's statements are at the depth of the statement, and their lists and queries are in brackets. The tests check that it gives every line of the formatted corpora its indentation, data scripts included.

## Tests

The tests check the corpora in `tests/common` against GoogleSQL's grammar and parser tests, and compare five trees with the baselines in `tests/baselines`. A test that finds a different tree writes it to `tests/output/`, and copying it to `tests/baselines/` accepts it.

## Conformance with GoogleSQL

The ignored test `tests/conformance.rs` compares the parser with GoogleSQL's `execute_query` 2026.11.1: it parses each input of a corpus with `execute_query --mode=parse --sql_mode=script --enabled_language_features=MAXIMUM` and with `parse`, and compares whether each accepts the input and where each finds its first error. The corpus is the inputs of this crate's tests and every input of GoogleSQL's parser tests apart from the tests of SQL graph syntax, including the pipe operators of a standard query with the query before its first `|>` written as `FROM t`. The inputs of the tests of `CREATE EXTERNAL TABLE`, `CREATE VIEW` and `CREATE SCHEMA`, and this crate's data scripts, parse with `parse_data`, and the others with `parse`. An input that avenger reports as unsupported agrees where GoogleSQL accepts it. A difference that `tests/conformance/known.txt` lists, such as a check that GoogleSQL makes as it analyzes, is explained, and any other fails the test, as does an entry that explains no difference. The report is in `tests/output/conformance.md`.

The check needs the binary at `scratch/tools/googlesql/execute_query_macos` and the corpus, which `tests/conformance/extract.py` writes to `scratch/tools/googlesql/parser-corpus.txt` from a checkout of GoogleSQL at the binary's tag, so the repository holds no text of GoogleSQL's tests. Without them, the test skips with a message. From the repository's root:

```bash
python3 avenger-sql-syntax/tests/conformance/extract.py
```

```bash
CARGO_TARGET_DIR=target cargo test --release -p avenger-sql-syntax --test conformance -- --ignored
```
