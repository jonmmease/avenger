//! A change of a chart by its reader as bytes: a `ChartChange` in a `Fragment`, with the values
//! that its references refer to, which a page sends to its server and a server sends back.

use arrow::array::{ArrayRef, AsArray};
use arrow::datatypes::DataType;
use prost::{bytes::Bytes, Message, Name};

use crate::{codec::values_to_ipc, proto, validate, ChartError, FORMAT_VERSION};

/// A change of a chart by its reader: a [`proto::ChartChange`] and the values that its `ValueRef`
/// fields refer to, checked against the rules of the format. Its bytes are a
/// [`proto::Fragment`] of the change, so its domains and its clauses keep the Arrow types of their
/// columns.
#[derive(Clone, Debug)]
pub struct ChangeMessage {
    message: proto::ChartChange,
    values: Vec<ArrayRef>,
}

impl ChangeMessage {
    /// The change that `builder` builds. It fails with the first error that the setters found, and
    /// for a change that breaks a rule of the format.
    pub fn new(builder: proto::ChartChangeBuilder) -> Result<Self, ChartError> {
        Self::from_fragment(&builder.to_fragment()?)
    }

    /// Decodes a change from the bytes of its fragment. It fails for bytes that are not a fragment
    /// of a `ChartChange` of this format version, and for a change that breaks a rule of the
    /// format, so a change from a page or a network never reaches a reader unchecked.
    pub fn from_bytes(bytes: impl Into<Bytes>) -> Result<Self, ChartError> {
        let fragment = proto::Fragment::decode(bytes.into())
            .map_err(|error| ChartError::invalid("", format!("not a fragment: {error}")))?;
        Self::from_fragment(&fragment)
    }

    fn from_fragment(fragment: &proto::Fragment) -> Result<Self, ChartError> {
        let (message, values) = crate::builder::fragment_parts::<proto::ChartChange>(fragment)?;
        validate::change(&message, &values)?;
        Ok(Self { message, values })
    }

    /// Encodes the change as the bytes of its fragment.
    pub fn to_bytes(&self) -> Result<Bytes, ChartError> {
        let values = match self.values.is_empty() {
            true => None,
            false => Some(proto::Table {
                padding: Bytes::new(),
                ipc: values_to_ipc(&self.values)?,
            }),
        };
        let fragment = proto::Fragment {
            version: FORMAT_VERSION,
            r#type: proto::ChartChange::full_name(),
            message: self.message.encode_to_vec().into(),
            values,
        };
        Ok(fragment.encode_to_vec().into())
    }

    /// The message of the change.
    pub fn message(&self) -> &proto::ChartChange {
        &self.message
    }

    /// The values that the references of the message refer to, each an array of one element.
    pub fn values(&self) -> &[ArrayRef] {
        &self.values
    }

    /// The value that `value` refers to, an array of one element.
    pub fn value(&self, value: &proto::ValueRef) -> &ArrayRef {
        &self.values[value.index as usize]
    }

    /// The elements of the list that `value` refers to, such as a domain or the values of a set.
    pub fn list_elements(&self, value: &proto::ValueRef) -> ArrayRef {
        match self.value(value).data_type() {
            DataType::List(_) => self.value(value).as_list::<i32>().value(0),
            other => panic!("a valid change refers to a list here, not {other}"),
        }
    }
}
