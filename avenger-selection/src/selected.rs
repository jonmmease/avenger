//! The test of a predicate over selections: the rows that the selections select for the options
//! of each `Selected`, prepared once for the types of the columns, as avenger-selection-datafusion's
//! `selected` filters rows and as a renderer styles them.

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
    condition::cast_to, hash::hash_array, Clause, Condition, Predicate, PreparedCondition,
    Resolution, Result, SelectionError, SelectionState,
};

/// The options of the test of one selection.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct SelectedOptions {
    /// The owners of the rows, such as the plot that draws them and the plots that hold it,
    /// whose clauses the test skips: all of them, or with `faceted`, the clause of each row's own
    /// panel. A cross filter gives the plot that draws the rows.
    pub exclude_own: Vec<String>,
    /// Whether each row gives its panel, by the partition columns of the owners' clauses, so a
    /// faceted owner's row skips only the clause of its own panel.
    pub faceted: bool,
    /// Whether a selection without clauses selects no row, rather than every row.
    pub empty_selects_none: bool,
    /// The owners whose clauses the test skips.
    pub without: Vec<String>,
}

/// The test of a predicate prepared for the types of its columns, which matches many batches of
/// rows: each condition's index is built once, when the test is prepared.
pub struct PreparedPredicate {
    conditions: Vec<PreparedCondition>,
    root: Node,
}

/// A test over rows, whose leaves index the prepared conditions.
enum Node {
    All,
    Nothing,
    Every(Vec<Node>),
    Any(Vec<Node>),
    Not(Box<Node>),
    Condition(usize),
    /// The test of the rows of each panel of a faceted owner, by the panel's partition values,
    /// in the order of the owner's clauses, and of the rows of no panel.
    Panels {
        panels: Vec<(Vec<(String, ArrayRef)>, Node)>,
        default: Box<Node>,
    },
}

impl Predicate {
    /// The test of the predicate over the states of the selections by name, prepared for the
    /// column types that `types` gives by name. A selection without a state has no clauses.
    /// Preparing fails for a condition of a column that `types` does not give, or a condition
    /// that does not prepare for its columns' types.
    ///
    /// A selection without clauses selects every row, or none with `empty_selects_none`. A
    /// selection whose clauses the test skips, all of them, selects every row. Otherwise a row
    /// matches an intersect selection when it matches the clause of every owner that the test
    /// keeps, and a global or a union selection when it matches the clause of one. A row
    /// matches a clause when it matches every condition of the clause.
    pub fn prepare(
        &self,
        states: &BTreeMap<String, SelectionState>,
        types: impl Fn(&str) -> Option<DataType>,
    ) -> Result<PreparedPredicate> {
        let mut builder = Builder {
            types: &types,
            conditions: Vec::new(),
            positions: HashMap::new(),
        };
        let empty = SelectionState::default();
        let root = builder.predicate(self, &|name| states.get(name).unwrap_or(&empty))?;
        Ok(PreparedPredicate {
            conditions: builder.conditions,
            root,
        })
    }

    /// Whether each row of `batch` matches the predicate over the states of the selections by
    /// name, as a mask without nulls, with the conditions' columns and the panels' partition
    /// columns by name in `batch`.
    pub fn mask(
        &self,
        states: &BTreeMap<String, SelectionState>,
        batch: &RecordBatch,
    ) -> Result<BooleanArray> {
        let schema = batch.schema();
        let types = |name: &str| (schema.field_with_name(name).ok()).map(|f| f.data_type().clone());
        let column = |name: &str| batch.column_by_name(name).cloned();
        self.prepare(states, types)?
            .mask(batch.num_rows(), column, column)
    }
}

impl SelectionState {
    /// The test of the state for `options`, prepared for the column types that `types` gives by
    /// name, as [`Predicate::prepare`] prepares a predicate of the one selection.
    pub fn prepare_selected(
        &self,
        options: &SelectedOptions,
        types: impl Fn(&str) -> Option<DataType>,
    ) -> Result<PreparedPredicate> {
        let mut builder = Builder {
            types: &types,
            conditions: Vec::new(),
            positions: HashMap::new(),
        };
        let root = builder.selected(self, options)?;
        Ok(PreparedPredicate {
            conditions: builder.conditions,
            root,
        })
    }

    /// Whether each row of `batch` is selected for `options`, as a mask without nulls, with the
    /// conditions' columns and the panels' partition columns by name in `batch`.
    pub fn mask(&self, options: &SelectedOptions, batch: &RecordBatch) -> Result<BooleanArray> {
        let schema = batch.schema();
        let types = |name: &str| (schema.field_with_name(name).ok()).map(|f| f.data_type().clone());
        let column = |name: &str| batch.column_by_name(name).cloned();
        self.prepare_selected(options, types)?
            .mask(batch.num_rows(), column, column)
    }
}

/// The prepared conditions of a test's tree, each condition of a clause prepared once.
struct Builder<'a, F> {
    types: &'a F,
    conditions: Vec<PreparedCondition>,
    positions: HashMap<*const Condition, usize>,
}

impl<F: Fn(&str) -> Option<DataType>> Builder<'_, F> {
    fn predicate<'s>(
        &mut self,
        predicate: &Predicate,
        state: &dyn Fn(&str) -> &'s SelectionState,
    ) -> Result<Node> {
        Ok(match predicate {
            Predicate::Selected(selected) => {
                self.selected(state(&selected.selection), &selected.options)?
            }
            Predicate::And(every) => Node::Every(self.predicates(every, state)?),
            Predicate::Or(any) => Node::Any(self.predicates(any, state)?),
            Predicate::Not(part) => Node::Not(Box::new(self.predicate(part, state)?)),
        })
    }

    fn predicates<'s>(
        &mut self,
        predicates: &[Predicate],
        state: &dyn Fn(&str) -> &'s SelectionState,
    ) -> Result<Vec<Node>> {
        (predicates.iter())
            .map(|predicate| self.predicate(predicate, state))
            .collect()
    }

    /// The test of `state` for `options`.
    fn selected(&mut self, state: &SelectionState, options: &SelectedOptions) -> Result<Node> {
        let without = |clause: &Clause| options.without.contains(&clause.owner);
        let own = |clause: &Clause| options.exclude_own.contains(&clause.owner);
        if !options.faceted || options.exclude_own.is_empty() {
            return self.clauses(state, options, &|clause| without(clause) || own(clause));
        }
        // A faceted owner wrote a clause for each of its panels, so a row skips only the clause of
        // its own panel: one test for each panel with a clause.
        let mut partitions: Vec<&BTreeMap<String, avenger_scales::scalar::Scalar>> = Vec::new();
        for clause in &state.clauses {
            if own(clause) && !without(clause) && !partitions.contains(&&clause.partition) {
                partitions.push(&clause.partition);
            }
        }
        let default = self.clauses(state, options, &without)?;
        if partitions.is_empty() {
            return Ok(default);
        }
        let mut panels = Vec::with_capacity(partitions.len());
        for partition in partitions {
            let skipped = |clause: &Clause| {
                without(clause) || (own(clause) && &clause.partition == partition)
            };
            let values = (partition.iter())
                .map(|(column, value)| (column.clone(), value.array().clone()))
                .collect();
            panels.push((values, self.clauses(state, options, &skipped)?));
        }
        Ok(Node::Panels {
            panels,
            default: Box::new(default),
        })
    }

    /// The test of the clauses of `state` that `skipped` does not skip.
    fn clauses(
        &mut self,
        state: &SelectionState,
        options: &SelectedOptions,
        skipped: &dyn Fn(&Clause) -> bool,
    ) -> Result<Node> {
        if state.clauses.is_empty() {
            return Ok(if options.empty_selects_none {
                Node::Nothing
            } else {
                Node::All
            });
        }
        let mut kept = Vec::new();
        for clause in state.clauses.iter().filter(|clause| !skipped(clause)) {
            let conditions = (clause.conditions.iter())
                .map(|condition| self.condition(condition))
                .collect::<Result<Vec<_>>>()?;
            kept.push(Node::Every(conditions));
        }
        Ok(match (kept.is_empty(), state.resolution) {
            (true, _) => Node::All,
            (false, Resolution::Intersect) => Node::Every(kept),
            (false, _) => Node::Any(kept),
        })
    }

    fn condition(&mut self, condition: &Condition) -> Result<Node> {
        let key = condition as *const Condition;
        if let Some(position) = self.positions.get(&key) {
            return Ok(Node::Condition(*position));
        }
        let types = (condition.columns().into_iter())
            .map(|name| (self.types)(name).ok_or_else(|| missing(name)))
            .collect::<Result<Vec<_>>>()?;
        self.conditions.push(condition.prepare(&types)?);
        self.positions.insert(key, self.conditions.len() - 1);
        Ok(Node::Condition(self.conditions.len() - 1))
    }
}

fn missing(name: &str) -> SelectionError {
    SelectionError::Condition(format!(
        "a selection tests the column `{name}`, which the rows do not have"
    ))
}

impl PreparedPredicate {
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
        match &self.root {
            Node::All => Some(true),
            Node::Nothing => Some(false),
            _ => None,
        }
    }

    /// Whether each of `rows` rows matches, as a mask without nulls. `columns` gives the columns
    /// that the conditions read by name, and `panel` gives a faceted owner's partition columns by
    /// name, which a row's panel compares with the panel's values as `IS NOT DISTINCT FROM` does.
    /// Each condition matches the rows once.
    pub fn mask(
        &self,
        rows: usize,
        columns: impl Fn(&str) -> Option<ArrayRef>,
        panel: impl Fn(&str) -> Option<ArrayRef>,
    ) -> Result<BooleanArray> {
        let mut masks: Vec<Option<BooleanBuffer>> = vec![None; self.conditions.len()];
        let mut eval = Eval {
            conditions: &self.conditions,
            rows,
            columns: &columns,
            panel: &panel,
            masks: &mut masks,
        };
        Ok(BooleanArray::new(eval.node(&self.root)?, None))
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
            Node::Panels { panels, default } => {
                panels.len().hash(hasher);
                for (values, node) in panels {
                    values.len().hash(hasher);
                    for (column, value) in values {
                        column.hash(hasher);
                        hash_array(value.as_ref(), hasher);
                    }
                    self.hash_node(node, hasher);
                }
                self.hash_node(default, hasher);
            }
        }
    }
}

/// Hashes the test's content, the tree of its conditions and panels, so tests that hash equally
/// select the same rows, up to collisions.
impl Hash for PreparedPredicate {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        self.hash_node(&self.root, hasher);
    }
}

/// The evaluation of a test's tree over one batch, with the mask of each condition, once.
struct Eval<'a, C, P> {
    conditions: &'a [PreparedCondition],
    rows: usize,
    columns: &'a C,
    panel: &'a P,
    masks: &'a mut [Option<BooleanBuffer>],
}

impl<C, P> Eval<'_, C, P>
where
    C: Fn(&str) -> Option<ArrayRef>,
    P: Fn(&str) -> Option<ArrayRef>,
{
    fn node(&mut self, node: &Node) -> Result<BooleanBuffer> {
        let rows = self.rows;
        Ok(match node {
            Node::All => BooleanBuffer::new_set(rows),
            Node::Nothing => BooleanBuffer::new_unset(rows),
            Node::Every(parts) => {
                let mut mask = BooleanBuffer::new_set(rows);
                for part in parts {
                    mask = &mask & &self.node(part)?;
                }
                mask
            }
            Node::Any(parts) => {
                let mut mask = BooleanBuffer::new_unset(rows);
                for part in parts {
                    mask = &mask | &self.node(part)?;
                }
                mask
            }
            Node::Not(part) => !&self.node(part)?,
            Node::Condition(index) => {
                if let Some(mask) = &self.masks[*index] {
                    return Ok(mask.clone());
                }
                let prepared = &self.conditions[*index];
                let arrays = (prepared.condition().columns().into_iter())
                    .map(|name| (self.columns)(name).ok_or_else(|| missing(name)))
                    .collect::<Result<Vec<_>>>()?;
                let mask = prepared.matches(&arrays)?.values().clone();
                self.masks[*index] = Some(mask.clone());
                mask
            }
            Node::Panels { panels, default } => {
                let mut mask = self.node(default)?;
                let mut claimed = BooleanBuffer::new_unset(rows);
                for (values, node) in panels {
                    let mut in_panel = BooleanBuffer::new_set(rows);
                    for (column, value) in values {
                        let partition = (self.panel)(column).ok_or_else(|| {
                            SelectionError::Condition(format!(
                                "a faceted selection reads the partition column `{column}`, \
                                 which the rows do not have"
                            ))
                        })?;
                        in_panel = &in_panel & &not_distinct_from(&partition, value)?;
                    }
                    let taken = &in_panel & &!&claimed;
                    let selected = self.node(node)?;
                    mask = &(&taken & &selected) | &(&!&taken & &mask);
                    claimed = &claimed | &in_panel;
                }
                mask
            }
        })
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
