//! The semantic model of a script: what each operator reads and writes, and what each name refers
//! to, which an editor reads for completion, hover, go-to-definition and highlighting.

use avenger_sql_syntax::{FunctionRole, Offset as TextSize, TextRange};
use datafusion::arrow::datatypes::DataType;

/// The semantic model of a script.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    /// The scope of each operator, in the order in which the pass plans them: the `FROM` clause of
    /// each query, and each of its pipe operators.
    pub operators: Vec<OperatorScope>,
    /// The definition of each name that the text refers to.
    pub names: Vec<NameUse>,
    /// The form of each call and operator that resolved, such as `$add(DATE, INT64) -> DATE`.
    pub calls: Vec<CallForm>,
    /// Each string literal that a parameter takes as one value of a set.
    pub enum_values: Vec<EnumValue>,
    /// The places where the pass plans expressions, and what each takes.
    pub places: Vec<Place>,
    /// The type of the expression before each `.` of a field access.
    pub field_bases: Vec<FieldBase>,
}

/// A place of the text where the pass plans expressions, as the condition of a `WHERE` or the
/// arguments of an aggregate, and what can stand there. The pass reports what a place does not
/// take as an error, and an editor offers what it takes.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    /// The range of the clause or the argument list.
    pub range: TextRange,
    pub kind: PlaceKind,
    /// Whether the place reads the columns of its input. A constant, such as the count of a
    /// `LIMIT`, reads none.
    pub columns: bool,
    /// Whether an aggregate or a mark can stand here.
    pub aggregates: bool,
    /// Whether a window function, or an aggregate with `OVER`, can stand here.
    pub windows: bool,
    /// The clause of a `MATCH_RECOGNIZE` that holds the place.
    pub match_recognize: Option<MatchRecognizePart>,
    /// The type of the value of the place, where the analysis knows it, as `BOOL` of a filter.
    pub expected: Option<DataType>,
}

/// What a place is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlaceKind {
    /// A condition: of a `WHERE` or of a join's `ON`.
    Filter,
    /// An item of `|> SELECT` or `|> EXTEND`, or a value of `|> SET`.
    SelectItem,
    /// An item of `|> AGGREGATE`, which is an aggregate or a mark, or computes from them.
    AggregateItem,
    /// A key of `GROUP BY`.
    GroupKey,
    /// The arguments of an aggregate or a window function, which call neither.
    Argument,
    /// A key of `ORDER BY`.
    OrderKey,
    /// A list of the names of the input's columns, as `CORRESPONDING BY (...)` or `USING (...)`.
    ColumnList,
    /// The aggregates of a `PIVOT`.
    Pivot,
    /// A constant, as the count of a `LIMIT`.
    Constant,
    /// The `MEASURES` of a `MATCH_RECOGNIZE`.
    Measures,
    /// A condition of the `DEFINE` of a `MATCH_RECOGNIZE`.
    Define,
    /// The value of a script variable's `SET`.
    Variable,
    /// The body of a temporary function, which reads its parameters.
    Body,
}

/// A clause of a `MATCH_RECOGNIZE` whose functions only it takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MatchRecognizePart {
    Measures,
    Define,
}

/// The type of the expression before a `.` of a field access.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldBase {
    /// The range of the expression before the `.`.
    pub range: TextRange,
    pub ty: DataType,
}

/// The clause of a `MATCH_RECOGNIZE` that the function `name` belongs to, when it belongs to
/// one: `PREV` and `NEXT` to `DEFINE`, and `FIRST`, `LAST`, `MATCH_NUMBER`, `MATCH_ROW_NUMBER`,
/// and `CLASSIFIER` to `MEASURES`.
pub fn match_recognize_part(name: &str) -> Option<MatchRecognizePart> {
    match name.to_ascii_uppercase().as_str() {
        "PREV" | "NEXT" => Some(MatchRecognizePart::Define),
        "FIRST" | "LAST" | "MATCH_NUMBER" | "MATCH_ROW_NUMBER" | "CLASSIFIER" => {
            Some(MatchRecognizePart::Measures)
        }
        _ => None,
    }
}

impl Place {
    /// Whether a call of `function` can stand here: a function of a `MATCH_RECOGNIZE` in its
    /// clause, a mark where aggregates can, an aggregate where aggregates can, or windows can and
    /// it takes `OVER`, a
    /// window function where windows can, and a scalar function anywhere but a list of columns
    /// and a `PIVOT`, and in an item of `|> AGGREGATE`, over an aggregate.
    /// A table function stands after `FROM` and a pipe `CALL`, which are no places.
    pub fn takes(&self, function: &crate::library::Function) -> bool {
        use crate::library::{Family, FunctionKind, Over};
        if let Some(part) = match_recognize_part(&function.name) {
            return self.match_recognize == Some(part);
        }
        if self.kind == PlaceKind::ColumnList {
            return false;
        }
        match (function.family, function.kind) {
            (Family::Mark, _) => self.aggregates,
            (_, FunctionKind::Aggregate) => {
                self.aggregates || (self.windows && function.clauses.over != Over::Forbidden)
            }
            (_, FunctionKind::Window) => self.windows,
            (_, FunctionKind::Table) => false,
            // A `PIVOT` takes an aggregate's call itself. An item of `|> AGGREGATE` takes a scalar
            // function over an aggregate, so one with an argument that takes a value.
            (_, FunctionKind::Scalar) => match self.kind {
                PlaceKind::Pivot => false,
                PlaceKind::AggregateItem => (function.forms.iter())
                    .flat_map(|form| &form.signature.args)
                    .any(|arg| matches!(arg.accept, crate::library::Accept::Value(_))),
                _ => true,
            },
        }
    }

    /// Whether a column can stand here by itself. An item of `|> AGGREGATE` and of a `PIVOT`
    /// reads columns only in the arguments of its aggregates, which are places of their own.
    pub fn takes_columns(&self) -> bool {
        self.columns && !matches!(self.kind, PlaceKind::AggregateItem | PlaceKind::Pivot)
    }

    /// Whether a call can stand here: everywhere but a list of columns.
    pub fn takes_calls(&self) -> bool {
        self.kind != PlaceKind::ColumnList
    }
}

/// A string literal that a parameter takes as one value of a set, as `'normalize'` in
/// `stack(n, offset => 'normalize')`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumValue {
    /// The range of the literal.
    pub range: TextRange,
    /// The name of the parameter, as the library names it.
    pub param: String,
}

/// The scope of an operator: the columns of its input, which its expressions read, the range
/// variables in scope there, and its output columns.
#[derive(Clone, Debug, PartialEq)]
pub struct OperatorScope {
    /// The range of the operator's text.
    pub range: TextRange,
    pub input: Vec<Column>,
    pub range_variables: Vec<RangeVariable>,
    pub output: Vec<Column>,
    /// The names of the queries in scope: the pipe `WITH` names and the temporary tables.
    pub queries: Vec<String>,
    /// The columns of the enclosing queries that a subquery reads, innermost first.
    pub outer: Vec<Column>,
    /// The range variables of the enclosing queries that a subquery reads, innermost first.
    pub outer_range_variables: Vec<RangeVariable>,
}

/// A column of an operator's input or output.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    /// The name as its query writes it, or the empty string for a column without a name, which no
    /// text can refer to.
    pub name: String,
    /// The range variables that hold the column, by which a name such as `t.x` reads it.
    pub qualifiers: Vec<String>,
    /// The range of the text that defines the column: its alias, its expression, or the table
    /// that holds it.
    pub definition: Option<TextRange>,
    /// The type, when the pass knows it: an operator after an error has columns without types.
    pub ty: Option<DataType>,
}

/// A range variable: a name for the row of a source, such as `t` in `FROM penguins AS t`.
#[derive(Clone, Debug, PartialEq)]
pub struct RangeVariable {
    pub name: String,
    /// The range of the alias or the table name that defines it.
    pub definition: Option<TextRange>,
    pub columns: Vec<Column>,
}

/// A name of the text and its definition.
#[derive(Clone, Debug, PartialEq)]
pub struct NameUse {
    /// The range of the name.
    pub range: TextRange,
    pub definition: Definition,
}

/// What a name refers to.
#[derive(Clone, Debug, PartialEq)]
pub enum Definition {
    Column(Column),
    RangeVariable {
        name: String,
        definition: Option<TextRange>,
    },
    /// A field of a struct.
    Field {
        name: String,
        ty: Option<DataType>,
    },
    /// A table of the catalog, by its path.
    Table {
        names: Vec<String>,
    },
    /// A name of a pipe `WITH` or a temporary table, and the range of its definition.
    Query {
        name: String,
        definition: Option<TextRange>,
    },
    /// A script variable.
    Variable {
        name: String,
        definition: Option<TextRange>,
        ty: Option<DataType>,
    },
    /// A module of an `IMPORT MODULE`, by its path.
    Module {
        names: Vec<String>,
    },
    /// A function of the library, or a temporary function or a function of a module, and what
    /// a call of it computes.
    Function {
        name: String,
        role: FunctionRole,
        /// The range of the name that the `CREATE FUNCTION` of a temporary function writes. A
        /// function of the library or of a module has none in the script's text.
        definition: Option<TextRange>,
    },
}

/// The form of a resolved call or operator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallForm {
    /// The range of the call or of the operator's expression.
    pub range: TextRange,
    pub form: String,
    /// The index of the form of the library's function that the call resolved to, among
    /// [`crate::library::Function::forms`]. A temporary function and `lump`, which the pass binds
    /// itself, have none.
    pub index: Option<usize>,
}

impl Model {
    /// The name whose range holds `offset`.
    pub fn name_at(&self, offset: TextSize) -> Option<&NameUse> {
        self.names
            .iter()
            .find(|name| name.range.contains_inclusive(offset))
    }

    /// The innermost place whose range holds `offset`, the last of those of one range.
    pub fn place_at(&self, offset: TextSize) -> Option<&Place> {
        (self.places.iter().rev())
            .filter(|place| place.range.contains_inclusive(offset))
            .min_by_key(|place| place.range.len())
    }

    /// The type of the expression that ends at `offset`, before a `.`.
    pub fn field_base_at(&self, offset: TextSize) -> Option<&DataType> {
        (self.field_bases.iter())
            .rfind(|base| base.range.end() == offset)
            .map(|base| &base.ty)
    }

    /// The innermost operator whose range holds `offset`.
    pub fn operator_at(&self, offset: TextSize) -> Option<&OperatorScope> {
        self.operators
            .iter()
            .filter(|operator| operator.range.contains_inclusive(offset))
            .min_by_key(|operator| operator.range.len())
    }

    /// The form of the outermost call or operator whose range is `range`, as `$not` of
    /// `x IS NOT NULL`.
    pub fn call_at(&self, range: TextRange) -> Option<&str> {
        self.calls
            .iter()
            .rfind(|call| call.range == range)
            .map(|call| call.form.as_str())
    }
}
