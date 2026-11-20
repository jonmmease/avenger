# The style of avenger-sql scripts

This is the layout of avenger-sql scripts. avenger-sql-syntax's formatter, `format::format`, writes it, and avenger-sql-language's `Document::formatting` gives its edits to an editor.

## Lines and depth

- A line holds at most 100 characters.
- Indentation is spaces only.
- The depth of a line is the column of its first word. The depth of a pipe operator's line is the column of the operator's keyword, after `|> `, so the pipe operators of a statement at column 0 have lines of depth 3.
- A part that breaks onto lines of its own is two spaces deeper than the depth of the line that opens it, and the closer of the part goes back to that depth. Each rule below applies this rule to a kind of part.

## Statements

- Each statement starts at column 0, and a statement other than the last ends with `;`.
- A blank line stays where the author wrote one: between statements, between pipe operators, and between the items of a list. A run of blank lines becomes one. Blank lines right after an opener, right before a closer, and at the start or end of the script are removed, and the formatter never adds one. A blank line inside a list keeps the list broken, as a comment does.
- A `DECLARE`, `SET`, `IMPORT`, `CREATE`, or `LOAD DATA` statement stays on one line when it fits, and the expression or the query in it otherwise breaks by the rules below.
- A statement that defines a table, a view, or a schema, as a data script's statements do, puts its clauses `WITH PARTITION COLUMNS`, `OPTIONS`, and `FROM FILES` on lines of their own at column 0 when it does not fit on one line. Its name and its column list stay on the line of its keywords, a column list follows the name after a space, and `AS` and its query follow the last clause, as the query in parentheses of any statement does.

  ```sql
  CREATE EXTERNAL TABLE vega_datasets.cars
  OPTIONS (format = 'JSON', uris = ['data/files/vega_datasets/cars.json']);

  LOAD DATA INTO TEMP TABLE staging.rates (day DATE, rate FLOAT64)
  OPTIONS (description = 'daily rates')
  FROM FILES (format = 'CSV', uris = ['data/files/rates.csv'], skip_leading_rows = 1)
  WITH PARTITION COLUMNS (region STRING);

  CREATE VIEW finance.recent OPTIONS (description = 'the orders of the last week') AS (
    FROM finance.orders
    |> WHERE day > DATE '2025-01-01'
  );
  ```

## Pipe queries

- A query without pipe operators stays on one line when it fits. Otherwise, `FROM` or the parenthesized query starts the first line, and each `|>` starts a line of its own at the depth of the query.
- An operator stays on one line when it fits, its lists have no comma after their last item, and it holds no comment. The lists are the items of `SELECT`, `EXTEND`, `AGGREGATE`, `SET`, `DROP`, `RENAME`, `WITH`, and the set operations, and the lists of their clauses, such as `GROUP BY`.
- Otherwise, the operator's clauses, such as `GROUP BY`, first go on lines of their own at the depth of the keyword. When the items still do not fit, they go on lines of their own at the depth of the keyword plus 2, each followed by a comma, as the arguments of a call do. A clause's list breaks by the same rule.

  ```sql
  |> AGGREGATE
       avg(body_mass_g) AS mu,
       stddev(body_mass_g) AS sigma,
       count(*) AS n,
     GROUP BY species
  ```

- An operator with one item keeps the item on the operator's line, and a long item, such as a mark call, breaks inside itself:

  ```sql
  |> AGGREGATE mark_bar(
       x => species,
       y => count(*),
     ) AS chart
     GROUP BY island
  ```

- A parenthesized query, as in a `JOIN` or a subquery, stays on one line when it fits and has no pipe operators. Otherwise, it breaks after `(`, the query is at the depth of the line plus 2, and the `)` goes on a line of its own at the depth of the line, followed by the rest of the line, such as `AS t ON ...`.

## Calls and lists in brackets

The width and a comma after the last item decide the layout of a call, an array, a struct, and the other lists in brackets:

- A list stays on one line when it fits on its line, has no comma after its last item, and holds no comment.
- Otherwise, it breaks after its opener. Each item goes on a line of its own at the depth of the line plus 2, followed by a comma, and the closer goes on a line of its own at the depth of the line. The items follow the rules again at their own depth.
- So a comma after the last item keeps a list broken, and removing the comma lets a list that fits collapse onto one line. A list that the width breaks takes the comma, so it stays broken when it is formatted again.
- A clause of a call, such as `ORDER BY` in a mark call, goes on a line of its own after the arguments, without a comma.

```sql
|> AGGREGATE mark_bar(x => species, y => count(*))
|> AGGREGATE mark_bar(
     x => species,
     y => count(*),
   )
```

- A window's specification, `OVER (...)`, stays on one line when it fits, and otherwise puts each of its clauses on a line of its own, two spaces deeper.

## Expressions

- Spaces inside a line are normalized: one space around binary operators, `=>`, and keywords, and after a comma. No space follows an opener, and none comes before a closer or a comma. A call's name, a path, and a type's angle brackets have no spaces inside.
- A chain of binary operators, such as `AND`, `||`, or arithmetic, breaks before its operators of the lowest precedence when it does not fit. Each of those operators starts a line at the depth of the chain's line plus 2. An operand that still does not fit breaks by its own operators or by the rule of lists.

  ```sql
  |> WHERE species = chosen
       AND body_mass_g IS NOT NULL
       AND flipper_length_mm > 200
  ```

- A `CASE` stays on one line when it fits, and otherwise puts each `WHEN` and the `ELSE` on a line of its own, two spaces deeper, and `END` back at the depth of its line.
- A string, a quoted name, or a number stays as written, even when it is longer than the width.

## Case

- Keywords are upper case: `FROM`, `AS`, `NULL`, `TRUE`, `OVER`.
- The names of functions are lower case, GoogleSQL's functions included, such as `count` and `avg`, since GoogleSQL's function names are case-insensitive. A form that reads like a call, such as `CAST(x AS INT64)` or `EXTRACT`, is keywords and stays upper case.
- The names of tables, columns, types, and variables stay as written.

## Comments

- The formatter never removes a comment, and a comment stays before or after the same tokens.
- A list that holds a comment stays broken, since nothing can follow a line comment on its line.
- A comment on a line of its own takes the depth of the line after it, so a comment above an argument stays above it at the argument's depth. A comment after the last argument takes the depth of the arguments.
- A comment at the end of a line stays at the end of its line, one space after its tokens.

```sql
|> AGGREGATE mark_bar(
     -- the x column
     x => species,
     -- the aggregation
     y => count(*),
   )
```

## What stays as written

- A statement with an error, from the `;` before it to its own `;`, stays as written, and the other statements are formatted.
- A part without a rule above keeps its tokens on one line, with the spaces of the style.
