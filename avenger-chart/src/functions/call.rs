//! The calls of mark functions: a [`MarkCall`] takes the channel arguments, the properties, and
//! `coord` of a call, checks them by the rules of its mark, and gives a [`MarkTemplate`], which
//! builds the chart of one layer from a table.

use arrow::array::{ArrayRef, RecordBatch};
use prost::{bytes::Bytes, Message};

use super::{
    generated::{apply_all_encoding, to_cartesian, to_geo, to_polar},
    Arg, ArgError, Kind, Mark, COORD,
};
use crate::{
    builder::{from_fragment, to_fragment, Builder, Parts, Value},
    layer::MarkMessage,
    proto::{self, Encoding, EncodingBuilder, Plot},
    validate::{scales_numbers, scales_strings},
    Chart, ChartError, CoordKind,
};

/// A mark in the `mark` oneof of its layer.
pub(crate) enum LayerMark {
    Cartesian(proto::cartesian_layer::Mark),
    Polar(proto::polar_layer::Mark),
    Geo(proto::geo_layer::Mark),
}

impl LayerMark {
    fn as_mark(&mut self) -> &mut dyn MarkMessage {
        match self {
            Self::Cartesian(mark) => mark,
            Self::Polar(mark) => mark,
            Self::Geo(mark) => mark,
        }
    }
}

/// The function that applies the arguments of the properties of a mark to its message and its
/// parts.
pub(crate) type ApplyProperties =
    fn(&mut LayerMark, &mut Parts, &[Option<Arg>]) -> Result<(), ArgError>;

/// The argument of a channel: a field, which names a column of the layer's table, or a value,
/// an array of one element, which can be a null of a type.
#[derive(Clone, Debug)]
pub enum ChannelArg {
    Field(String),
    Value(ArrayRef),
}

impl super::Channel {
    /// The kind that a bare field of the channel takes: scaled on a channel that scales strings,
    /// such as `color`, and direct on another, such as `text`.
    pub fn field_kind(&self) -> Kind {
        match scales_strings(self.channel) {
            true => Kind::Scaled,
            false => Kind::Direct,
        }
    }

    /// The kind that a bare value of the channel takes: scaled on a channel that scales numbers,
    /// such as `y` for a reference line, and direct on another, such as `fill`.
    pub fn value_kind(&self) -> Kind {
        match scales_numbers(self.channel) {
            true => Kind::Scaled,
            false => Kind::Direct,
        }
    }
}

impl Mark {
    /// A call of the mark function, without arguments.
    pub fn call(&'static self) -> MarkCall {
        MarkCall {
            mark: self,
            layer: (self.mark)(),
            parts: Parts::default(),
            options: None,
            columns: Vec::new(),
        }
    }
}

/// A call of a mark function, which takes its arguments one at a time. Each step fails with the
/// first error of its arguments.
pub struct MarkCall {
    mark: &'static Mark,
    layer: LayerMark,
    parts: Parts,
    options: Option<proto::Fragment>,
    columns: Vec<String>,
}

impl MarkCall {
    /// Sets the channel `name` from `arg`, with the kind `kind`, or the default kind of the
    /// channel for `None`, and for the scaled kind the options of an encoding `options`, in the
    /// order of [`encoding_options`](super::encoding_options).
    pub fn channel(
        &mut self,
        name: &str,
        arg: ChannelArg,
        kind: Option<Kind>,
        options: &[Option<Arg>],
    ) -> Result<(), ArgError> {
        let mark = self.mark.name;
        let declared = (self.mark.channels.iter())
            .find(|channel| channel.name == name)
            .ok_or_else(|| ArgError::mark(format!("{mark}() has no channel `{name}`")))?;
        let channel = declared.channel;
        if kind == Some(Kind::Scaled) && !scales_strings(channel) && !scales_numbers(channel) {
            return Err(ArgError::mark(format!(
                "scaled() needs a channel with a scale, and `{name}` of {mark}() has none"
            )));
        }
        let mut encoding = match arg {
            ChannelArg::Field(column) => {
                if !self.columns.contains(&column) {
                    self.columns.push(column.clone());
                }
                match kind.unwrap_or(declared.field_kind()) {
                    Kind::Scaled => Encoding::scaled_field(column),
                    Kind::Direct => Encoding::direct_field(column),
                }
            }
            ChannelArg::Value(array) => {
                let value = Value::scalar(array);
                match kind.unwrap_or(declared.value_kind()) {
                    Kind::Scaled => Encoding::scaled_value(value),
                    Kind::Direct => Encoding::direct_value(value),
                }
            }
        };
        if !options.is_empty() {
            encoding =
                apply_all_encoding(encoding, options).map_err(|e| e.within(declared.name))?;
        }
        set_encoding(
            &mut self.layer,
            &mut self.parts,
            declared.name,
            channel,
            encoding,
        );
        Ok(())
    }

    /// Sets the properties of the mark from `args`, in the order of the mark's properties.
    pub fn properties(&mut self, args: &[Option<Arg>]) -> Result<(), ArgError> {
        (self.mark.apply_properties)(&mut self.layer, &mut self.parts, args)
    }

    /// Sets the options of the mark's plot from `arg`, a fragment of the options of its
    /// coordinate system or an argument that converts to one, such as
    /// `spherical(projection => 'mercator')` for a geo plot.
    pub fn coord(&mut self, arg: &Arg) -> Result<(), ArgError> {
        self.set_coord(arg).map_err(|error| error.within(COORD))
    }

    fn set_coord(&mut self, arg: &Arg) -> Result<(), ArgError> {
        let fragment = match self.mark.coord {
            CoordKind::Cartesian => to_cartesian(arg)?.to_fragment(),
            CoordKind::Polar => to_polar(arg)?.to_fragment(),
            CoordKind::Geo => to_geo(arg)?.to_fragment(),
        };
        self.options = Some(fragment.map_err(ArgError::invalid)?);
        Ok(())
    }

    /// The template of the call, after the checks of the rules of its mark: a geo mark has
    /// `coord`, each required channel has an argument, and a channel reads a column.
    pub fn bind(mut self) -> Result<MarkTemplate, ArgError> {
        let mark = self.mark;
        let name = mark.name;
        if mark.coord == CoordKind::Geo && self.options.is_none() {
            return Err(ArgError::mark(format!(
                "{name}() needs `coord`, the options of its geo plot, such as `coord => geo(surface => spherical(projection => 'equal_earth'))`"
            )));
        }
        if let Some(declared) = (mark.channels.iter()).find(|declared| {
            declared.required && self.layer.as_mark().encoding(declared.channel).is_none()
        }) {
            return Err(ArgError::mark(format!(
                "{name}() needs a value for `{}`",
                declared.name
            )));
        }
        if self.columns.is_empty() {
            return Err(ArgError::mark(format!(
                "{name}() needs a channel that reads a column"
            )));
        }
        let parts = self.parts;
        let layer = match self.layer {
            LayerMark::Cartesian(mark) => to_fragment(builder(
                proto::CartesianLayer {
                    mark: Some(mark),
                    ..Default::default()
                },
                parts,
            )),
            LayerMark::Polar(mark) => to_fragment(builder(
                proto::PolarLayer {
                    mark: Some(mark),
                    ..Default::default()
                },
                parts,
            )),
            LayerMark::Geo(mark) => to_fragment(builder(
                proto::GeoLayer {
                    mark: Some(mark),
                    ..Default::default()
                },
                parts,
            )),
        }
        .map_err(ArgError::invalid)?;
        Ok(MarkTemplate {
            name,
            coord: mark.coord,
            layer: layer.encode_to_vec().into(),
            options: self.options.map(|options| options.encode_to_vec().into()),
            columns: self.columns,
        })
    }
}

/// A checked call of a mark function: the encoded fragment of its layer, which holds its
/// encodings and their values and no table, the encoded fragment of the options of its plot, and
/// the names of the columns that its table holds, in the order of their first channels.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MarkTemplate {
    name: &'static str,
    coord: CoordKind,
    layer: Bytes,
    options: Option<Bytes>,
    columns: Vec<String>,
}

impl MarkTemplate {
    /// The name of the mark function.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The names of the columns that the table of the layer holds.
    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    /// The chart of one layer whose table is `table`, in a plot with the options of the call.
    pub fn chart(&self, table: RecordBatch) -> Result<Chart, ChartError> {
        let decode = |bytes: &Bytes| {
            proto::Fragment::decode(bytes.clone())
                .map_err(|error| ChartError::invalid("", error.to_string()))
        };
        let fragment = decode(&self.layer)?;
        let options = self.options.as_ref().map(decode).transpose()?;
        let plot = match self.coord {
            CoordKind::Cartesian => {
                let mut layer = from_fragment::<proto::CartesianLayer>(&fragment)?;
                layer.message.table = Some(layer.parts.table("table", table));
                let mut plot = proto::CartesianPlot::new([layer]);
                if let Some(options) = &options {
                    plot = plot.options(from_fragment::<proto::Cartesian>(options)?);
                }
                Plot::cartesian(plot)
            }
            CoordKind::Polar => {
                let mut layer = from_fragment::<proto::PolarLayer>(&fragment)?;
                layer.message.table = Some(layer.parts.table("table", table));
                let mut plot = proto::PolarPlot::new([layer]);
                if let Some(options) = &options {
                    plot = plot.options(from_fragment::<proto::Polar>(options)?);
                }
                Plot::polar(plot)
            }
            CoordKind::Geo => {
                let mut layer = from_fragment::<proto::GeoLayer>(&fragment)?;
                layer.message.table = Some(layer.parts.table("table", table));
                let options = options.expect("a geo mark has the options of its plot");
                let options = from_fragment::<proto::Geo>(&options)?;
                Plot::geo(proto::GeoPlot::new(options, [layer]))
            }
        };
        Chart::builder(plot).build()
    }
}

/// Sets the encoding of `channel` of `mark`, with its values moved into `parts`.
fn set_encoding(
    mark: &mut LayerMark,
    parts: &mut Parts,
    field: &str,
    channel: proto::Channel,
    encoding: EncodingBuilder,
) {
    let encoding = parts.embed(field, encoding);
    let slot = (mark.as_mark().encoding_slot(channel))
        .expect("a declared channel is a channel of its mark");
    *slot = Some(encoding);
}

/// A builder of `message` with `parts`.
fn builder<M>(message: M, parts: Parts) -> Builder<M> {
    Builder { message, parts }
}
