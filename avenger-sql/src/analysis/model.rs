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
    },
}

/// The form of a resolved call or operator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallForm {
    /// The range of the call or of the operator's expression.
    pub range: TextRange,
    pub form: String,
}

impl Model {
    /// The name whose range holds `offset`.
    pub fn name_at(&self, offset: TextSize) -> Option<&NameUse> {
        self.names
            .iter()
            .find(|name| name.range.contains_inclusive(offset))
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
