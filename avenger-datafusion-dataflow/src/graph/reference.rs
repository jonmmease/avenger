use std::{
    cmp::Ordering,
    fmt,
    hash::{Hash, Hasher},
    sync::Arc,
};

use datafusion::arrow::datatypes::FieldRef;
use datafusion::{
    common::{DFSchemaRef, Result},
    logical_expr::{expr::Placeholder, Expr, Extension, LogicalPlan, UserDefinedLogicalNodeCore},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum TableRef {
    Input(usize),
    Node(usize),
    Rows(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScalarRef {
    Input(usize),
    Node(usize),
}

#[derive(Clone, Debug)]
pub(crate) struct GraphRead {
    pub graph: u64,
    pub source: TableRef,
    pub schema: DFSchemaRef,
    pub label: Arc<str>,
}

impl GraphRead {
    pub fn plan(self) -> LogicalPlan {
        LogicalPlan::Extension(Extension {
            node: Arc::new(self),
        })
    }
}

// A reference's schema and label are fixed by its graph-local identity.
impl PartialEq for GraphRead {
    fn eq(&self, other: &Self) -> bool {
        (self.graph, self.source) == (other.graph, other.source)
    }
}
impl Eq for GraphRead {}
impl PartialOrd for GraphRead {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some((self.graph, self.source).cmp(&(other.graph, other.source)))
    }
}
impl Hash for GraphRead {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.graph, self.source).hash(state);
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
        write!(f, "GraphRead: {}", self.label)
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
    }
}

pub(crate) fn scalar_ref(graph: u64, source: ScalarRef, field: FieldRef) -> Expr {
    Expr::Placeholder(Placeholder::new_with_field(
        placeholder_id(graph, source),
        Some(field),
    ))
}
