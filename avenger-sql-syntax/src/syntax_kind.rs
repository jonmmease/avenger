//! The kinds of the tokens and nodes of the syntax tree.
//!
//! The lexer gives every keyword and name the kind [`SyntaxKind::WORD`]. When the parser reads a
//! word as a keyword, the tree holds it with that keyword's kind, such as
//! [`SyntaxKind::SELECT_KW`], so a non-reserved keyword, such as `OFFSET`, is a keyword where it
//! acts as one and a name where it is a name.

macro_rules! syntax_kinds {
    (
        tokens: [$($(#[$token_meta:meta])* $token:ident),* $(,)?],
        keywords: [$($keyword:ident = $text:literal),* $(,)?],
        nodes: [$($(#[$node_meta:meta])* $node:ident),* $(,)?] $(,)?
    ) => {
        /// The kind of a token or a node.
        #[allow(non_camel_case_types)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(u16)]
        pub enum SyntaxKind {
            $($(#[$token_meta])* $token,)*
            $($keyword,)*
            $($(#[$node_meta])* $node,)*
            #[doc(hidden)]
            __LAST,
        }

        /// The keywords and their kinds, in the order of [`SyntaxKind`].
        const KEYWORDS: &[(&str, SyntaxKind)] = &[$(($text, SyntaxKind::$keyword)),*];

        const FIRST_KEYWORD: u16 = { let kinds = [$(SyntaxKind::$keyword),*]; kinds[0] as u16 };
        const LAST_KEYWORD: u16 = { let kinds = [$(SyntaxKind::$keyword),*]; kinds[kinds.len() - 1] as u16 };
    };
}

syntax_kinds! {
    tokens: [
        /// Spaces, tabs, line ends and the other whitespace of GoogleSQL.
        WHITESPACE,
        /// `#`, `--` or `/* */`.
        COMMENT,
        /// A keyword or an unquoted name, before the parser reads it.
        WORD,
        /// A name in backticks.
        QUOTED_IDENT,
        STRING,
        BYTES,
        INTEGER,
        FLOAT,
        /// `@name`
        PARAM,
        /// `?`
        POSITIONAL,
        /// `|>`
        PIPE,
        L_PAREN,
        R_PAREN,
        L_BRACK,
        R_BRACK,
        COMMA,
        DOT,
        SEMICOLON,
        EQ,
        /// `!=`
        NEQ,
        LT,
        LT_EQ,
        GT,
        GT_EQ,
        PLUS,
        MINUS,
        STAR,
        SLASH,
        /// `||`
        CONCAT,
        /// `|`
        BIT_OR,
        AMP,
        CARET,
        TILDE,
        /// `<<`
        SHL,
        /// `=>`
        FAT_ARROW,
        /// A character outside the grammar, or a malformed token.
        ERROR_TOKEN,
        /// The end of the text, which the parser sees and the tree does not hold.
        EOF,
    ],
    keywords: [
        AGGREGATE_KW = "AGGREGATE",
        ALL_KW = "ALL",
        AND_KW = "AND",
        ANY_KW = "ANY",
        ARRAY_KW = "ARRAY",
        AS_KW = "AS",
        ASC_KW = "ASC",
        ASSERT_ROWS_MODIFIED_KW = "ASSERT_ROWS_MODIFIED",
        AT_KW = "AT",
        BETWEEN_KW = "BETWEEN",
        BIGDECIMAL_KW = "BIGDECIMAL",
        BIGNUMERIC_KW = "BIGNUMERIC",
        BY_KW = "BY",
        CALL_KW = "CALL",
        CASE_KW = "CASE",
        CAST_KW = "CAST",
        COLLATE_KW = "COLLATE",
        CONSTANT_KW = "CONSTANT",
        CONTAINS_KW = "CONTAINS",
        CORRESPONDING_KW = "CORRESPONDING",
        CREATE_KW = "CREATE",
        CROSS_KW = "CROSS",
        CUBE_KW = "CUBE",
        CURRENT_KW = "CURRENT",
        DATE_KW = "DATE",
        DATETIME_KW = "DATETIME",
        DECIMAL_KW = "DECIMAL",
        DECLARE_KW = "DECLARE",
        DEFAULT_KW = "DEFAULT",
        DEFINE_KW = "DEFINE",
        DESC_KW = "DESC",
        DISTINCT_KW = "DISTINCT",
        DROP_KW = "DROP",
        ELSE_KW = "ELSE",
        END_KW = "END",
        ENUM_KW = "ENUM",
        ESCAPE_KW = "ESCAPE",
        EXCEPT_KW = "EXCEPT",
        EXCLUDE_KW = "EXCLUDE",
        EXISTS_KW = "EXISTS",
        EXTEND_KW = "EXTEND",
        EXTRACT_KW = "EXTRACT",
        FALSE_KW = "FALSE",
        FETCH_KW = "FETCH",
        FIRST_KW = "FIRST",
        FOLLOWING_KW = "FOLLOWING",
        FOR_KW = "FOR",
        FORMAT_KW = "FORMAT",
        FROM_KW = "FROM",
        FULL_KW = "FULL",
        FUNCTION_KW = "FUNCTION",
        GROUP_KW = "GROUP",
        GROUPING_KW = "GROUPING",
        GROUPS_KW = "GROUPS",
        HASH_KW = "HASH",
        HAVING_KW = "HAVING",
        IF_KW = "IF",
        IGNORE_KW = "IGNORE",
        IMPORT_KW = "IMPORT",
        IN_KW = "IN",
        INCLUDE_KW = "INCLUDE",
        INNER_KW = "INNER",
        INTERSECT_KW = "INTERSECT",
        INTERVAL_KW = "INTERVAL",
        INTO_KW = "INTO",
        IS_KW = "IS",
        JOIN_KW = "JOIN",
        JSON_KW = "JSON",
        LANGUAGE_KW = "LANGUAGE",
        LAST_KW = "LAST",
        LATERAL_KW = "LATERAL",
        LEFT_KW = "LEFT",
        LIKE_KW = "LIKE",
        LIMIT_KW = "LIMIT",
        LOOKUP_KW = "LOOKUP",
        MAX_KW = "MAX",
        MERGE_KW = "MERGE",
        MIN_KW = "MIN",
        MODULE_KW = "MODULE",
        NAME_KW = "NAME",
        NATURAL_KW = "NATURAL",
        NEW_KW = "NEW",
        NO_KW = "NO",
        NOT_KW = "NOT",
        NULL_KW = "NULL",
        NULLS_KW = "NULLS",
        NUMERIC_KW = "NUMERIC",
        OF_KW = "OF",
        OFFSET_KW = "OFFSET",
        ON_KW = "ON",
        OPTIONS_KW = "OPTIONS",
        OR_KW = "OR",
        ORDER_KW = "ORDER",
        OUTER_KW = "OUTER",
        OVER_KW = "OVER",
        PARTITION_KW = "PARTITION",
        PERCENT_KW = "PERCENT",
        PIVOT_KW = "PIVOT",
        PRECEDING_KW = "PRECEDING",
        PRIVATE_KW = "PRIVATE",
        PROTO_KW = "PROTO",
        PUBLIC_KW = "PUBLIC",
        QUALIFY_KW = "QUALIFY",
        RANGE_KW = "RANGE",
        RECURSIVE_KW = "RECURSIVE",
        RENAME_KW = "RENAME",
        REPEATABLE_KW = "REPEATABLE",
        REPLACE_KW = "REPLACE",
        RESPECT_KW = "RESPECT",
        RETURNS_KW = "RETURNS",
        RIGHT_KW = "RIGHT",
        ROLLUP_KW = "ROLLUP",
        ROW_KW = "ROW",
        ROWS_KW = "ROWS",
        SAFE_CAST_KW = "SAFE_CAST",
        SELECT_KW = "SELECT",
        SET_KW = "SET",
        SETS_KW = "SETS",
        SOME_KW = "SOME",
        STRICT_KW = "STRICT",
        STRUCT_KW = "STRUCT",
        SYSTEM_TIME_KW = "SYSTEM_TIME",
        TABLE_KW = "TABLE",
        TABLESAMPLE_KW = "TABLESAMPLE",
        TEMP_KW = "TEMP",
        TEMPORARY_KW = "TEMPORARY",
        THEN_KW = "THEN",
        TIME_KW = "TIME",
        TIMESTAMP_KW = "TIMESTAMP",
        TO_KW = "TO",
        TREAT_KW = "TREAT",
        TRUE_KW = "TRUE",
        TYPE_KW = "TYPE",
        UNBOUNDED_KW = "UNBOUNDED",
        UNION_KW = "UNION",
        UNKNOWN_KW = "UNKNOWN",
        UNNEST_KW = "UNNEST",
        UNPIVOT_KW = "UNPIVOT",
        USING_KW = "USING",
        WEIGHT_KW = "WEIGHT",
        WHEN_KW = "WHEN",
        WHERE_KW = "WHERE",
        WINDOW_KW = "WINDOW",
        WITH_KW = "WITH",
        WITHIN_KW = "WITHIN",
        ZONE_KW = "ZONE",
    ],
    nodes: [
        /// Tokens that the parser skipped while it recovered from an error.
        ERROR,
        /// A script: statements separated by `;`.
        SCRIPT,
        /// A module: `MODULE path [OPTIONS (...)]` and its statements.
        MODULE,
        MODULE_STMT,
        IMPORT_STMT,
        DECLARE_STMT,
        SET_STMT,
        CREATE_TABLE_STMT,
        CREATE_FUNCTION_STMT,
        CREATE_CONSTANT_STMT,
        /// A name that a node defines: an alias, a `WITH` name, a parameter, or a `CREATE` name.
        NAME,
        /// A name that refers to a definition: a column, a table, a function, or a module.
        NAME_REF,
        /// Names separated by dots.
        PATH,
        /// `[AS] name`
        ALIAS,
        /// `(name, ...)`
        NAME_LIST,
        OPTIONS,
        OPTION_ENTRY,
        /// The modifiers of a `CREATE`: `OR REPLACE`, the scope, and `IF NOT EXISTS`.
        CREATE_MODE,
        COLUMN_LIST,
        COLUMN_DEF,
        PARAM_LIST,
        PARAMETER,
        RETURNS_CLAUSE,
        FUNCTION_BODY,
        NAMED_TYPE,
        TYPE_PARAMS,
        ARRAY_TYPE,
        STRUCT_TYPE,
        STRUCT_FIELD,
        RANGE_TYPE,
        TABLE_TYPE,
        ANY_TYPE,
        ANY_TABLE,
        /// A query: a `FROM` clause, or a query in parentheses, and pipe operators.
        QUERY,
        PAREN_QUERY,
        FROM_CLAUSE,
        TABLE_SOURCE,
        SUBQUERY_SOURCE,
        UNNEST_SOURCE,
        TVF_SOURCE,
        PAREN_JOIN,
        JOIN,
        JOIN_OP,
        ON_CONDITION,
        USING_CONDITION,
        WITH_OFFSET,
        PIVOT_SOURCE,
        UNPIVOT_SOURCE,
        SAMPLE_SOURCE,
        SAMPLE_CLAUSE,
        SAMPLE_SIZE,
        REPEATABLE,
        WEIGHT,
        PIVOT_CLAUSE,
        UNPIVOT_CLAUSE,
        UNPIVOT_COLUMNS,
        ALIASED_EXPR,
        PIPE_SELECT,
        PIPE_EXTEND,
        PIPE_SET,
        PIPE_DROP,
        PIPE_RENAME,
        PIPE_AS,
        PIPE_WHERE,
        PIPE_AGGREGATE,
        PIPE_DISTINCT,
        PIPE_ORDER_BY,
        PIPE_LIMIT,
        PIPE_SET_OPERATION,
        PIPE_JOIN,
        PIPE_CALL,
        PIPE_SAMPLE,
        PIPE_WITH,
        PIPE_PIVOT,
        PIPE_UNPIVOT,
        SELECT_ITEM,
        WILDCARD,
        EXCEPT_LIST,
        REPLACE_LIST,
        REPLACE_ITEM,
        WINDOW_CLAUSE,
        NAMED_WINDOW,
        ASSIGNMENT,
        RENAME_ITEM,
        AGGREGATE_ITEM,
        ORDER_SUFFIX,
        GROUP_BY,
        GROUP_ITEM,
        GROUPING_EMPTY,
        ROLLUP,
        CUBE,
        GROUPING_SETS,
        ORDER_BY,
        ORDER_KEY,
        LIMIT_OFFSET,
        SET_OP_MODE,
        BY_NAME,
        CORRESPONDING,
        NAMED_QUERY,
        LITERAL,
        TYPED_LITERAL,
        INTERVAL_EXPR,
        PARAM_EXPR,
        PATH_EXPR,
        FIELD_EXPR,
        SUBSCRIPT_EXPR,
        PREFIX_EXPR,
        BIN_EXPR,
        IS_EXPR,
        IS_DISTINCT_EXPR,
        BETWEEN_EXPR,
        IN_EXPR,
        IN_LIST,
        IN_QUERY,
        IN_UNNEST,
        LIKE_EXPR,
        QUANTIFIED_PATTERNS,
        CASE_EXPR,
        WHEN_CLAUSE,
        ELSE_CLAUSE,
        CAST_EXPR,
        FORMAT_CLAUSE,
        TIME_ZONE,
        EXTRACT_EXPR,
        ARRAY_EXPR,
        STRUCT_EXPR,
        TUPLE_EXPR,
        PAREN_EXPR,
        CALL_EXPR,
        ARG_LIST,
        ARG,
        NAMED_ARG,
        STAR_ARG,
        TABLE_ARG,
        NAMED_TABLE_ARG,
        NULL_TREATMENT,
        HAVING_CLAUSE,
        CALL_LIMIT,
        OVER_CLAUSE,
        WINDOW_SPEC,
        PARTITION_BY,
        FRAME,
        FRAME_BOUND,
        SUBQUERY_EXPR,
        EXISTS_EXPR,
        ARRAY_SUBQUERY_EXPR,
    ],
}

impl SyntaxKind {
    /// The keyword kind of `word`, without regard to case.
    pub fn keyword(word: &str) -> Option<SyntaxKind> {
        KEYWORDS
            .iter()
            .find(|(text, _)| text.eq_ignore_ascii_case(word))
            .map(|(_, kind)| *kind)
    }

    pub fn is_keyword(self) -> bool {
        (FIRST_KEYWORD..=LAST_KEYWORD).contains(&(self as u16))
    }

    /// Whitespace and comments, which the parser skips.
    pub fn is_trivia(self) -> bool {
        matches!(self, SyntaxKind::WHITESPACE | SyntaxKind::COMMENT)
    }

    fn from_raw(raw: u16) -> SyntaxKind {
        assert!(
            raw < SyntaxKind::__LAST as u16,
            "{raw} is not a syntax kind"
        );
        // SAFETY: `SyntaxKind` is `repr(u16)` and `raw` is below its last variant.
        unsafe { std::mem::transmute::<u16, SyntaxKind>(raw) }
    }
}

/// The language of avenger-sql's syntax tree, for `rowan`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SqlLanguage {}

impl rowan::Language for SqlLanguage {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> SyntaxKind {
        SyntaxKind::from_raw(raw.0)
    }

    fn kind_to_raw(kind: SyntaxKind) -> rowan::SyntaxKind {
        rowan::SyntaxKind(kind as u16)
    }
}

pub type SyntaxNode = rowan::SyntaxNode<SqlLanguage>;
pub type SyntaxToken = rowan::SyntaxToken<SqlLanguage>;
pub type SyntaxElement = rowan::SyntaxElement<SqlLanguage>;
