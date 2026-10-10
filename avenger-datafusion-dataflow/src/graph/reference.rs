use std::{
    cmp::Ordering,
    fmt,
    hash::{Hash, Hasher},
    sync::Arc,
};

use datafusion::arrow::datatypes::{DataType, Field, FieldRef};
use datafusion::{
    common::{plan_err, DFSchema, DFSchemaRef, Result},
    logical_expr::{expr::Placeholder, Expr, Extension, LogicalPlan, UserDefinedLogicalNodeCore},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum TableRef {
    Input(usize),
    Node(usize),
    Rows(usize),
    Asset(usize),
    Import(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScalarRef {
    Input(usize),
    Node(usize),
    BaseInput(usize),
    Import(usize),
}

#[derive(Clone, Debug)]
pub(crate) struct GraphRead {
    pub graph: u64,
    pub source: TableRef,
    pub schema: DFSchemaRef,
    pub label: Arc<str>,
    pub row_index: Option<String>,
}

impl GraphRead {
    pub fn indexed_schema(schema: &DFSchemaRef, name: &str) -> Result<DFSchemaRef> {
        if name.is_empty() || schema.fields().iter().any(|f| f.name() == name) {
            return plan_err!("Row index requires a nonempty name absent from the input schema");
        }
        let fields = schema
            .iter()
            .map(|(q, f)| (q.cloned(), f.clone()))
            .chain(std::iter::once((
                None,
                Arc::new(Field::new(name, DataType::UInt64, false)),
            )))
            .collect();
        Ok(Arc::new(DFSchema::new_with_metadata(
            fields,
            schema.metadata().clone(),
        )?))
    }

    pub fn matches_input_schema(&self, schema: &DFSchemaRef) -> bool {
        match &self.row_index {
            Some(name) => Self::indexed_schema(schema, name).is_ok_and(|s| s == self.schema),
            None => schema == &self.schema,
        }
    }

    pub fn plan(self) -> LogicalPlan {
        LogicalPlan::Extension(Extension {
            node: Arc::new(self),
        })
    }
}

// A reference's schema is fixed by its graph-local identity and optional row index.
impl PartialEq for GraphRead {
    fn eq(&self, other: &Self) -> bool {
        (self.graph, self.source, &self.row_index) == (other.graph, other.source, &other.row_index)
    }
}
impl Eq for GraphRead {}
impl PartialOrd for GraphRead {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some((self.graph, self.source, &self.row_index).cmp(&(
            other.graph,
            other.source,
            &other.row_index,
        )))
    }
}
impl Hash for GraphRead {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.graph, self.source, &self.row_index).hash(state);
    }
}

impl UserDefinedLogicalNodeCore for GraphRead {
    fn name(&self) -> &str {
        "GraphRead"
    }
    fn inputs(&self) -> Vec<&LogicalPlan> {
        vec![]
    }
    fn schema(&self) -> &DFSchemaRef {
        &self.schema
    }
    fn expressions(&self) -> Vec<Expr> {
        vec![]
    }
    fn fmt_for_explain(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "GraphRead: {}", self.label)?;
        if let Some(name) = &self.row_index {
            write!(f, " row_index={name}")?;
        }
        Ok(())
    }
    fn with_exprs_and_inputs(&self, exprs: Vec<Expr>, inputs: Vec<LogicalPlan>) -> Result<Self> {
        if !exprs.is_empty() || !inputs.is_empty() {
            return datafusion::common::internal_err!("GraphRead is a leaf without expressions");
        }
        Ok(self.clone())
    }
}

pub(crate) fn placeholder_id(graph: u64, source: ScalarRef) -> String {
    match source {
        ScalarRef::Input(index) => format!("$__avenger_{graph}_input_{index}"),
        ScalarRef::Node(index) => format!("$__avenger_{graph}_node_{index}"),
        ScalarRef::BaseInput(index) => format!("$__avenger_{graph}_base_input_{index}"),
        ScalarRef::Import(index) => format!("$__avenger_{graph}_import_{index}"),
    }
}

pub(crate) fn scalar_ref(graph: u64, source: ScalarRef, field: FieldRef) -> Expr {
    Expr::Placeholder(Placeholder::new_with_field(
        placeholder_id(graph, source),
        Some(field),
    ))
}
