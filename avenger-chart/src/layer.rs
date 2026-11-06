//! The plots and layers of a chart: the coordinate system of a plot, a view of a layer of any
//! coordinate system, and the channels and encodings of the mark of a layer.
//!
//! build.rs generates the channels and the encodings of each mark message, and an impl of
//! [`MarkMessage`] for the `mark` oneof of each kind of layer, from the descriptors of the format.

use crate::proto::{self, plot::Coord, Channel, Encoding};

include!(concat!(env!("OUT_DIR"), "/marks.rs"));

/// The kind of a plot's coordinate system, with the facts of the kind that do not depend on a
/// chart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CoordKind {
    Cartesian,
    Polar,
    Geo,
}

impl CoordKind {
    /// The kind of the coordinate system of `plot`, or `None` when the plot does not set one.
    pub fn of(plot: &proto::Plot) -> Option<Self> {
        match plot.coord.as_ref()? {
            Coord::Cartesian(_) => Some(Self::Cartesian),
            Coord::Polar(_) => Some(Self::Polar),
            Coord::Geo(_) => Some(Self::Geo),
        }
    }

    /// The name of the kind's member of `Plot.coord`, such as `cartesian`.
    pub fn field(self) -> &'static str {
        match self {
            Self::Cartesian => "cartesian",
            Self::Polar => "polar",
            Self::Geo => "geo",
        }
    }

    /// The name of the kind in errors, such as `polar`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Cartesian => "Cartesian",
            Self::Polar => "polar",
            Self::Geo => "map",
        }
    }

    /// The position channels: `x` and `y` of a Cartesian plot, `theta` and `radius` of a polar
    /// plot, and `longitude` and `latitude` of a map plot.
    pub fn positions(self) -> &'static [Channel] {
        match self {
            Self::Cartesian => &[Channel::X, Channel::Y],
            Self::Polar => &[Channel::Theta, Channel::Radius],
            Self::Geo => &[Channel::Longitude, Channel::Latitude],
        }
    }

    /// Each position channel with its second channel, such as `x` with `x2`, from which a mark
    /// with a range spans to its second channel.
    pub fn pairs(self) -> &'static [[Channel; 2]] {
        match self {
            Self::Cartesian => &[[Channel::X, Channel::X2], [Channel::Y, Channel::Y2]],
            Self::Polar => &[
                [Channel::Theta, Channel::Theta2],
                [Channel::Radius, Channel::Radius2],
            ],
            Self::Geo => &[
                [Channel::Longitude, Channel::Longitude2],
                [Channel::Latitude, Channel::Latitude2],
            ],
        }
    }

    /// The position channels whose scales run along a side of the plot: `x` and `y` of a
    /// Cartesian plot. The axis of such a scale stands on a side of the plot, and its range is the
    /// plot's length along the side.
    pub fn edges(self) -> &'static [Channel] {
        match self {
            Self::Cartesian => &[Channel::X, Channel::Y],
            Self::Polar | Self::Geo => &[],
        }
    }

    /// The position channels whose scales a `Navigate` moves: `x` and `y` of a Cartesian plot. A
    /// map plot navigates its projection, and a polar plot takes no navigation.
    pub fn navigated(self) -> &'static [Channel] {
        match self {
            Self::Cartesian => &[Channel::X, Channel::Y],
            Self::Polar | Self::Geo => &[],
        }
    }
}

/// The type of a mark, which its messages in each coordinate system share.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MarkType {
    Symbol,
    Line,
    Trail,
    Area,
    Bar,
    Rect,
    Rule,
    Text,
    Image,
    Path,
    Shape,
}

/// The mark of a layer: the `mark` oneof of a layer of each coordinate system. build.rs
/// implements it for each.
pub trait MarkMessage {
    /// The kind of the coordinate system that the mark draws in.
    fn kind(&self) -> CoordKind;
    /// The type of the mark, which the marks of each kind share.
    fn mark_type(&self) -> MarkType;
    /// The name of the mark's member of the `mark` oneof, such as `symbol`.
    fn name(&self) -> &'static str;
    /// The channels that the mark takes, in the order of `Channel`.
    fn channels(&self) -> &'static [Channel];
    /// The encoding of `channel`, or `None` when the mark does not encode it.
    fn encoding(&self, channel: Channel) -> Option<&Encoding>;
    /// The encoding of `channel`, to change, or `None` when the mark does not encode it.
    fn encoding_mut(&mut self, channel: Channel) -> Option<&mut Encoding>;
}

impl dyn MarkMessage + '_ {
    /// The encodings of the mark with their channels, in the order of `Channel`.
    pub fn encodings(&self) -> impl Iterator<Item = (Channel, &Encoding)> {
        (self.channels().iter()).filter_map(|channel| Some((*channel, self.encoding(*channel)?)))
    }
}

/// A layer of any coordinate system.
#[derive(Clone, Copy, Debug)]
pub enum LayerRef<'a> {
    Cartesian(&'a proto::CartesianLayer),
    Polar(&'a proto::PolarLayer),
    Geo(&'a proto::GeoLayer),
}

impl<'a> LayerRef<'a> {
    /// The layers of `plot`, in the order that they draw.
    pub fn of(plot: &'a proto::Plot) -> Vec<Self> {
        match &plot.coord {
            Some(Coord::Cartesian(plot)) => plot.layers.iter().map(Self::Cartesian).collect(),
            Some(Coord::Polar(plot)) => plot.layers.iter().map(Self::Polar).collect(),
            Some(Coord::Geo(plot)) => plot.layers.iter().map(Self::Geo).collect(),
            None => Vec::new(),
        }
    }

    /// The kind of the layer's coordinate system.
    pub fn kind(self) -> CoordKind {
        match self {
            Self::Cartesian(_) => CoordKind::Cartesian,
            Self::Polar(_) => CoordKind::Polar,
            Self::Geo(_) => CoordKind::Geo,
        }
    }

    pub fn id(self) -> &'a str {
        match self {
            Self::Cartesian(layer) => &layer.id,
            Self::Polar(layer) => &layer.id,
            Self::Geo(layer) => &layer.id,
        }
    }

    /// The class names of the layer, which stylesheets select.
    pub fn classes(self) -> &'a [String] {
        match self {
            Self::Cartesian(layer) => &layer.classes,
            Self::Polar(layer) => &layer.classes,
            Self::Geo(layer) => &layer.classes,
        }
    }

    pub fn table(self) -> Option<&'a proto::TableRef> {
        match self {
            Self::Cartesian(layer) => layer.table.as_ref(),
            Self::Polar(layer) => layer.table.as_ref(),
            Self::Geo(layer) => layer.table.as_ref(),
        }
    }

    pub fn tools(self) -> &'a [proto::LayerTool] {
        match self {
            Self::Cartesian(layer) => &layer.tools,
            Self::Polar(layer) => &layer.tools,
            Self::Geo(layer) => &layer.tools,
        }
    }

    pub fn partition(self) -> &'a [proto::PartitionValue] {
        match self {
            Self::Cartesian(layer) => &layer.partition,
            Self::Polar(layer) => &layer.partition,
            Self::Geo(layer) => &layer.partition,
        }
    }

    /// The layer's mark, or `None` when the layer does not set one.
    pub fn mark(self) -> Option<&'a dyn MarkMessage> {
        match self {
            Self::Cartesian(layer) => layer.mark.as_ref().map(|mark| mark as &dyn MarkMessage),
            Self::Polar(layer) => layer.mark.as_ref().map(|mark| mark as &dyn MarkMessage),
            Self::Geo(layer) => layer.mark.as_ref().map(|mark| mark as &dyn MarkMessage),
        }
    }
}
