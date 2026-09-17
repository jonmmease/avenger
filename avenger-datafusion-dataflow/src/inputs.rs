use std::{collections::HashMap, sync::Arc};

use datafusion::{common::ScalarValue, logical_expr::Expr};

use crate::{
    graph::{reference::TableRef, BindingMetadata, InputKind},
    Error, ExprInput, Result, ScalarInput, ScopeHandle, ScopeInstance, TableInput, TableSnapshot,
};

#[derive(Clone, Debug)]
pub(crate) enum MaterializedValue {
    Table(TableSnapshot),
    Scalar(ScalarValue),
}

impl MaterializedValue {
    pub(crate) fn size(&self) -> usize {
        match self {
            Self::Table(t) => {
                t.batches()
                    .iter()
                    .map(|b| {
                        b.get_array_memory_size()
                            + std::mem::size_of::<datafusion::arrow::record_batch::RecordBatch>()
                    })
                    .sum::<usize>()
                    + std::mem::size_of::<TableSnapshot>()
            }
            Self::Scalar(v) => v.size(),
        }
    }
    pub(crate) fn key(&self) -> Result<crate::cache::BindingKey> {
        Ok(match self {
            Self::Table(t) => crate::cache::BindingKey::Table(t.id()),
            Self::Scalar(v) => {
                use datafusion::arrow::{
                    datatypes::{Field, Schema},
                    ipc::writer::StreamWriter,
                    record_batch::RecordBatch,
                };
                let schema = Arc::new(Schema::new(vec![Field::new("value", v.data_type(), true)]));
                let batch = RecordBatch::try_new(schema.clone(), vec![v.to_array()?])?;
                let mut bytes = Vec::new();
                let mut writer = StreamWriter::try_new(&mut bytes, &schema)?;
                writer.write(&batch)?;
                writer.finish()?;
                crate::cache::BindingKey::Scalar(bytes.into())
            }
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) enum InputBinding {
    Value(MaterializedValue),
    Expr(Arc<crate::expr_input::BoundExpr>),
}
impl InputBinding {
    pub(crate) fn key(&self) -> Result<crate::cache::BindingKey> {
        match self {
            Self::Value(value) => value.key(),
            Self::Expr(expr) => Ok(crate::cache::BindingKey::Expr(expr.key.clone())),
        }
    }
    pub(crate) fn frame_size(&self) -> usize {
        match self {
            Self::Value(MaterializedValue::Table(_)) => 0,
            Self::Value(MaterializedValue::Scalar(value)) => value.size(),
            Self::Expr(expr) => expr.size(),
        }
    }
}

type Bindings = HashMap<usize, InputBinding>;
type Defaults = HashMap<usize, Arc<Bindings>>;
type Overrides = HashMap<ScopeInstance, Arc<Bindings>>;

/// Immutable root bindings, scope defaults, and sparse instance overrides.
/// Store replacement does not modify captured snapshots.
#[derive(Clone, Debug)]
pub struct Inputs {
    pub(crate) graph: Arc<BindingMetadata>,
    pub(crate) values: Arc<[Option<InputBinding>]>,
    defaults: Defaults,
    overrides: Overrides,
}

impl Inputs {
    /// Edit individual entries while preserving other defaults and overrides.
    pub fn edit(&self) -> InputsBuilder {
        InputsBuilder {
            graph: self.graph.clone(),
            values: self.values.to_vec(),
            defaults: self.defaults.clone(),
            overrides: self.overrides.clone(),
        }
    }

    pub(crate) fn resolve(
        &self,
        index: usize,
        instance: Option<&ScopeInstance>,
    ) -> Option<&InputBinding> {
        let scope = self.graph.inputs[index].scope;
        if scope == 0 {
            return self.values[index].as_ref();
        }
        instance
            .and_then(|address| self.overrides.get(address))
            .and_then(|bindings| bindings.get(&index))
            .or_else(|| {
                self.defaults
                    .get(&scope)
                    .and_then(|bindings| bindings.get(&index))
            })
    }
}

/// Builds one immutable binding set with complete root inputs.
#[derive(Debug)]
pub struct InputsBuilder {
    pub(crate) graph: Arc<BindingMetadata>,
    pub(crate) values: Vec<Option<InputBinding>>,
    defaults: Defaults,
    overrides: Overrides,
}

impl InputsBuilder {
    pub(crate) fn new(graph: Arc<BindingMetadata>) -> Self {
        Self {
            values: vec![None; graph.inputs.len()],
            graph,
            defaults: HashMap::new(),
            overrides: HashMap::new(),
        }
    }

    /// Set a root table binding with an exact schema.
    pub fn table(mut self, input: &TableInput, value: TableSnapshot) -> Result<Self> {
        let index = table_index(&self.graph, 0, input)?;
        validate_table(&self.graph, index, &value)?;
        self.values[index] = Some(InputBinding::Value(MaterializedValue::Table(value)));
        Ok(self)
    }

    /// Set a root scalar binding with an exact type.
    pub fn scalar(mut self, input: &ScalarInput, value: ScalarValue) -> Result<Self> {
        let index = scalar_index(&self.graph, 0, input)?;
        validate_scalar(&self.graph, index, &value)?;
        self.values[index] = Some(InputBinding::Value(MaterializedValue::Scalar(value)));
        Ok(self)
    }

    /// Bind a root expression after checking every declared usage context.
    pub fn expr(mut self, input: &ExprInput, value: Expr) -> Result<Self> {
        let index = expr_index(&self.graph, 0, input)?;
        self.values[index] = Some(bind_expr(&self.graph, index, value)?);
        Ok(self)
    }

    /// Edit defaults for inputs declared directly in this definition.
    pub fn scope_defaults(
        mut self,
        scope: &ScopeHandle,
        build: impl FnOnce(ScopedBindingsBuilder) -> Result<ScopedBindingsBuilder>,
    ) -> Result<Self> {
        if scope.graph != self.graph.id {
            return Err(Error::ForeignHandle);
        }
        let values = self
            .defaults
            .get(&scope.index)
            .map(|v| v.as_ref().clone())
            .unwrap_or_default();
        let bindings = build(ScopedBindingsBuilder {
            graph: self.graph.clone(),
            scope: scope.index,
            values,
        })?;
        bindings.check_target(self.graph.id, scope.index)?;
        self.defaults.insert(scope.index, Arc::new(bindings.values));
        Ok(self)
    }

    /// Edit sparse overrides for an address, including an instance absent from current data.
    pub fn at(
        mut self,
        instance: &ScopeInstance,
        build: impl FnOnce(ScopedBindingsBuilder) -> Result<ScopedBindingsBuilder>,
    ) -> Result<Self> {
        if instance.graph != self.graph.id {
            return Err(Error::ForeignHandle);
        }
        let scope = instance.scope();
        let values = self
            .overrides
            .get(instance)
            .map(|v| v.as_ref().clone())
            .unwrap_or_default();
        let bindings = build(ScopedBindingsBuilder {
            graph: self.graph.clone(),
            scope,
            values,
        })?;
        bindings.check_target(self.graph.id, scope)?;
        self.overrides
            .insert(instance.clone(), Arc::new(bindings.values));
        Ok(self)
    }

    /// Require all root inputs. Scoped completeness is checked for demanded instances during query.
    pub fn finish(self) -> Result<Inputs> {
        for (index, input) in self.graph.inputs.iter().enumerate() {
            if input.scope == 0 && self.values[index].is_none() {
                return Err(Error::MissingInput(input.name.to_string()));
            }
        }
        Ok(Inputs {
            graph: self.graph,
            values: self.values.into(),
            defaults: self.defaults,
            overrides: self.overrides,
        })
    }
}

/// Edits one definition-default or exact-instance entry, without copying inherited values.
#[derive(Debug)]
pub struct ScopedBindingsBuilder {
    graph: Arc<BindingMetadata>,
    scope: usize,
    values: Bindings,
}

impl ScopedBindingsBuilder {
    fn check_target(&self, graph: u64, scope: usize) -> Result<()> {
        if self.graph.id != graph {
            return Err(Error::ForeignHandle);
        }
        if self.scope != scope {
            return Err(Error::OutOfScope(
                "binding callback returned a different scope".into(),
            ));
        }
        Ok(())
    }

    /// Set a directly owned table input. An empty table is an explicit value.
    pub fn table(mut self, input: &TableInput, value: TableSnapshot) -> Result<Self> {
        let index = table_index(&self.graph, self.scope, input)?;
        validate_table(&self.graph, index, &value)?;
        self.values
            .insert(index, InputBinding::Value(MaterializedValue::Table(value)));
        Ok(self)
    }

    /// Set a directly owned scalar input. A typed null is an explicit value.
    pub fn scalar(mut self, input: &ScalarInput, value: ScalarValue) -> Result<Self> {
        let index = scalar_index(&self.graph, self.scope, input)?;
        validate_scalar(&self.graph, index, &value)?;
        self.values
            .insert(index, InputBinding::Value(MaterializedValue::Scalar(value)));
        Ok(self)
    }

    /// Bind an expression declared directly in this scope.
    pub fn expr(mut self, input: &ExprInput, value: Expr) -> Result<Self> {
        let index = expr_index(&self.graph, self.scope, input)?;
        self.values
            .insert(index, bind_expr(&self.graph, index, value)?);
        Ok(self)
    }

    /// Remove this entry, restoring default inheritance for an instance override.
    pub fn unset_expr(mut self, input: &ExprInput) -> Result<Self> {
        let index = expr_index(&self.graph, self.scope, input)?;
        self.values.remove(&index);
        Ok(self)
    }

    /// Remove this entry only. Removing an override restores default inheritance.
    pub fn unset_table(mut self, input: &TableInput) -> Result<Self> {
        let index = table_index(&self.graph, self.scope, input)?;
        self.values.remove(&index);
        Ok(self)
    }

    /// Remove this entry only, succeeding when it is already absent.
    pub fn unset_scalar(mut self, input: &ScalarInput) -> Result<Self> {
        let index = scalar_index(&self.graph, self.scope, input)?;
        self.values.remove(&index);
        Ok(self)
    }
}

fn check_owner(graph: &BindingMetadata, scope: usize, index: usize) -> Result<()> {
    if graph.inputs[index].scope != scope {
        return Err(Error::OutOfScope(graph.inputs[index].name.to_string()));
    }
    Ok(())
}
fn table_index(graph: &BindingMetadata, scope: usize, input: &TableInput) -> Result<usize> {
    if input.read.graph != graph.id {
        return Err(Error::ForeignHandle);
    }
    let TableRef::Input(index) = input.read.source else {
        unreachable!()
    };
    check_owner(graph, scope, index)?;
    Ok(index)
}
fn scalar_index(graph: &BindingMetadata, scope: usize, input: &ScalarInput) -> Result<usize> {
    if input.graph != graph.id {
        return Err(Error::ForeignHandle);
    }
    check_owner(graph, scope, input.index)?;
    Ok(input.index)
}
fn validate_table(graph: &BindingMetadata, index: usize, value: &TableSnapshot) -> Result<()> {
    let InputKind::Table(schema) = &graph.inputs[index].kind else {
        unreachable!()
    };
    if schema.as_arrow() != value.schema().as_ref() {
        return Err(Error::SchemaMismatch(graph.inputs[index].name.to_string()));
    }
    Ok(())
}
fn validate_scalar(graph: &BindingMetadata, index: usize, value: &ScalarValue) -> Result<()> {
    let InputKind::Scalar(field) = &graph.inputs[index].kind else {
        unreachable!()
    };
    if field.data_type() != &value.data_type() {
        return Err(Error::ScalarTypeMismatch {
            name: graph.inputs[index].name.to_string(),
            expected: field.data_type().to_string(),
            actual: value.data_type().to_string(),
        });
    }
    Ok(())
}

fn expr_index(graph: &BindingMetadata, scope: usize, input: &ExprInput) -> Result<usize> {
    if input.graph != graph.id {
        return Err(Error::ForeignHandle);
    }
    check_owner(graph, scope, input.index)?;
    Ok(input.index)
}
fn bind_expr(graph: &BindingMetadata, index: usize, value: Expr) -> Result<InputBinding> {
    let InputKind::Expr(field) = &graph.inputs[index].kind else {
        unreachable!()
    };
    Ok(InputBinding::Expr(Arc::new(
        crate::expr_input::BoundExpr::new(
            value,
            &graph.expr_sites[index],
            field,
            &graph.inputs[index].name,
        )?,
    )))
}
