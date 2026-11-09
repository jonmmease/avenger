//! The data that the message builders of [`proto`] collect: the values that their
//! fields take, which become the chart's values, and the order of the nodes that
//! [`Node::partitions`](crate::proto::Node::partitions) builds.

use std::sync::Arc;

use arrow::array::{
    ArrayRef, BooleanArray, Float32Array, Float64Array, Int32Array, Int64Array, RecordBatch,
    StringArray, UInt32Array,
};
use prost::bytes::Bytes;

use crate::{
    codec::list_value,
    proto,
    validate::{is_position, scaled, scales_numbers, scales_strings, takes_scale},
    ChartError, MarkMessage,
};

/// A value of a field that takes a `ValueRef`: an array of one element, such as a scalar or a list.
#[derive(Clone, Debug)]
pub struct Value(Result<ArrayRef, String>);

impl Value {
    /// A scalar value of any Arrow type, such as a date: `array`, which has exactly one element.
    pub fn scalar(array: ArrayRef) -> Self {
        match array.len() {
            1 => Self(Ok(array)),
            len => Self(Err(format!(
                "a scalar value is an array of one element, and this one has {len}"
            ))),
        }
    }

    /// A list value of any Arrow type, such as an explicit domain of dates: one element of a list
    /// whose items are `items`.
    pub fn list(items: ArrayRef) -> Self {
        Self(Ok(list_value(items)))
    }
}

/// The values that a field of a `ValueRef` takes: [`Value`], a scalar of a Rust type, which becomes
/// one element of its own Arrow type without a conversion, or an array, a `Vec`, or a slice of
/// them, which becomes one element of a list.
pub trait IntoValue {
    fn into_value(self) -> Value;
}

impl IntoValue for Value {
    fn into_value(self) -> Value {
        self
    }
}

/// A Rust type whose values make an Arrow array.
pub trait ListItem: Sized {
    fn array(items: Vec<Self>) -> ArrayRef;
}

macro_rules! items {
    ($($rust:ty => $array:ty),* $(,)?) => {$(
        impl ListItem for $rust {
            fn array(items: Vec<Self>) -> ArrayRef {
                Arc::new(<$array>::from(items))
            }
        }

        impl IntoValue for $rust {
            fn into_value(self) -> Value {
                Value(Ok(<$rust as ListItem>::array(vec![self])))
            }
        }
    )*};
}

items!(
    f64 => Float64Array,
    f32 => Float32Array,
    i64 => Int64Array,
    i32 => Int32Array,
    u32 => UInt32Array,
    bool => BooleanArray,
    String => StringArray,
);

impl ListItem for &str {
    fn array(items: Vec<Self>) -> ArrayRef {
        Arc::new(StringArray::from(items))
    }
}

impl IntoValue for &str {
    fn into_value(self) -> Value {
        Value(Ok(<&str as ListItem>::array(vec![self])))
    }
}

impl<T: ListItem> IntoValue for Vec<T> {
    fn into_value(self) -> Value {
        Value::list(T::array(self))
    }
}

impl<T: ListItem, const N: usize> IntoValue for [T; N] {
    fn into_value(self) -> Value {
        Value::list(T::array(self.into()))
    }
}

impl<T: ListItem + Clone> IntoValue for &[T] {
    fn into_value(self) -> Value {
        Value::list(T::array(self.to_vec()))
    }
}

/// The values that the setter of an encoding of a channel takes: an [`EncodingBuilder`], an
/// [`Encoding`], a string, which names a column, or a number or a bool, which is one value.
///
/// A string is a `scaled_field` on a channel that has a scale or follows the scale of its partner,
/// such as `x`, `color`, or `x2`, and a `direct_field` on a channel of direct values, such as
/// `text` or `longitude`, as the `channel_scale` option of the channel's value in `channel.proto`
/// says. A number is a `scaled_value` on a position of a coordinate system with a scale, a data
/// value such as the place of a reference line on `x`, and a `direct_value` on any other channel,
/// which the mark draws as it is, such as a size of `100.0`, as the `position` option says. A bool
/// is a direct value. A direct number on a position is `Encoding::direct_value(..)`, a data value
/// on another channel is `Encoding::scaled_value(..)`, and a string constant, such as a CSS color,
/// is `Encoding::direct_value(..)`.
///
/// [`EncodingBuilder`]: crate::proto::EncodingBuilder
/// [`Encoding`]: crate::proto::Encoding
pub trait IntoEncoding {
    /// The encoding of `channel`.
    fn into_encoding(self, channel: proto::Channel) -> proto::EncodingBuilder;
}

impl IntoEncoding for proto::EncodingBuilder {
    fn into_encoding(self, _channel: proto::Channel) -> proto::EncodingBuilder {
        self
    }
}

impl IntoEncoding for proto::Encoding {
    fn into_encoding(self, _channel: proto::Channel) -> proto::EncodingBuilder {
        self.into()
    }
}

impl IntoEncoding for &str {
    fn into_encoding(self, channel: proto::Channel) -> proto::EncodingBuilder {
        match scales_strings(channel) {
            true => proto::Encoding::scaled_field(self),
            false => proto::Encoding::direct_field(self),
        }
    }
}

impl IntoEncoding for String {
    fn into_encoding(self, channel: proto::Channel) -> proto::EncodingBuilder {
        self.as_str().into_encoding(channel)
    }
}

macro_rules! numbers {
    ($($rust:ty),* $(,)?) => {$(
        impl IntoEncoding for $rust {
            fn into_encoding(self, channel: proto::Channel) -> proto::EncodingBuilder {
                match scales_numbers(channel) {
                    true => proto::Encoding::scaled_value(self),
                    false => proto::Encoding::direct_value(self),
                }
            }
        }
    )*};
}

numbers!(f64, f32, i64, i32, u32);

impl IntoEncoding for bool {
    fn into_encoding(self, _channel: proto::Channel) -> proto::EncodingBuilder {
        proto::Encoding::direct_value(self)
    }
}

/// The order of the nodes of [`Node::partitions`](crate::proto::Node::partitions).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartitionOrder {
    /// The order in which the values first appear in the rows.
    Appearance,
    /// The values from least to greatest, by Arrow's order of their type, with nulls last.
    Ascending,
    /// The values from greatest to least, by Arrow's order of their type, with nulls last.
    Descending,
}

/// The format syntaxes of a chart that a builder embeds, at the path of its node from the builder.
#[derive(Clone, Debug)]
pub(crate) struct Syntaxes {
    pub(crate) path: String,
    pub(crate) number: String,
    pub(crate) datetime: String,
}

/// What a builder collects beside its message: the tables with their kept IPC streams and the
/// values that its references refer to, the selections, the modes, and the format syntaxes of the
/// charts that it embeds, and the errors that its setters found, each at its path from the builder.
#[derive(Clone, Debug, Default)]
pub(crate) struct Parts {
    pub(crate) tables: Vec<(RecordBatch, Option<Bytes>)>,
    pub(crate) values: Vec<ArrayRef>,
    pub(crate) selections: Vec<proto::Selection>,
    pub(crate) modes: Vec<proto::Mode>,
    pub(crate) syntaxes: Vec<Syntaxes>,
    pub(crate) errors: Vec<(String, String)>,
    /// The path of the first table, value, or chart, for the error of a `build` below `Chart`.
    pub(crate) datum: Option<String>,
    /// The nodes that the builder of a plot overlays, which its [`Finish`] merges into the plot.
    pub(crate) overlays: Vec<proto::NodeBuilder>,
}

/// `path` below `prefix`.
pub(crate) fn join(prefix: &str, path: &str) -> String {
    match (prefix.is_empty(), path.is_empty()) {
        (true, _) => path.to_string(),
        (_, true) => prefix.to_string(),
        _ => format!("{prefix}.{path}"),
    }
}

impl Parts {
    /// Adds `value` at the field `path` and returns its reference.
    pub(crate) fn value(&mut self, path: &str, value: impl IntoValue) -> proto::ValueRef {
        let array = match value.into_value().0 {
            Ok(array) => array,
            Err(reason) => {
                self.errors.push((path.to_string(), reason));
                Arc::new(Float64Array::from(vec![0.0]))
            }
        };
        self.datum.get_or_insert_with(|| path.to_string());
        self.values.push(array);
        proto::ValueRef {
            index: (self.values.len() - 1) as u32,
        }
    }

    /// Adds `table` at the field `path` and returns its reference.
    pub(crate) fn table(&mut self, path: &str, table: RecordBatch) -> proto::TableRef {
        self.datum.get_or_insert_with(|| path.to_string());
        self.tables.push((table, None));
        proto::TableRef {
            index: (self.tables.len() - 1) as u32,
        }
    }

    /// Moves the parts of `builder`, at the field `path`, into these parts, and returns its
    /// message, whose references refer to them.
    pub(crate) fn embed<B: Build>(&mut self, path: &str, builder: B) -> B::Message {
        let (message, parts) = builder.into_parts();
        self.adopt(path, message, parts)
    }

    /// Moves `parts`, which `message` refers to, at the field `path`, into these parts, and returns
    /// `message` with its references renumbered.
    pub(crate) fn adopt<M: Renumber>(&mut self, path: &str, mut message: M, parts: Parts) -> M {
        let (values, tables) = (self.values.len() as u32, self.tables.len() as u32);
        message.renumber(values, tables);
        let Parts {
            tables: own_tables,
            values: own_values,
            selections,
            modes,
            syntaxes,
            errors,
            datum,
            overlays,
        } = parts;
        // Only a plot overlays nodes, and its `finish` takes them before it is embedded.
        debug_assert!(
            overlays.is_empty(),
            "the overlays of an embedded plot are merged"
        );
        self.modes.extend(modes);
        self.tables.extend(own_tables);
        self.values.extend(own_values);
        for mut selection in selections {
            selection.renumber(values, 0);
            self.selections.push(selection);
        }
        self.syntaxes
            .extend(syntaxes.into_iter().map(|syntaxes| Syntaxes {
                path: join(path, &syntaxes.path),
                ..syntaxes
            }));
        (self.errors).extend((errors.into_iter()).map(|(at, reason)| (join(path, &at), reason)));
        if self.datum.is_none() {
            self.datum = datum.map(|at| join(path, &at));
        }
        message
    }

    /// The first error that the setters found, at its path below `prefix`.
    pub(crate) fn error(&self, prefix: &str) -> Result<(), ChartError> {
        match self.errors.first() {
            Some((path, reason)) => Err(ChartError::invalid(join(prefix, path), reason.clone())),
            None => Ok(()),
        }
    }

    /// Checks that a builder below `Chart` holds no tables, values, or charts, whose references
    /// mean something only in a chart.
    pub(crate) fn standalone(&self) -> Result<(), ChartError> {
        self.error("")?;
        match &self.datum {
            Some(path) => Err(ChartError::invalid(
                path.clone(),
                "a builder with tables, values, or charts builds only in a chart, which holds its \
                 data",
            )),
            None => Ok(()),
        }
    }
}

/// A builder of a message of the format: the message and what its setters collected beside it,
/// such as the values and the tables that its references refer to.
///
/// Each message `Foo` of [`proto`] has the alias `FooBuilder = Builder<Foo>`, whose page lists the
/// setters of `Foo`, and the constructors on the message, such as `Foo::builder()`. A message
/// converts to its builder as it is, which holds no tables and no values.
#[derive(Clone, Debug, Default)]
pub struct Builder<M> {
    pub(crate) message: M,
    pub(crate) parts: Parts,
}

impl<M> From<M> for Builder<M> {
    /// A builder of `message` as it is, which holds no tables and no values, so its references
    /// refer to none.
    fn from(message: M) -> Self {
        Self {
            message,
            parts: Parts::default(),
        }
    }
}

/// A builder of a message: its message and the parts that its references refer to.
pub(crate) trait Build {
    type Message: Renumber;
    fn into_parts(self) -> (Self::Message, Parts);
}

impl<M: Finish> Build for Builder<M> {
    type Message = M;
    fn into_parts(self) -> (M, Parts) {
        M::finish(self.message, self.parts)
    }
}

/// A message whose references shift by the counts of the values and the tables before its own.
/// build.rs implements it for each message with a builder.
pub(crate) trait Renumber {
    fn renumber(&mut self, values: u32, tables: u32);
}

/// A message that its builder's parts complete when the builder ends. build.rs implements it as
/// the identity for each message with a builder except `Plot`, whose builder merges the plots that
/// it overlays.
pub(crate) trait Finish: Renumber + Sized {
    fn finish(message: Self, parts: Parts) -> (Self, Parts) {
        (message, parts)
    }
}

/// Hides the axis of `scale`, keeping its other fields.
fn hide_axis(scale: &mut proto::Scale) {
    scale.axis.get_or_insert_with(Default::default).show = Some(false);
}

/// Hides the legend of `scale`, keeping its other fields.
fn hide_legend(scale: &mut proto::Scale) {
    scale.legend.get_or_insert_with(Default::default).show = Some(false);
}

impl proto::ScaleBuilder {
    /// Draws no axis for the scale: sets `axis.show` to `false`.
    pub fn no_axis(mut self) -> Self {
        hide_axis(&mut self.message);
        self
    }

    /// Draws no legend for the scale: sets `legend.show` to `false`.
    pub fn no_legend(mut self) -> Self {
        hide_legend(&mut self.message);
        self
    }
}

impl proto::EncodingBuilder {
    /// Draws no axis for the encoding's scale: sets `axis.show` of its scale to `false`, and
    /// gives the encoding a scale when it has none, whose type and domain the reader infers. The
    /// encoding has a scaled value, as every encoding with a scale does.
    pub fn no_axis(mut self) -> Self {
        hide_axis(self.message.scale.get_or_insert_with(Default::default));
        self
    }

    /// Draws no legend for the encoding's scale, as [`no_axis`](Self::no_axis) draws no axis.
    pub fn no_legend(mut self) -> Self {
        hide_legend(self.message.scale.get_or_insert_with(Default::default));
        self
    }
}

/// Hides the axes and the legends of the scales of the encodings of `mark`: the axis of each
/// scaled position and the legend of each other scaled channel with a scale of its own.
fn hide_guides(mark: &mut dyn MarkMessage) {
    for channel in mark.channels() {
        let Some(encoding) = mark.encoding_mut(*channel) else {
            continue;
        };
        if !takes_scale(*channel) || !scaled(encoding) {
            continue;
        }
        let scale = encoding.scale.get_or_insert_with(Default::default);
        match is_position(*channel) {
            true => hide_axis(scale),
            false => hide_legend(scale),
        }
    }
}

macro_rules! no_guides {
    ($($builder:ty),* $(,)?) => {$(
        impl $builder {
            /// Draws no axes and no legends for the scales of the plot's layers so far: hides the
            /// axis of each scaled position and the legend of each other scaled channel. A scale
            /// that the plot shares with another plot draws none there either, since the merged
            /// spec hides them.
            pub fn no_guides(mut self) -> Self {
                for layer in &mut self.message.layers {
                    if let Some(mark) = layer.mark.as_mut() {
                        hide_guides(mark);
                    }
                }
                self
            }
        }
    )*};
}

no_guides!(
    proto::CartesianPlotBuilder,
    proto::PolarPlotBuilder,
    proto::GeoPlotBuilder,
);
