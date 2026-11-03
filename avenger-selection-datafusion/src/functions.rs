//! The scalar functions: `selected`, which [`bind_selections`](crate::bind_selections) lowers,
//! and `selection_matches`, the row test that runs avenger-selection's `selected` test on the
//! columns of each batch.

use std::sync::Arc;

use avenger_selection::PreparedPredicate;
use datafusion::{
    arrow::{array::AsArray, datatypes::DataType},
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
