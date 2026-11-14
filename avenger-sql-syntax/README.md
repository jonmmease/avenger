# avenger-sql-syntax

The syntax of avenger-sql: a subset of GoogleSQL's grammar, read into a lossless syntax tree with typed nodes. The crate reads scripts of pipe queries, `IMPORT MODULE`, `DECLARE`, `SET`, `CREATE TEMP TABLE` and `CREATE FUNCTION`, and `.sqlm` modules of public functions, table functions and constants. Within the subset, it accepts what GoogleSQL accepts, rejects what GoogleSQL rejects, and reports its first error where GoogleSQL does.

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

## The tree

`parse` and `parse_module` never fail. Each returns a `rowan` tree that holds every byte of the text, comments and whitespace included, with a range on every node. Every node is a `SyntaxNode` of a `SyntaxKind`. A comment before a statement or a pipe operator belongs to that node, and no node starts or ends with trivia.

`sql.ungram` describes the children of each kind of node, with notes on the GoogleSQL sources, the subset, and the forms left out. The grammar describes the tree, and the hand-written parser builds it. A test checks that every tree of the test corpora has the shape that the grammar describes.

## Typed nodes

`build.rs` reads `sql.ungram` and generates the node kinds of `SyntaxKind` and the typed nodes of `ast`. A typed node, such as `ast::PipeWhere`, wraps a node of its kind and has an accessor for each child, such as `condition()`. A rule that chooses between nodes, such as `Expr`, is an enum. Since a tree of a text with errors can lack any part, an accessor returns an `Option` or an iterator. `src/ast/manual.rs` adds the accessors that decode text: the values of literals, the text of names, the operators of expressions, and the modifiers of `CREATE` statements.

## Diagnostics

Each error is a `Diagnostic`, with a `DiagnosticCode`, a range, a message, help, and fixes, which are the edits that repair the text. Three stages report errors:

- The lexer reports malformed tokens, such as an unclosed string.
- The parser reports the errors of the grammar. It puts what it cannot read into `ERROR` nodes and goes on at the next `,`, `)`, `|>` or `;`.
- The validation pass, `src/validation.rs`, reports the checks that GoogleSQL makes after its parser: the rules of modules, the order of `DECLARE`, the clauses that each kind of function takes, the joins after comma joins, and the escapes of literals.

## Completion

`expected_at(text, offset)` returns what the grammar accepts at a cursor: keywords, tokens, names in their roles (a column, a table, a function, and so on), expressions, types, pipe operators, and statements. When a word ends at the cursor, the result is what can take that word's place.

## Highlighting

`highlight(root)` classifies the tokens of a tree for syntax highlighting, in the order of the text. A word is a keyword only where the parser read it as one, so a column named `name` is a name in `|> SELECT name` and `NAME` is a keyword in `BY NAME`. A name takes the role of the node around it, with the roles of completion: the path of a `FROM` item is a table, the path of a call is a function, and the first name of a path in an expression is a column. Each range also says whether it defines a name, as an alias or a parameter does. A column and a variable of a script are both a column here, since only scope tells them apart.

## Tests

The tests check the corpora in `tests/common` against GoogleSQL's grammar and parser tests, and compare four trees with the baselines in `tests/baselines`. A test that finds a different tree writes it to `tests/output/`, and copying it to `tests/baselines/` accepts it.
