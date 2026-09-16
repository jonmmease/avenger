use std::sync::Arc;

use datafusion::common::ScalarValue;

use crate::{
    graph::{reference::TableRef, GraphDef, InputKind},
    Error, Result, ScalarInput, TableInput, TableSnapshot,
};

#[derive(Clone, Debug)]
pub(crate) enum InputValue {
    Table(TableSnapshot),
    Scalar(ScalarValue),
}

/// A complete immutable binding set. Store changes do not affect existing inputs.
#[derive(Clone, Debug)]
pub struct Inputs {
    pub(crate) graph: Arc<GraphDef>,
    pub(crate) values: Arc<[InputValue]>,
}

impl Inputs {
    pub fn edit(&self) -> InputsBuilder {
        InputsBuilder {
            graph: self.graph.clone(),
            values: self.values.iter().cloned().map(Some).collect(),
        }
    }
}

#[derive(Debug)]
pub struct InputsBuilder {
    pub(crate) graph: Arc<GraphDef>,
    pub(crate) values: Vec<Option<InputValue>>,
}

impl InputsBuilder {
    pub fn table(mut self, input: &TableInput, value: TableSnapshot) -> Result<Self> {
        if input.read.graph != self.graph.id {
            return Err(Error::ForeignHandle);
        }
        let TableRef::Input(index) = input.read.source else {
            unreachable!()
        };
        let InputKind::Table(schema) = &self.graph.inputs[index].kind else {
            unreachable!()
        };
        if schema.as_arrow() != value.schema().as_ref() {
            return Err(Error::SchemaMismatch(input.name().into()));
        }
        self.values[index] = Some(InputValue::Table(value));
        Ok(self)
    }

    pub fn scalar(mut self, input: &ScalarInput, value: ScalarValue) -> Result<Self> {
        if input.graph != self.graph.id {
            return Err(Error::ForeignHandle);
        }
        let InputKind::Scalar(field) = &self.graph.inputs[input.index].kind else {
            unreachable!()
        };
        if field.data_type() != &value.data_type() {
            return Err(Error::ScalarTypeMismatch {
                name: input.name().into(),
                expected: field.data_type().to_string(),
                actual: value.data_type().to_string(),
            });
        }
        self.values[input.index] = Some(InputValue::Scalar(value));
        Ok(self)
    }

    pub fn finish(self) -> Result<Inputs> {
        let values = self
            .values
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                value.ok_or_else(|| Error::MissingInput(self.graph.inputs[index].name.to_string()))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Inputs {
            graph: self.graph,
            values: values.into(),
        })
    }
}
