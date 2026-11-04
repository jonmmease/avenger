//! The scalar functions: `selected`, which [`bind_selections`](crate::bind_selections) lowers,
//! the row test `selection_matches`, which runs avenger-selection's `selected` test on the columns
//! of each batch, and `pixel_cell` and `in_keys`, which compute and check the keys of a
//! pre-aggregation's table of cells.

use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
};

use avenger_selection::{Clause, PixelGrid, PreparedCondition, PreparedPredicate, SelectionState};
use datafusion::{
    arrow::{
        array::{AsArray, BooleanArray},
        buffer::BooleanBuffer,
        compute::and,
        datatypes::DataType,
    },
    common::{exec_err, plan_err, Result},
    logical_expr::{ColumnarValue, Expr, ScalarFunctionArgs, ScalarUDFImpl, Signature, Volatility},
};

/// The parameters of `selected`, in order. A missing trailing parameter and a `NULL` argument
/// mean the same: the parameter's default.
pub(crate) const SELECTED_PARAMETERS: [&str; 6] = [
    "state",
    "exclude_own",
    "owner",
    "partition",
    "empty",
    "without",
];

pub(crate) fn external(
    error: impl std::error::Error + Send + Sync + 'static,
) -> datafusion::error::DataFusionError {
    datafusion::error::DataFusionError::External(Box::new(error))
}

/// `selected(state, exclude_own, owner, partition, empty, without)`: whether each row is
/// selected. [`bind_selections`](crate::bind_selections) lowers each call on a bound placeholder
/// to a filter, so the function itself never runs.
#[derive(Debug, PartialEq, Eq, Hash)]
pub(crate) struct SelectedFunction {
    signature: Signature,
}

impl SelectedFunction {
    pub(crate) fn new() -> Self {
        let signature = Signature::user_defined(Volatility::Immutable)
            .with_parameter_names(SELECTED_PARAMETERS.to_vec())
            .expect("a user-defined signature takes parameter names");
        Self { signature }
    }
}

/// Whether `data_type` is a string type, or `Null`.
fn string_or_null(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::Null | DataType::Utf8 | DataType::Utf8View | DataType::LargeUtf8
    )
}

impl ScalarUDFImpl for SelectedFunction {
    fn name(&self) -> &str {
        "selected"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _: &[DataType]) -> Result<DataType> {
        Ok(DataType::Boolean)
    }

    fn coerce_types(&self, types: &[DataType]) -> Result<Vec<DataType>> {
        if types.is_empty() || types.len() > SELECTED_PARAMETERS.len() {
            return plan_err!(
                "`selected` takes from 1 to {} arguments ({}), not {}",
                SELECTED_PARAMETERS.len(),
                SELECTED_PARAMETERS.join(", "),
                types.len()
            );
        }
        for (index, data_type) in types.iter().enumerate() {
            let fits = match SELECTED_PARAMETERS[index] {
                // The state is a placeholder, which has no type before it is bound.
                "state" => *data_type == DataType::Null,
                "partition" => matches!(data_type, DataType::Null | DataType::Struct(_)),
                "exclude_own" => matches!(data_type, DataType::Null | DataType::Boolean),
                "owner" | "empty" => string_or_null(data_type),
                _ => match data_type {
                    DataType::List(item) | DataType::LargeList(item) => {
                        string_or_null(item.data_type())
                    }
                    other => *other == DataType::Null,
                },
            };
            if !fits {
                return plan_err!(
                    "the argument `{}` of `selected` has the type {data_type}",
                    SELECTED_PARAMETERS[index]
                );
            }
        }
        Ok(types.to_vec())
    }

    fn invoke_with_args(&self, _: ScalarFunctionArgs) -> Result<ColumnarValue> {
        exec_err!(
            "`selected` runs only through bind_selections: lower the plan with \
             avenger_selection_datafusion::bind_selections before optimizing it, then run it"
        )
    }
}

/// `selection_matches(columns..., partition)`: whether each row is selected, by a `selected` test
/// that the function carries prepared for the types of its columns, avenger-selection's
/// [`PreparedPredicate`]. The arguments are the columns that the test reads, in the order of
/// [`PreparedPredicate::columns`], and for a faceted owner's test, the struct of the row's
/// partition columns.
///
/// The lowering builds each call, so the plan holds no state, and each batch matches without
/// building the conditions' indexes again. A fingerprint of the test names the call and decides
/// its equality.
pub(crate) struct SelectionMatches {
    signature: Signature,
    fingerprint: u64,
    prepared: Arc<PreparedPredicate>,
    columns: Vec<String>,
    faceted: bool,
}

impl SelectionMatches {
    /// The function of the test `prepared`, which takes the struct of the row's partition columns
    /// after the test's columns when `faceted`.
    pub(crate) fn new(prepared: PreparedPredicate, faceted: bool) -> Self {
        Self {
            signature: Signature::user_defined(Volatility::Immutable),
            fingerprint: prepared.fingerprint(),
            columns: (prepared.columns().into_iter())
                .map(str::to_string)
                .collect(),
            prepared: Arc::new(prepared),
            faceted,
        }
    }
}

/// The name of a call of `function`, whose fingerprint is `fingerprint`, with `args`.
pub(crate) fn call_name(function: &str, fingerprint: u64, args: &[Expr]) -> String {
    let args: Vec<String> = (args.iter())
        .map(|arg| arg.schema_name().to_string())
        .collect();
    format!("{function}#{fingerprint:016x}({})", args.join(","))
}

impl std::fmt::Debug for SelectionMatches {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        (formatter.debug_struct("SelectionMatches"))
            .field("fingerprint", &self.fingerprint)
            .finish()
    }
}

impl PartialEq for SelectionMatches {
    fn eq(&self, other: &Self) -> bool {
        self.fingerprint == other.fingerprint
    }
}

impl Eq for SelectionMatches {}

impl std::hash::Hash for SelectionMatches {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.fingerprint.hash(state);
    }
}

impl ScalarUDFImpl for SelectionMatches {
    fn name(&self) -> &str {
        "selection_matches"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn schema_name(&self, args: &[Expr]) -> Result<String> {
        Ok(call_name(self.name(), self.fingerprint, args))
    }

    fn return_type(&self, _: &[DataType]) -> Result<DataType> {
        Ok(DataType::Boolean)
    }

    fn coerce_types(&self, types: &[DataType]) -> Result<Vec<DataType>> {
        Ok(types.to_vec())
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let rows = args.number_rows;
        let arrays = (args.args.iter())
            .map(|column| column.to_array(rows))
            .collect::<Result<Vec<_>>>()?;
        if arrays.len() != self.columns.len() + usize::from(self.faceted) {
            return exec_err!(
                "`selection_matches` takes a column for each column of its test, and a faceted \
                 test the struct of the row's partition columns"
            );
        }
        let column = |name: &str| {
            (self.columns.iter())
                .position(|column| column == name)
                .map(|index| arrays[index].clone())
        };
        let partition = (arrays.last()).filter(|_| self.faceted);
        let panel = |name: &str| {
            partition.and_then(|struct_| struct_.as_struct().column_by_name(name).cloned())
        };
        let mask = (self.prepared)
            .mask(rows, column, panel)
            .map_err(external)?;
        Ok(ColumnarValue::Array(Arc::new(mask)))
    }
}

/// `pixel_cell(value)`: the cell of each value in the grid that the function carries, or null for
/// a value without a cell. A fingerprint of the grid names the call and decides its equality.
pub(crate) struct PixelCell {
    signature: Signature,
    fingerprint: u64,
    grid: PixelGrid,
}

impl PixelCell {
    pub(crate) fn new(grid: PixelGrid) -> Self {
        Self {
            signature: Signature::user_defined(Volatility::Immutable),
            fingerprint: grid.fingerprint(),
            grid,
        }
    }
}

impl std::fmt::Debug for PixelCell {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        (formatter.debug_struct("PixelCell"))
            .field("fingerprint", &self.fingerprint)
            .finish()
    }
}

impl PartialEq for PixelCell {
    fn eq(&self, other: &Self) -> bool {
        self.fingerprint == other.fingerprint
    }
}

impl Eq for PixelCell {}

impl std::hash::Hash for PixelCell {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.fingerprint.hash(state);
    }
}

impl ScalarUDFImpl for PixelCell {
    fn name(&self) -> &str {
        "pixel_cell"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn schema_name(&self, args: &[Expr]) -> Result<String> {
        Ok(call_name(self.name(), self.fingerprint, args))
    }

    fn return_type(&self, _: &[DataType]) -> Result<DataType> {
        Ok(DataType::Int64)
    }

    fn coerce_types(&self, types: &[DataType]) -> Result<Vec<DataType>> {
        match types {
            [_] => Ok(types.to_vec()),
            _ => plan_err!("`pixel_cell` takes a value"),
        }
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let values = args.args[0].to_array(args.number_rows)?;
        let cells = self.grid.cells(&values).map_err(external)?;
        Ok(ColumnarValue::Array(Arc::new(cells)))
    }
}

/// `in_keys(state, owner, keys)`: whether each row of a table of cells matches every clause of
/// `owner` in the selection's state, or `TRUE` when the owner has none. `state` is a placeholder,
/// and `keys` is a struct whose field for each checked column holds the row's key: the cell of a
/// gridded range or of each column of a polygon with cells, or the value of a column of a set.
///
/// [`bind_selections`](crate::bind_selections) turns a call on a bound placeholder into
/// `in_keys(keys)` of a function that carries the conditions of the owner's clauses, prepared
/// once for the types of the keys, whose fingerprint names the call and decides its equality.
pub(crate) struct InKeys {
    signature: Signature,
    conditions: Option<(u64, Arc<Vec<PreparedCondition>>)>,
}

impl InKeys {
    pub(crate) fn new() -> Self {
        Self {
            signature: Signature::user_defined(Volatility::Immutable),
            conditions: None,
        }
    }

    /// The function of the clauses `clauses` of one owner, which takes only the keys, of the
    /// struct type `keys`: the conditions of the clauses, prepared for the types of their keys.
    pub(crate) fn of_clauses(clauses: &[Clause], keys: &DataType) -> Result<Self> {
        let DataType::Struct(fields) = keys else {
            return plan_err!("the keys of `in_keys` are a struct, not {keys}");
        };
        let key_type = |column: &str| match fields.find(column) {
            Some((_, field)) => Ok(field.data_type().clone()),
            None => plan_err!("`in_keys` has no key of the column {column}"),
        };
        let mut conditions = Vec::new();
        for condition in clauses.iter().flat_map(|clause| &clause.conditions) {
            let types = (condition.columns().into_iter())
                .map(key_type)
                .collect::<Result<Vec<_>>>()?;
            conditions.push(condition.prepare(&types).map_err(external)?);
        }
        Ok(Self {
            conditions: Some((fingerprint(clauses), Arc::new(conditions))),
            ..Self::new()
        })
    }

    /// Whether the function takes the state and the owner, rather than carrying the conditions.
    pub(crate) fn takes_state(&self) -> bool {
        self.conditions.is_none()
    }

    fn fingerprint(&self) -> Option<u64> {
        self.conditions
            .as_ref()
            .map(|(fingerprint, _)| *fingerprint)
    }
}

impl std::fmt::Debug for InKeys {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        (formatter.debug_struct("InKeys"))
            .field("fingerprint", &self.fingerprint())
            .finish()
    }
}

impl PartialEq for InKeys {
    fn eq(&self, other: &Self) -> bool {
        self.fingerprint() == other.fingerprint()
    }
}

impl Eq for InKeys {}

impl std::hash::Hash for InKeys {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.fingerprint().hash(state);
    }
}

impl ScalarUDFImpl for InKeys {
    fn name(&self) -> &str {
        "in_keys"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn schema_name(&self, args: &[Expr]) -> Result<String> {
        match self.fingerprint() {
            Some(fingerprint) => Ok(call_name(self.name(), fingerprint, args)),
            None => {
                let args: Vec<String> = (args.iter())
                    .map(|arg| arg.schema_name().to_string())
                    .collect();
                Ok(format!("{}({})", self.name(), args.join(",")))
            }
        }
    }

    fn return_type(&self, _: &[DataType]) -> Result<DataType> {
        Ok(DataType::Boolean)
    }

    fn coerce_types(&self, types: &[DataType]) -> Result<Vec<DataType>> {
        Ok(types.to_vec())
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let Some((_, conditions)) = &self.conditions else {
            return exec_err!(
                "`in_keys` runs only through bind_selections: bind the selection's state to the \
                 plan before running it"
            );
        };
        let rows = args.number_rows;
        let keys = args.args[0].to_array(rows)?;
        let keys = keys.as_struct();
        let mut mask = BooleanArray::new(BooleanBuffer::new_set(rows), None);
        for prepared in conditions.iter() {
            let columns = (prepared.condition().columns().into_iter())
                .map(|column| match keys.column_by_name(column) {
                    Some(key) => Ok(key.clone()),
                    None => exec_err!("`in_keys` has no key of the column {column}"),
                })
                .collect::<Result<Vec<_>>>()?;
            mask = and(&mask, &prepared.matches_keys(&columns).map_err(external)?)?;
        }
        Ok(ColumnarValue::Array(Arc::new(mask)))
    }
}

/// The clauses of `owner` in `state`, the owner of no name when it is `None`.
pub(crate) fn owners_clauses(state: &SelectionState, owner: Option<&str>) -> Vec<Clause> {
    let owner = owner.unwrap_or_default();
    (state.clauses.iter())
        .filter(|clause| clause.owner == owner)
        .cloned()
        .collect()
}

/// A fingerprint of `clauses`, from the fingerprint of each clause in order.
fn fingerprint(clauses: &[Clause]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for clause in clauses {
        clause.fingerprint().hash(&mut hasher);
    }
    hasher.finish()
}
