; The highlighting of avenger-sql, a translation of the classes of avenger-sql-syntax's
; `highlight.rs`. A name's capture is its role, which the node around it gives, as in
; `name_role` and `path_role`: the path of a table source is a table, the path of a call is a
; function, and the first name of a path expression is a column. The capture names are Zed's,
; and q2's palette spells two of them differently: `comment.doc` and `variable.special`.
;
; No node matches two patterns, and no two captures share a range, because hosts disagree on
; which of two wins. Where `highlight.rs` gives a specific case precedence, the general pattern
; leaves the case out, or the specific capture is on a smaller node inside the general one,
; which hosts draw over it.

; Comments

(comment) @comment

(hint) @comment

(doc_comment) @comment.documentation

; Literals

[
  (string)
  (bytes)
] @string

(escape_sequence) @string.escape

; The value of `format` in the options of the files that `FROM FILES` or an external table reads,
; a constant such as `'CSV'`. The capture is on the content, inside the string's own capture.
; The predicate reads the option's name at the start of its entry, because the Tree-sitter
; highlighter drops a match whose node a later pattern also captures, as the pattern of options
; captures the name. The URIs of `uris` stay strings, as the language server's semantic tokens
; give them.
(from_files
  (option_entry
    (literal
      (string
        (string_content) @string.special))) @_entry
  (#match? @_entry "^[fF][oO][rR][mM][aA][tT][ \t\r\n]*="))

(create_external_table_stmt
  (options
    (option_entry
      (literal
        (string
          (string_content) @string.special))) @_entry)
  (#match? @_entry "^[fF][oO][rR][mM][aA][tT][ \t\r\n]*="))

; The value of a typed literal: a date, a time, or a range, as `2024-01-01` in
; `DATE '2024-01-01'`, a constant of its own, and a number of `NUMERIC` and its kin. The capture
; is on the content, inside the string's own capture, so the quotes stay strings. The injection
; of `avenger-datetime` draws the parts of a date or a time over it, as its digits, where a host
; draws injections.
(typed_literal
  [
    "DATE"
    "TIME"
    "DATETIME"
    "TIMESTAMP"
    "RANGE"
  ]
  (string
    (string_content) @string.special))

(typed_literal
  [
    "NUMERIC"
    "DECIMAL"
    "BIGNUMERIC"
    "BIGDECIMAL"
  ]
  (string
    (string_content) @number))

; A number of a literal. The digits of a dashed table name, as in `my-project-2`, are part of
; the name.
(literal
  [
    (integer)
    (float)
  ] @number)

[
  "TRUE"
  "FALSE"
] @boolean

"NULL" @constant.builtin

; Query parameters

(param) @variable.builtin

(param_expr
  "?" @variable.builtin)

; Operators. `?`, `^`, `$`, `|`, `||`, `*`, and `+` are the operators of a pattern of
; `MATCH_RECOGNIZE` too. `-` is an operator of an expression, and part of a dashed table name.

[
  "="
  "!="
  "<"
  "<="
  ">"
  ">="
  "+"
  "*"
  "/"
  "||"
  "|"
  "&"
  "^"
  "~"
  "<<"
  ">>"
  "=>"
  "|>"
  "$"
  ".*"
] @operator

(bin_expr
  "-" @operator)

(prefix_expr
  "-" @operator)

(quantifier
  "?" @operator)

; Punctuation

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
] @punctuation.bracket

[
  ","
  ";"
  "."
] @punctuation.delimiter

; The names that nodes define

(alias
  (name) @variable)

(pipe_as
  (name) @variable)

(rename_item
  (name) @variable)

(measure
  (name) @variable)

(replace_item
  (name) @variable)

(column_def
  (name) @variable)

(declare_stmt
  (name) @variable)

(pattern_definition
  (name) @variable)

(struct_type
  (struct_field
    (name) @property))

(struct_expr
  (struct_field
    (name) @property))

; The fields of a `TABLE<...>` type are its columns.
(table_type
  (struct_field
    (name) @variable))

(named_query
  (name) @type)

(named_window
  (name) @label)

(parameter
  (name) @variable.parameter)

(option_entry
  (name) @property)

; The names that refer to definitions

(named_type
  (name_ref) @type.builtin)

(field_expr
  (name_ref) @property)

(name_list
  (name_ref) @variable)

(set_stmt
  (name_ref) @variable)

(pipe_drop
  (name_ref) @variable)

(assignment
  (name_ref) @variable)

(rename_item
  (name_ref) @variable)

(named_arg
  (name_ref) @variable.parameter)

(named_table_arg
  (name_ref) @variable.parameter)

(over_clause
  (name_ref) @label)

(window_spec
  (name_ref) @label)

(named_window
  (name_ref) @label)

(interval_expr
  (name_ref) @constant)

(pattern_variable
  (name_ref) @variable)

; The names of paths, by the node that holds the path

(table_source
  (path
    (name_ref) @type))

(table_arg
  (path
    (name_ref) @type))

(named_table_arg
  (path
    (name_ref) @type))

(wildcard
  (path
    (name_ref) @type))

(create_table_stmt
  (path
    (name_ref) @type))

(create_external_table_stmt
  (path
    (name_ref) @type))

(create_view_stmt
  (path
    (name_ref) @type))

(load_data_stmt
  (path
    (name_ref) @type))

(tvf_source
  (path
    (name_ref) @function))

(pipe_call
  (path
    (name_ref) @function))

(call_expr
  (path
    (name_ref) @function))

(create_function_stmt
  (path
    (name_ref) @function))

(import_stmt
  (path
    (name_ref) @namespace))

(module_stmt
  (path
    (name_ref) @namespace))

(create_schema_stmt
  (path
    (name_ref) @namespace))

(create_constant_stmt
  (path
    (name_ref) @variable))

; The columns of an `UNPIVOT`: the value columns and the label column, which are new names, and
; the columns of its items.
(path_list
  (path
    (name_ref) @variable))

(unpivot_clause
  (path
    (name_ref) @variable))

; A path expression: a column, and its fields. The first name of the date part of an `EXTRACT`,
; as `MONTH` is in `EXTRACT(MONTH FROM d)`, is a date part.
; generated: the columns of path expressions, by scripts/sync_queries.mjs
[
  (aggregate_item (path_expr (path . (name_ref) @variable)))
  (aliased_expr (path_expr (path . (name_ref) @variable)))
  (arg (path_expr (path . (name_ref) @variable)))
  (array_expr (path_expr (path . (name_ref) @variable)))
  (assignment (path_expr (path . (name_ref) @variable)))
  (between_expr (path_expr (path . (name_ref) @variable)))
  (bin_expr (path_expr (path . (name_ref) @variable)))
  (call_limit (path_expr (path . (name_ref) @variable)))
  (case_expr (path_expr (path . (name_ref) @variable)))
  (cast_expr (path_expr (path . (name_ref) @variable)))
  (create_constant_stmt (path_expr (path . (name_ref) @variable)))
  (cube (path_expr (path . (name_ref) @variable)))
  (declare_stmt (path_expr (path . (name_ref) @variable)))
  (else_clause (path_expr (path . (name_ref) @variable)))
  (format_clause (path_expr (path . (name_ref) @variable)))
  (frame_bound (path_expr (path . (name_ref) @variable)))
  (function_body (path_expr (path . (name_ref) @variable)))
  (group_item (path_expr (path . (name_ref) @variable)))
  (grouping_sets (path_expr (path . (name_ref) @variable)))
  (having_clause (path_expr (path . (name_ref) @variable)))
  (in_expr (path_expr (path . (name_ref) @variable)))
  (in_list (path_expr (path . (name_ref) @variable)))
  (in_unnest (path_expr (path . (name_ref) @variable)))
  (interval_expr (path_expr (path . (name_ref) @variable)))
  (is_distinct_expr (path_expr (path . (name_ref) @variable)))
  (is_expr (path_expr (path . (name_ref) @variable)))
  (like_expr (path_expr (path . (name_ref) @variable)))
  (limit_offset (path_expr (path . (name_ref) @variable)))
  (measure (path_expr (path . (name_ref) @variable)))
  (named_arg (path_expr (path . (name_ref) @variable)))
  (on_condition (path_expr (path . (name_ref) @variable)))
  (option_entry (path_expr (path . (name_ref) @variable)))
  (order_key (path_expr (path . (name_ref) @variable)))
  (paren_expr (path_expr (path . (name_ref) @variable)))
  (partition_by (path_expr (path . (name_ref) @variable)))
  (pattern_definition (path_expr (path . (name_ref) @variable)))
  (pipe_limit (path_expr (path . (name_ref) @variable)))
  (pipe_where (path_expr (path . (name_ref) @variable)))
  (pivot_clause (path_expr (path . (name_ref) @variable)))
  (prefix_expr (path_expr (path . (name_ref) @variable)))
  (quantifier_max (path_expr (path . (name_ref) @variable)))
  (quantifier_min (path_expr (path . (name_ref) @variable)))
  (repeatable (path_expr (path . (name_ref) @variable)))
  (replace_item (path_expr (path . (name_ref) @variable)))
  (rollup (path_expr (path . (name_ref) @variable)))
  (sample_size (path_expr (path . (name_ref) @variable)))
  (select_item (path_expr (path . (name_ref) @variable)))
  (set_stmt (path_expr (path . (name_ref) @variable)))
  (subscript_expr (path_expr (path . (name_ref) @variable)))
  (time_zone (path_expr (path . (name_ref) @variable)))
  (tuple_expr (path_expr (path . (name_ref) @variable)))
  (when_clause (path_expr (path . (name_ref) @variable)))
]
; end generated

(extract_expr
  value: (path_expr
    (path
      .
      (name_ref) @variable)))

(extract_expr
  part: (path_expr
    (path
      .
      (name_ref) @constant)))

(path_expr
  (path
    (name_ref)
    (name_ref) @property))

; Keywords

; generated: the keywords of src/node-types.json, by scripts/sync_queries.mjs
[
  "AFTER"
  "AGGREGATE"
  "ALL"
  "AND"
  "ANY"
  "ARRAY"
  "AS"
  "ASC"
  "AT"
  "BETWEEN"
  "BIGDECIMAL"
  "BIGNUMERIC"
  "BY"
  "CALL"
  "CASE"
  "CAST"
  "COLUMNS"
  "CONSTANT"
  "CORRESPONDING"
  "CREATE"
  "CROSS"
  "CUBE"
  "CURRENT"
  "DATA"
  "DATE"
  "DATETIME"
  "DECIMAL"
  "DECLARE"
  "DEFAULT"
  "DEFINE"
  "DESC"
  "DISTINCT"
  "DROP"
  "ELSE"
  "END"
  "EXCEPT"
  "EXCLUDE"
  "EXISTS"
  "EXTEND"
  "EXTERNAL"
  "EXTRACT"
  "FILES"
  "FIRST"
  "FOLLOWING"
  "FOR"
  "FORMAT"
  "FROM"
  "FULL"
  "FUNCTION"
  "GROUP"
  "GROUPING"
  "HAVING"
  "IF"
  "IGNORE"
  "IMPORT"
  "IN"
  "INCLUDE"
  "INNER"
  "INTERSECT"
  "INTERVAL"
  "INTO"
  "IS"
  "JOIN"
  "JSON"
  "LAST"
  "LATERAL"
  "LEFT"
  "LIKE"
  "LIMIT"
  "LOAD"
  "MATCH"
  "MATCH_RECOGNIZE"
  "MAX"
  "MEASURES"
  "MIN"
  "MODULE"
  "NAME"
  "NATURAL"
  "NEXT"
  "NOT"
  "NULLS"
  "NUMERIC"
  "OFFSET"
  "ON"
  "OPTIONS"
  "OR"
  "ORDER"
  "OUTER"
  "OVER"
  "OVERWRITE"
  "PARTITION"
  "PAST"
  "PATTERN"
  "PERCENT"
  "PIVOT"
  "PRECEDING"
  "PRIVATE"
  "PUBLIC"
  "RANGE"
  "RENAME"
  "REPEATABLE"
  "REPLACE"
  "RESPECT"
  "RETURNS"
  "RIGHT"
  "ROLLUP"
  "ROW"
  "ROWS"
  "SAFE_CAST"
  "SCHEMA"
  "SELECT"
  "SET"
  "SETS"
  "SKIP"
  "SOME"
  "STRICT"
  "STRUCT"
  "TABLE"
  "TABLESAMPLE"
  "TEMP"
  "TEMPORARY"
  "THEN"
  "TIME"
  "TIMESTAMP"
  "TO"
  "TYPE"
  "UNBOUNDED"
  "UNION"
  "UNKNOWN"
  "UNNEST"
  "UNPIVOT"
  "USING"
  "VALUE"
  "VIEW"
  "WEIGHT"
  "WHEN"
  "WHERE"
  "WINDOW"
  "WITH"
  "ZONE"
] @keyword
; end generated
