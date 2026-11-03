//! The `selected` test of a selection's state: the rows that the selection selects for the
//! options of a call, as avenger-selection-datafusion's `selected` filters them.

use std::{
    collections::{BTreeMap, HashMap},
    hash::{Hash, Hasher},
};

use arrow::{
    array::{Array, ArrayRef, AsArray, BooleanArray, RecordBatch, Scalar},
    buffer::BooleanBuffer,
    compute::kernels::cmp::not_distinct,
    datatypes::DataType,
};

use crate::{
    condition::cast_to, hash::hash_array, matching, Clause, Condition, Matching, Predicate,
    PreparedCondition, Result, Selected, SelectionError, SelectionState,
};

/// The options of a `selected` test, after the selection's state.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct SelectedOptions {
    /// The owner of the rows, whose clauses the test skips: all of them, or with `faceted`, the
    /// clause of each row's own panel. A cross filter gives the plot that draws the rows.
    pub exclude_own: Option<String>,
    /// Whether each row gives its panel, by the partition columns of the owner's clauses, so a
    /// faceted owner's row skips only the clause of its own panel.
    pub faceted: bool,
    /// Whether a selection without clauses selects no row, rather than every row.
    pub empty_selects_none: bool,
    /// The owners whose clauses the test skips.
    pub without: Vec<String>,
}

/// A `selected` test prepared for the types of its columns, which matches many batches of rows:
/// each condition's index is built once, when the test is prepared.
pub struct PreparedSelected {
    conditions: Vec<PreparedCondition>,
    /// The test of the rows of each panel of a faceted owner, by the panel's partition values,
    /// in the order of the owner's clauses. A row of no panel takes `default`.
    panels: Vec<(Vec<(String, ArrayRef)>, Node)>,
    default: Node,
}

/// A test over rows, whose leaves index the prepared conditions.
enum Node {
    All,
    Nothing,
    Every(Vec<Node>),
    Any(Vec<Node>),
    Not(Box<Node>),
    Condition(usize),
}

impl SelectionState {
    /// The `selected` test of the state for `options`, prepared for the column types that
    /// `types` gives by name. Preparing fails for a condition of a column that `types` does not
    /// give, or a condition that does not prepare for its columns' types.
    ///
    /// A selection without clauses selects every row, or none with `empty_selects_none`. A
    /// selection whose clauses the test skips, all of them, selects every row. Otherwise a row
    /// matches an intersect selection when it matches the clause of every owner that the test
    /// keeps, and a global or a union selection when it matches the clause of one.
    pub fn prepare_selected(
        &self,
        options: &SelectedOptions,
        types: impl Fn(&str) -> Option<DataType>,
    ) -> Result<PreparedSelected> {
        let without = |clause: &Clause| options.without.contains(&clause.owner);
        let own = |clause: &Clause| options.exclude_own.as_deref() == Some(clause.owner.as_str());
        let mut builder = Builder {
            types: &types,
            conditions: Vec::new(),
            positions: HashMap::new(),
        };
        let name = String::new();
        let clauses = BTreeMap::from([(name.clone(), self.clauses.clone())]);
        let resolutions = BTreeMap::from([(name.clone(), self.resolution)]);
        let predicate = Predicate::Selected(Selected {
            selection: name,
            empty_selects_none: options.empty_selects_none,
            exclude_own: true,
        });
        let mut tree = |skipped: &dyn Fn(&Clause) -> bool| -> Result<Node> {
            builder.node(&matching(&predicate, &resolutions, &clauses, skipped))
        };
        if !(options.faceted && options.exclude_own.is_some()) {
            let default = tree(&|clause| without(clause) || own(clause))?;
            return Ok(builder.finish(Vec::new(), default));
        }
        // A faceted owner wrote a clause for each of its panels, so a row skips only the clause of
        // its own panel: one test for each panel with a clause.
        let mut partitions: Vec<&BTreeMap<String, avenger_scales::scalar::Scalar>> = Vec::new();
        for clause in &self.clauses {
            if own(clause) && !without(clause) && !partitions.contains(&&clause.partition) {
                partitions.push(&clause.partition);
            }
        }
        let default = tree(&without)?;
        let mut panels = Vec::with_capacity(partitions.len());
        for partition in partitions {
            let node =
                tree(&|clause| without(clause) || (own(clause) && &clause.partition == partition))?;
            let values = (partition.iter())
                .map(|(column, value)| (column.clone(), value.array().clone()))
                .collect();
            panels.push((values, node));
        }
        Ok(builder.finish(panels, default))
    }

    /// Whether each row of `batch` is selected for `options`, as a mask without nulls: the test
    /// of [`prepare_selected`](Self::prepare_selected), with the conditions' columns and the
    /// panels' partition columns by name in `batch`.
    pub fn mask(&self, options: &SelectedOptions, batch: &RecordBatch) -> Result<BooleanArray> {
        let schema = batch.schema();
        let types = |name: &str| (schema.field_with_name(name).ok()).map(|f| f.data_type().clone());
        let column = |name: &str| batch.column_by_name(name).cloned();
        self.prepare_selected(options, types)?
            .mask(batch.num_rows(), column, column)
    }
}

/// The prepared conditions of the trees of a test, each condition of a clause prepared once.
struct Builder<'a, F> {
    types: &'a F,
    conditions: Vec<PreparedCondition>,
    positions: HashMap<*const Condition, usize>,
}

impl<F: Fn(&str) -> Option<DataType>> Builder<'_, F> {
    fn node(&mut self, matching: &Matching) -> Result<Node> {
        let mut parts = |parts: &[Matching]| -> Result<Vec<Node>> {
            parts.iter().map(|part| self.node(part)).collect()
        };
        Ok(match matching {
            Matching::All => Node::All,
            Matching::Nothing => Node::Nothing,
            Matching::Every(every) => Node::Every(parts(every)?),
            Matching::Any(any) => Node::Any(parts(any)?),
            Matching::Not(part) => Node::Not(Box::new(self.node(part)?)),
            Matching::Condition { condition, .. } => {
                let key = *condition as *const Condition;
                if let Some(position) = self.positions.get(&key) {
                    return Ok(Node::Condition(*position));
                }
                let types = (condition.columns().into_iter())
                    .map(|name| {
                        (self.types)(name).ok_or_else(|| {
                            SelectionError::Condition(format!(
                                "a selection tests the column `{name}`, which the rows do not have"
                            ))
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                self.conditions.push(condition.prepare(&types)?);
                self.positions.insert(key, self.conditions.len() - 1);
                Node::Condition(self.conditions.len() - 1)
            }
        })
    }

    fn finish(
        self,
        panels: Vec<(Vec<(String, ArrayRef)>, Node)>,
        default: Node,
    ) -> PreparedSelected {
        PreparedSelected {
            conditions: self.conditions,
            panels,
            default,
        }
    }
}

impl PreparedSelected {
    /// The columns that the test's conditions read, each once, in the order of the conditions.
    pub fn columns(&self) -> Vec<&str> {
        let mut columns: Vec<&str> = Vec::new();
        for condition in &self.conditions {
            for column in condition.condition().columns() {
                if !columns.contains(&column) {
                    columns.push(column);
                }
            }
        }
        columns
    }

    /// The answer for every row, when the test gives every row the same answer without reading
    /// it, such as for an empty selection.
    pub fn constant(&self) -> Option<bool> {
        match (&self.default, self.panels.is_empty()) {
            (Node::All, true) => Some(true),
            (Node::Nothing, true) => Some(false),
            _ => None,
        }
    }

    /// Whether each of `rows` rows is selected, as a mask without nulls. `columns` gives the
    /// columns that the conditions read by name, and `panel` gives a faceted owner's partition
    /// columns by name, which a row's panel compares with the panel's values as `IS NOT DISTINCT
    /// FROM` does. Each condition matches the rows once.
    pub fn mask(
        &self,
        rows: usize,
        columns: impl Fn(&str) -> Option<ArrayRef>,
        panel: impl Fn(&str) -> Option<ArrayRef>,
    ) -> Result<BooleanArray> {
        let mut masks: Vec<Option<BooleanBuffer>> = vec![None; self.conditions.len()];
        let mut mask = self.eval(&self.default, rows, &columns, &mut masks)?;
        let mut claimed = BooleanBuffer::new_unset(rows);
        for (values, node) in &self.panels {
            let mut in_panel = BooleanBuffer::new_set(rows);
            for (column, value) in values {
                let Some(partition) = panel(column) else {
                    return Err(SelectionError::Condition(format!(
                        "a faceted selection reads the partition column `{column}`, which the \
                         rows do not have"
                    )));
                };
                in_panel = &in_panel & &not_distinct_from(&partition, value)?;
            }
            let taken = &in_panel & &!&claimed;
            let selected = self.eval(node, rows, &columns, &mut masks)?;
            mask = &(&taken & &selected) | &(&!&taken & &mask);
            claimed = &claimed | &in_panel;
        }
        Ok(BooleanArray::new(mask, None))
    }

    fn eval(
        &self,
        node: &Node,
        rows: usize,
        columns: &impl Fn(&str) -> Option<ArrayRef>,
        masks: &mut [Option<BooleanBuffer>],
    ) -> Result<BooleanBuffer> {
        Ok(match node {
            Node::All => BooleanBuffer::new_set(rows),
            Node::Nothing => BooleanBuffer::new_unset(rows),
            Node::Every(parts) => {
                let mut mask = BooleanBuffer::new_set(rows);
                for part in parts {
                    mask = &mask & &self.eval(part, rows, columns, masks)?;
                }
                mask
            }
            Node::Any(parts) => {
                let mut mask = BooleanBuffer::new_unset(rows);
                for part in parts {
                    mask = &mask | &self.eval(part, rows, columns, masks)?;
                }
                mask
            }
            Node::Not(part) => !&self.eval(part, rows, columns, masks)?,
            Node::Condition(index) => {
                if let Some(mask) = &masks[*index] {
                    return Ok(mask.clone());
                }
                let prepared = &self.conditions[*index];
                let arrays = (prepared.condition().columns().into_iter())
                    .map(|name| {
                        columns(name).ok_or_else(|| {
                            SelectionError::Condition(format!(
                                "a selection tests the column `{name}`, which the rows do not have"
                            ))
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                let mask = prepared.matches(&arrays)?.values().clone();
                masks[*index] = Some(mask.clone());
                mask
            }
        })
    }

    fn hash_node<H: Hasher>(&self, node: &Node, hasher: &mut H) {
        std::mem::discriminant(node).hash(hasher);
        match node {
            Node::All | Node::Nothing => {}
            Node::Every(parts) | Node::Any(parts) => {
                parts.len().hash(hasher);
                for part in parts {
                    self.hash_node(part, hasher);
                }
            }
            Node::Not(part) => self.hash_node(part, hasher),
            Node::Condition(index) => self.conditions[*index].condition().hash(hasher),
        }
    }
}

/// Hashes the test's content: its panels' values and the trees of its conditions, so tests that
/// hash equally select the same rows, up to collisions.
impl Hash for PreparedSelected {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        self.panels.len().hash(hasher);
        for (values, node) in &self.panels {
            values.len().hash(hasher);
            for (column, value) in values {
                column.hash(hasher);
                hash_array(value.as_ref(), hasher);
            }
            self.hash_node(node, hasher);
        }
        self.hash_node(&self.default, hasher);
    }
}

/// Whether each value of `column` is not distinct from the one value of `value`, as `IS NOT
/// DISTINCT FROM` compares them, with `value` cast to the column's type. A dictionary column
/// compares its dictionary's values once.
fn not_distinct_from(column: &ArrayRef, value: &ArrayRef) -> Result<BooleanBuffer> {
    let DataType::Dictionary(_, _) = column.data_type() else {
        let value = cast_to(value, column.data_type())?;
        return Ok(not_distinct(column, &Scalar::new(value))?.values().clone());
    };
    let dictionary = column.as_any_dictionary();
    let found = not_distinct_from(dictionary.values(), value)?;
    let keys = dictionary.normalized_keys();
    let (key_nulls, null) = (dictionary.keys().logical_nulls(), value.is_null(0));
    Ok(BooleanBuffer::collect_bool(
        keys.len(),
        |row| match &key_nulls {
            Some(nulls) if nulls.is_null(row) => null,
            _ => found.value(keys[row]),
        },
    ))
}
